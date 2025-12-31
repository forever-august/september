//! Message types for the NNTP worker pool
//!
//! These messages are sent from the NntpService to worker tasks via async_channel,
//! with responses sent back via oneshot channels. Requests are prioritized to ensure
//! user-facing operations (like fetching an article) are processed before background
//! tasks (like refreshing group statistics).

use std::fmt;

use tokio::sync::oneshot;

use nntp_rs::OverviewEntry;

use super::{ArticleView, GroupView, ThreadView};

/// Priority levels for NNTP operations.
///
/// Higher priority requests are processed before lower priority ones to ensure
/// responsive user experience. Aging prevents starvation of low-priority requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// User-facing requests that block page rendering (GetArticle, PostArticle)
    High,
    /// Page load requests, slightly less latency-sensitive (GetThreads, GetGroups)
    Normal,
    /// Background operations that can wait (GetGroupStats, GetNewArticles)
    Low,
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Priority::High => write!(f, "high"),
            Priority::Normal => write!(f, "normal"),
            Priority::Low => write!(f, "low"),
        }
    }
}

/// Error type for NNTP operations that can be sent across channels
#[derive(Debug, Clone)]
pub struct NntpError(pub String);

impl std::fmt::Display for NntpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for NntpError {}

/// Group statistics including last article date
#[derive(Debug, Clone)]
pub struct GroupStatsView {
    /// Date of the last article (RFC 2822 format)
    pub last_article_date: Option<String>,
    /// Last article number (high water mark for incremental updates)
    pub last_article_number: u64,
}

/// Request messages sent to NNTP workers
#[allow(clippy::enum_variant_names)] // "Get" prefix is intentional for request/response pattern
pub enum NntpRequest {
    /// Fetch the list of available newsgroups
    GetGroups {
        response: oneshot::Sender<Result<Vec<GroupView>, NntpError>>,
    },
    /// Fetch recent threads from a newsgroup
    GetThreads {
        group: String,
        count: u64,
        response: oneshot::Sender<Result<Vec<ThreadView>, NntpError>>,
    },
    /// Fetch a single article by message ID
    GetArticle {
        message_id: String,
        response: oneshot::Sender<Result<ArticleView, NntpError>>,
    },
    /// Fetch group statistics including last article date
    GetGroupStats {
        group: String,
        response: oneshot::Sender<Result<GroupStatsView, NntpError>>,
    },
    /// Fetch new articles since a given article number (for incremental updates)
    GetNewArticles {
        group: String,
        since_article_number: u64,
        response: oneshot::Sender<Result<Vec<OverviewEntry>, NntpError>>,
    },
    /// Post a new article or reply
    PostArticle {
        /// Headers as name/value pairs (From, Subject, Newsgroups, References, Date, Message-ID, etc.)
        headers: Vec<(String, String)>,
        /// Article body text (plain text)
        body: String,
        response: oneshot::Sender<Result<(), NntpError>>,
    },
    /// Check if an article exists using STAT command (fast existence check)
    CheckArticleExists {
        message_id: String,
        response: oneshot::Sender<Result<bool, NntpError>>,
    },
}

impl NntpRequest {
    /// Get the priority level for this request type.
    ///
    /// Priority is determined by how latency-sensitive the operation is:
    /// - High: User clicked something and is waiting (GetArticle, PostArticle)
    /// - Normal: Page load operations (GetThreads, GetGroups)
    /// - Low: Background refresh operations (GetGroupStats, GetNewArticles)
    pub fn priority(&self) -> Priority {
        match self {
            NntpRequest::GetArticle { .. }
            | NntpRequest::PostArticle { .. }
            | NntpRequest::CheckArticleExists { .. } => Priority::High,
            NntpRequest::GetThreads { .. } | NntpRequest::GetGroups { .. } => Priority::Normal,
            NntpRequest::GetGroupStats { .. } | NntpRequest::GetNewArticles { .. } => Priority::Low,
        }
    }

    /// Send the response for this request
    pub fn respond(self, result: Result<NntpResponse, NntpError>) {
        match self {
            NntpRequest::GetGroups { response } => {
                if let Ok(NntpResponse::Groups(groups)) = result {
                    let _ = response.send(Ok(groups));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::GetThreads { response, .. } => {
                if let Ok(NntpResponse::Threads(threads)) = result {
                    let _ = response.send(Ok(threads));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::GetArticle { response, .. } => {
                if let Ok(NntpResponse::Article(article)) = result {
                    let _ = response.send(Ok(article));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::GetGroupStats { response, .. } => {
                if let Ok(NntpResponse::GroupStats(stats)) = result {
                    let _ = response.send(Ok(stats));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::GetNewArticles { response, .. } => {
                if let Ok(NntpResponse::NewArticles(entries)) = result {
                    let _ = response.send(Ok(entries));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::PostArticle { response, .. } => {
                if let Ok(NntpResponse::PostResult) = result {
                    let _ = response.send(Ok(()));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
            NntpRequest::CheckArticleExists { response, .. } => {
                if let Ok(NntpResponse::ArticleExists(exists)) = result {
                    let _ = response.send(Ok(exists));
                } else if let Err(e) = result {
                    let _ = response.send(Err(e));
                }
            }
        }
    }
}

/// Response types from NNTP operations
pub enum NntpResponse {
    Groups(Vec<GroupView>),
    Threads(Vec<ThreadView>),
    Article(ArticleView),
    GroupStats(GroupStatsView),
    NewArticles(Vec<OverviewEntry>),
    PostResult,
    ArticleExists(bool),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[test]
    fn test_priority_get_article_is_high() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::GetArticle {
            message_id: "test@example.com".to_string(),
            response: tx,
        };
        assert_eq!(req.priority(), Priority::High);
    }

    #[test]
    fn test_priority_post_article_is_high() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::PostArticle {
            headers: vec![],
            body: "test".to_string(),
            response: tx,
        };
        assert_eq!(req.priority(), Priority::High);
    }

    #[test]
    fn test_priority_check_article_exists_is_high() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::CheckArticleExists {
            message_id: "test@example.com".to_string(),
            response: tx,
        };
        assert_eq!(req.priority(), Priority::High);
    }

    #[test]
    fn test_priority_get_threads_is_normal() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::GetThreads {
            group: "test.group".to_string(),
            count: 25,
            response: tx,
        };
        assert_eq!(req.priority(), Priority::Normal);
    }

    #[test]
    fn test_priority_get_groups_is_normal() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::GetGroups { response: tx };
        assert_eq!(req.priority(), Priority::Normal);
    }

    #[test]
    fn test_priority_get_group_stats_is_low() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::GetGroupStats {
            group: "test.group".to_string(),
            response: tx,
        };
        assert_eq!(req.priority(), Priority::Low);
    }

    #[test]
    fn test_priority_get_new_articles_is_low() {
        let (tx, _rx) = oneshot::channel();
        let req = NntpRequest::GetNewArticles {
            group: "test.group".to_string(),
            since_article_number: 100,
            response: tx,
        };
        assert_eq!(req.priority(), Priority::Low);
    }

    #[test]
    fn test_priority_display() {
        assert_eq!(format!("{}", Priority::High), "high");
        assert_eq!(format!("{}", Priority::Normal), "normal");
        assert_eq!(format!("{}", Priority::Low), "low");
    }

    #[test]
    fn test_priority_ordering() {
        // High < Normal < Low in the Ord implementation (smaller = higher priority)
        assert!(Priority::High < Priority::Normal);
        assert!(Priority::Normal < Priority::Low);
        assert!(Priority::High < Priority::Low);
    }

    #[test]
    fn test_nntp_error_display() {
        let err = NntpError("connection failed".to_string());
        assert_eq!(format!("{}", err), "connection failed");
    }
}
