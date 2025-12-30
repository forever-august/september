//! NNTP Service for a single server
//!
//! Provides communication with a single NNTP server through a worker pool.
//! Request coalescing prevents duplicate requests for the same resource.
//! Requests are prioritized to ensure user-facing operations are processed
//! before background tasks. Caching is handled at the federated service level.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::instrument;

use nntp_rs::OverviewEntry;

use crate::config::{
    NntpServerConfig, NntpSettings, BROADCAST_CHANNEL_CAPACITY, NNTP_HIGH_PRIORITY_QUEUE_CAPACITY,
    NNTP_LOW_PRIORITY_QUEUE_CAPACITY, NNTP_NORMAL_PRIORITY_QUEUE_CAPACITY,
};

use super::messages::{GroupStatsView, NntpError, NntpRequest, Priority};
use super::worker::{NntpWorker, WorkerCounters, WorkerQueues};
use super::{ArticleView, GroupView, ThreadView};

/// Pending request with timestamp for timeout checking
type PendingEntry<T> = (broadcast::Sender<Result<T, NntpError>>, Instant);

/// Arc-wrapped pending entry for large types to avoid cloning on broadcast
type ArcPendingEntry<T> = (broadcast::Sender<Result<Arc<T>, NntpError>>, Instant);

/// Unwrap Arc, returning owned value if unique or cloning if shared
fn unwrap_arc<T: Clone>(arc: Arc<T>) -> T {
    Arc::try_unwrap(arc).unwrap_or_else(|arc| (*arc).clone())
}

/// Pending request tracking for coalescing
struct PendingRequests {
    articles: Mutex<HashMap<String, PendingEntry<ArticleView>>>,
    /// Arc-wrapped to avoid cloning Vec<ThreadView> on broadcast
    threads: Mutex<HashMap<String, ArcPendingEntry<Vec<ThreadView>>>>,
    /// Arc-wrapped to avoid cloning Vec<GroupView> on broadcast
    groups: Mutex<Option<ArcPendingEntry<Vec<GroupView>>>>,
    group_stats: Mutex<HashMap<String, PendingEntry<GroupStatsView>>>,
}

/// NNTP Service for a single server with request coalescing and priority queues
#[derive(Clone)]
pub struct NntpService {
    /// Server name for logging
    name: String,
    /// High-priority request queue (user-facing: GetArticle, PostArticle)
    high_tx: Sender<NntpRequest>,
    high_rx: Receiver<NntpRequest>,
    /// Normal-priority request queue (page load: GetThreads, GetGroups)
    normal_tx: Sender<NntpRequest>,
    normal_rx: Receiver<NntpRequest>,
    /// Low-priority request queue (background: GetGroupStats, GetNewArticles)
    low_tx: Sender<NntpRequest>,
    low_rx: Receiver<NntpRequest>,
    /// Server configuration
    server_config: Arc<NntpServerConfig>,
    /// Global NNTP settings
    global_settings: Arc<NntpSettings>,
    /// Request timeout duration
    request_timeout: Duration,
    /// Pending requests for coalescing
    pending: Arc<PendingRequests>,
    /// Count of workers with active connections
    connected_workers: Arc<AtomicUsize>,
    /// Count of workers whose connections allow posting
    posting_workers: Arc<AtomicUsize>,
}

impl NntpService {
    /// Create a new NNTP service for a single server
    pub fn new(server_config: NntpServerConfig, global_settings: NntpSettings) -> Self {
        // Create priority request channels with backpressure
        let (high_tx, high_rx) = async_channel::bounded(NNTP_HIGH_PRIORITY_QUEUE_CAPACITY);
        let (normal_tx, normal_rx) = async_channel::bounded(NNTP_NORMAL_PRIORITY_QUEUE_CAPACITY);
        let (low_tx, low_rx) = async_channel::bounded(NNTP_LOW_PRIORITY_QUEUE_CAPACITY);

        let request_timeout =
            Duration::from_secs(server_config.request_timeout_seconds(&global_settings));

        Self {
            name: server_config.name.clone(),
            high_tx,
            high_rx,
            normal_tx,
            normal_rx,
            low_tx,
            low_rx,
            server_config: Arc::new(server_config),
            global_settings: Arc::new(global_settings),
            request_timeout,
            pending: Arc::new(PendingRequests {
                articles: Mutex::new(HashMap::new()),
                threads: Mutex::new(HashMap::new()),
                groups: Mutex::new(None),
                group_stats: Mutex::new(HashMap::new()),
            }),
            connected_workers: Arc::new(AtomicUsize::new(0)),
            posting_workers: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get the server name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if posting is allowed (at least one worker has a posting-capable connection)
    pub fn is_posting_allowed(&self) -> bool {
        self.posting_workers.load(Ordering::Relaxed) > 0
    }

    /// Send a request to the appropriate priority queue
    async fn send_request(&self, request: NntpRequest) -> Result<(), NntpError> {
        let priority = request.priority();
        let result = match priority {
            Priority::High => self.high_tx.send(request).await,
            Priority::Normal => self.normal_tx.send(request).await,
            Priority::Low => self.low_tx.send(request).await,
        };
        result.map_err(|_| NntpError("Worker pool closed".into()))
    }

    /// Spawn worker tasks for this server
    pub fn spawn_workers(&self) {
        let count = self.server_config.worker_count();
        for id in 0..count {
            let worker = NntpWorker::new(
                id,
                (*self.server_config).clone(),
                (*self.global_settings).clone(),
                WorkerQueues {
                    high: self.high_rx.clone(),
                    normal: self.normal_rx.clone(),
                    low: self.low_rx.clone(),
                },
                WorkerCounters {
                    connected: self.connected_workers.clone(),
                    posting: self.posting_workers.clone(),
                },
            );
            tokio::spawn(worker.run());
        }
        tracing::info!(server = %self.name, count, "Spawned NNTP workers");
    }

    /// Fetch an article by message ID
    #[instrument(
        name = "nntp.service.get_article",
        skip(self),
        fields(server = %self.name, coalesced = false, duration_ms)
    )]
    pub async fn get_article(&self, message_id: &str) -> Result<ArticleView, NntpError> {
        let start = Instant::now();
        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.articles.lock().await;
        if let Some((tx, started_at)) = pending.get(message_id) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting
                tracing::Span::current().record("coalesced", true);

                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                // Pending request timed out, remove it and start fresh
                tracing::debug!(server = %self.name, %message_id, "Pending request timed out, starting new request");
                pending.remove(message_id);
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        pending.insert(message_id.to_string(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::GetArticle {
            message_id: message_id.to_string(),
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        // Broadcast to waiters and cleanup pending in one lock acquisition
        // Remove first to minimize time holding lock, then broadcast
        self.pending.articles.lock().await.remove(message_id);
        let _ = tx.send(result.clone());

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Fetch recent threads from a newsgroup
    #[instrument(
        name = "nntp.service.get_threads",
        skip(self),
        fields(server = %self.name, coalesced = false, duration_ms)
    )]
    pub async fn get_threads(&self, group: &str, count: u64) -> Result<Vec<ThreadView>, NntpError> {
        let start = Instant::now();
        let cache_key = format!("{}:{}", group, count);

        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.threads.lock().await;
        if let Some((tx, started_at)) = pending.get(&cache_key) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);
                tracing::Span::current().record("coalesced", true);

                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result.map(unwrap_arc),
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, %group, %count, "Pending request timed out, starting new request");
                pending.remove(&cache_key);
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        pending.insert(cache_key.clone(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::GetThreads {
            group: group.to_string(),
            count,
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        // Broadcast Arc-wrapped result to waiters, then cleanup pending
        self.pending.threads.lock().await.remove(&cache_key);
        let _ = tx.send(
            result
                .as_ref()
                .map(|v| Arc::new(v.clone()))
                .map_err(|e| e.clone()),
        );

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Fetch the list of available newsgroups
    #[instrument(
        name = "nntp.service.get_groups",
        skip(self),
        fields(server = %self.name, coalesced = false, duration_ms)
    )]
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, NntpError> {
        let start = Instant::now();
        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.groups.lock().await;
        if let Some((tx, started_at)) = pending.as_ref() {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);
                tracing::Span::current().record("coalesced", true);

                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result.map(unwrap_arc),
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, "Pending groups request timed out, starting new request");
                *pending = None;
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        *pending = Some((tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::GetGroups { response: resp_tx })
            .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        // Broadcast Arc-wrapped result to waiters, then cleanup pending
        *self.pending.groups.lock().await = None;
        let _ = tx.send(
            result
                .as_ref()
                .map(|v| Arc::new(v.clone()))
                .map_err(|e| e.clone()),
        );

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Fetch group statistics (article count and last article date)
    #[instrument(
        name = "nntp.service.get_group_stats",
        skip(self),
        fields(server = %self.name, coalesced = false, duration_ms)
    )]
    pub async fn get_group_stats(&self, group: &str) -> Result<GroupStatsView, NntpError> {
        let start = Instant::now();
        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.group_stats.lock().await;
        if let Some((tx, started_at)) = pending.get(group) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);
                tracing::Span::current().record("coalesced", true);

                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, %group, "Pending group stats request timed out, starting new request");
                pending.remove(group);
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        pending.insert(group.to_string(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::GetGroupStats {
            group: group.to_string(),
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        // Broadcast to waiters and cleanup pending in one lock acquisition
        // Remove first to minimize time holding lock, then broadcast
        self.pending.group_stats.lock().await.remove(group);
        let _ = tx.send(result.clone());

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Fetch new articles since a given article number (for incremental updates)
    /// Note: No coalescing for this request as it's parameterized by article number
    #[instrument(
        name = "nntp.service.get_new_articles",
        skip(self),
        fields(server = %self.name, duration_ms)
    )]
    pub async fn get_new_articles(
        &self,
        group: &str,
        since_article_number: u64,
    ) -> Result<Vec<OverviewEntry>, NntpError> {
        let start = Instant::now();
        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::GetNewArticles {
            group: group.to_string(),
            since_article_number,
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => {
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                result
            }
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        }
    }

    /// Post an article to the server
    #[instrument(
        name = "nntp.service.post_article",
        skip(self, headers, body),
        fields(server = %self.name, duration_ms)
    )]
    pub async fn post_article(
        &self,
        headers: Vec<(String, String)>,
        body: String,
    ) -> Result<(), NntpError> {
        let start = Instant::now();

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::PostArticle {
            headers,
            body,
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Check if an article exists on this server using the STAT command.
    ///
    /// Returns Ok(true) if the article exists, Ok(false) if not found,
    /// or Err for connection/other errors. This is faster than get_article
    /// as it doesn't transfer the article content.
    #[instrument(
        name = "nntp.service.check_article_exists",
        skip(self),
        fields(server = %self.name, message_id = %message_id, duration_ms)
    )]
    pub async fn check_article_exists(&self, message_id: &str) -> Result<bool, NntpError> {
        let start = Instant::now();

        let (resp_tx, resp_rx) = oneshot::channel();
        self.send_request(NntpRequest::CheckArticleExists {
            message_id: message_id.to_string(),
            response: resp_tx,
        })
        .await?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(NntpError("Worker dropped request".into())),
            Err(_) => Err(NntpError("Request timeout".into())),
        };

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }
}
