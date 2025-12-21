//! Message types for the NNTP worker pool
//!
//! These messages are sent from the NntpService to worker tasks via async_channel,
//! with responses sent back via oneshot channels.

use tokio::sync::oneshot;

use super::{ArticleView, GroupView, ThreadView};

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
    /// Number of articles in the group
    pub article_count: u64,
    /// Date of the last article (RFC 2822 format)
    pub last_article_date: Option<String>,
}

/// Request messages sent to NNTP workers
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
    /// Fetch a single thread by root message ID
    GetThread {
        group: String,
        message_id: String,
        response: oneshot::Sender<Result<ThreadView, NntpError>>,
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
}

impl NntpRequest {
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
            NntpRequest::GetThread { response, .. } => {
                if let Ok(NntpResponse::Thread(thread)) = result {
                    let _ = response.send(Ok(thread));
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
        }
    }
}

/// Response types from NNTP operations
pub enum NntpResponse {
    Groups(Vec<GroupView>),
    Threads(Vec<ThreadView>),
    Thread(ThreadView),
    Article(ArticleView),
    GroupStats(GroupStatsView),
}
