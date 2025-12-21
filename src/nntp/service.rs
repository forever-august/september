//! NNTP Service for a single server
//!
//! Provides communication with a single NNTP server through a worker pool.
//! Request coalescing prevents duplicate requests for the same resource.
//! Caching is handled at the federated service level.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::config::{NntpServerConfig, NntpSettings};

use super::messages::{NntpError, NntpRequest};
use super::worker::NntpWorker;
use super::{ArticleView, GroupView, ThreadView};

/// Pending request with timestamp for timeout checking
type PendingEntry<T> = (broadcast::Sender<Result<T, NntpError>>, Instant);

/// Pending request tracking for coalescing
struct PendingRequests {
    articles: Mutex<HashMap<String, PendingEntry<ArticleView>>>,
    threads: Mutex<HashMap<String, PendingEntry<Vec<ThreadView>>>>,
    thread: Mutex<HashMap<String, PendingEntry<ThreadView>>>,
    groups: Mutex<Option<PendingEntry<Vec<GroupView>>>>,
}

/// NNTP Service for a single server with request coalescing
#[derive(Clone)]
pub struct NntpService {
    /// Server name for logging
    name: String,
    /// Request queue sender - workers pull from the receiver
    request_tx: Sender<NntpRequest>,
    /// Request queue receiver - cloned for each worker
    request_rx: Receiver<NntpRequest>,
    /// Server configuration
    server_config: Arc<NntpServerConfig>,
    /// Global NNTP settings
    global_settings: Arc<NntpSettings>,
    /// Request timeout duration
    request_timeout: Duration,
    /// Pending requests for coalescing
    pending: Arc<PendingRequests>,
}

impl NntpService {
    /// Create a new NNTP service for a single server
    pub fn new(server_config: NntpServerConfig, global_settings: NntpSettings) -> Self {
        // Create the request channel with backpressure
        let (tx, rx) = async_channel::bounded(100);

        let request_timeout = Duration::from_secs(
            server_config.request_timeout_seconds(&global_settings)
        );

        Self {
            name: server_config.name.clone(),
            request_tx: tx,
            request_rx: rx,
            server_config: Arc::new(server_config),
            global_settings: Arc::new(global_settings),
            request_timeout,
            pending: Arc::new(PendingRequests {
                articles: Mutex::new(HashMap::new()),
                threads: Mutex::new(HashMap::new()),
                thread: Mutex::new(HashMap::new()),
                groups: Mutex::new(None),
            }),
        }
    }

    /// Get the server name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Spawn worker tasks for this server
    pub fn spawn_workers(&self) {
        let count = self.server_config.worker_count();
        for id in 0..count {
            let worker = NntpWorker::new(
                id,
                (*self.server_config).clone(),
                (*self.global_settings).clone(),
                self.request_rx.clone(),
            );
            tokio::spawn(worker.run());
        }
        tracing::info!(server = %self.name, count, "Spawned NNTP workers");
    }

    /// Fetch an article by message ID
    pub async fn get_article(&self, message_id: &str) -> Result<ArticleView, NntpError> {
        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.articles.lock().await;
        if let Some((tx, started_at)) = pending.get(message_id) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting

                tracing::debug!(server = %self.name, %message_id, "Coalescing with pending article request");
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
        let (tx, _) = broadcast::channel(16);
        pending.insert(message_id.to_string(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetArticle {
                message_id: message_id.to_string(),
                response: resp_tx,
            })
            .await
            .map_err(|_| NntpError("Worker pool closed".into()))?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.articles.lock().await.remove(message_id);
                return Err(NntpError("Worker dropped request".into()));
            }
            Err(_) => {
                self.pending.articles.lock().await.remove(message_id);
                return Err(NntpError("Request timeout".into()));
            }
        };

        // Broadcast to waiters
        let _ = tx.send(result.clone());

        // Cleanup pending
        self.pending.articles.lock().await.remove(message_id);

        result
    }

    /// Fetch recent threads from a newsgroup
    pub async fn get_threads(&self, group: &str, count: u64) -> Result<Vec<ThreadView>, NntpError> {
        let cache_key = format!("{}:{}", group, count);

        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.threads.lock().await;
        if let Some((tx, started_at)) = pending.get(&cache_key) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);

                tracing::debug!(server = %self.name, %group, %count, "Coalescing with pending threads request");
                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, %group, %count, "Pending request timed out, starting new request");
                pending.remove(&cache_key);
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        pending.insert(cache_key.clone(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetThreads {
                group: group.to_string(),
                count,
                response: resp_tx,
            })
            .await
            .map_err(|_| NntpError("Worker pool closed".into()))?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.threads.lock().await.remove(&cache_key);
                return Err(NntpError("Worker dropped request".into()));
            }
            Err(_) => {
                self.pending.threads.lock().await.remove(&cache_key);
                return Err(NntpError("Request timeout".into()));
            }
        };

        // Broadcast to waiters
        let _ = tx.send(result.clone());

        // Cleanup pending
        self.pending.threads.lock().await.remove(&cache_key);

        result
    }

    /// Fetch a single thread by group and root message ID
    pub async fn get_thread(&self, group: &str, message_id: &str) -> Result<ThreadView, NntpError> {
        let cache_key = format!("{}:{}", group, message_id);

        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.thread.lock().await;
        if let Some((tx, started_at)) = pending.get(&cache_key) {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);

                tracing::debug!(server = %self.name, %group, %message_id, "Coalescing with pending thread request");
                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, %group, %message_id, "Pending request timed out, starting new request");
                pending.remove(&cache_key);
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        pending.insert(cache_key.clone(), (tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetThread {
                group: group.to_string(),
                message_id: message_id.to_string(),
                response: resp_tx,
            })
            .await
            .map_err(|_| NntpError("Worker pool closed".into()))?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.thread.lock().await.remove(&cache_key);
                return Err(NntpError("Worker dropped request".into()));
            }
            Err(_) => {
                self.pending.thread.lock().await.remove(&cache_key);
                return Err(NntpError("Request timeout".into()));
            }
        };

        // Broadcast to waiters
        let _ = tx.send(result.clone());

        // Cleanup pending
        self.pending.thread.lock().await.remove(&cache_key);

        result
    }

    /// Fetch the list of available newsgroups
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, NntpError> {
        // Check for pending request (coalesce if not timed out)
        let mut pending = self.pending.groups.lock().await;
        if let Some((tx, started_at)) = pending.as_ref() {
            if started_at.elapsed() < self.request_timeout {
                let mut rx = tx.subscribe();
                drop(pending);

                tracing::debug!(server = %self.name, "Coalescing with pending groups request");
                return match tokio::time::timeout(self.request_timeout, rx.recv()).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err(NntpError("Broadcast channel closed".into())),
                    Err(_) => Err(NntpError("Request timeout".into())),
                };
            } else {
                tracing::debug!(server = %self.name, "Pending groups request timed out, starting new request");
                *pending = None;
            }
        }

        // Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        *pending = Some((tx.clone(), Instant::now()));
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetGroups { response: resp_tx })
            .await
            .map_err(|_| NntpError("Worker pool closed".into()))?;

        // Wait for result with timeout
        let result = match tokio::time::timeout(self.request_timeout, resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                *self.pending.groups.lock().await = None;
                return Err(NntpError("Worker dropped request".into()));
            }
            Err(_) => {
                *self.pending.groups.lock().await = None;
                return Err(NntpError("Request timeout".into()));
            }
        };

        // Broadcast to waiters
        let _ = tx.send(result.clone());

        // Cleanup pending
        *self.pending.groups.lock().await = None;

        result
    }
}
