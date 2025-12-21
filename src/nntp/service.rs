//! NNTP Service with caching and request coalescing
//!
//! Provides a high-level API for NNTP operations with:
//! - Local caching using moka
//! - Request coalescing for concurrent identical requests
//! - Worker pool for parallel NNTP connections

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use moka::future::Cache;
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::config::AppConfig;
use crate::error::AppError;

use super::messages::{NntpError, NntpRequest};
use super::worker::NntpWorker;
use super::{ArticleView, GroupView, ThreadView};

/// Pending request tracking for coalescing
struct PendingRequests {
    articles: Mutex<HashMap<String, broadcast::Sender<Result<ArticleView, NntpError>>>>,
    threads: Mutex<HashMap<String, broadcast::Sender<Result<Vec<ThreadView>, NntpError>>>>,
    thread: Mutex<HashMap<String, broadcast::Sender<Result<ThreadView, NntpError>>>>,
    groups: Mutex<Option<broadcast::Sender<Result<Vec<GroupView>, NntpError>>>>,
}

/// NNTP Service with caching and request coalescing
#[derive(Clone)]
pub struct NntpService {
    /// Request queue sender - workers pull from the receiver
    request_tx: Sender<NntpRequest>,
    /// Request queue receiver - cloned for each worker
    request_rx: Receiver<NntpRequest>,
    /// Configuration for spawning workers
    config: Arc<AppConfig>,

    /// Cache for individual articles
    article_cache: Cache<String, ArticleView>,
    /// Cache for thread lists (key: "group:count")
    threads_cache: Cache<String, Vec<ThreadView>>,
    /// Cache for single threads (key: "group:message_id")
    thread_cache: Cache<String, ThreadView>,
    /// Cache for group list
    groups_cache: Cache<String, Vec<GroupView>>,

    /// Pending requests for coalescing
    pending: Arc<PendingRequests>,
}

impl NntpService {
    /// Create a new NNTP service
    pub fn new(config: &AppConfig) -> Self {
        // Create the request channel with backpressure
        let (tx, rx) = async_channel::bounded(100);

        // Build caches with TTL and size limits
        let article_cache = Cache::builder()
            .max_capacity(config.cache.max_articles)
            .time_to_live(Duration::from_secs(config.cache.article_ttl_seconds))
            .build();

        let threads_cache = Cache::builder()
            .max_capacity(config.cache.max_thread_lists)
            .time_to_live(Duration::from_secs(config.cache.threads_ttl_seconds))
            .build();

        let thread_cache = Cache::builder()
            .max_capacity(config.cache.max_thread_lists * 10) // More individual threads than lists
            .time_to_live(Duration::from_secs(config.cache.threads_ttl_seconds))
            .build();

        let groups_cache = Cache::builder()
            .max_capacity(1) // Only one groups list
            .time_to_live(Duration::from_secs(config.cache.groups_ttl_seconds))
            .build();

        Self {
            request_tx: tx,
            request_rx: rx,
            config: Arc::new(config.clone()),
            article_cache,
            threads_cache,
            thread_cache,
            groups_cache,
            pending: Arc::new(PendingRequests {
                articles: Mutex::new(HashMap::new()),
                threads: Mutex::new(HashMap::new()),
                thread: Mutex::new(HashMap::new()),
                groups: Mutex::new(None),
            }),
        }
    }

    /// Spawn worker tasks
    ///
    /// Call this after creating the service to start the worker pool.
    pub fn spawn_workers(&self, count: usize) {
        for id in 0..count {
            let worker = NntpWorker::new(id, self.config.nntp.clone(), self.request_rx.clone());
            tokio::spawn(worker.run());
        }
        tracing::info!(count, "Spawned NNTP workers");
    }

    /// Fetch an article by message ID
    pub async fn get_article(&self, message_id: &str) -> Result<ArticleView, AppError> {
        // 1. Check cache
        if let Some(article) = self.article_cache.get(message_id).await {
            tracing::debug!(%message_id, "Article cache hit");
            return Ok(article);
        }

        // 2. Check for pending request (coalesce)
        let mut pending = self.pending.articles.lock().await;
        if let Some(tx) = pending.get(message_id) {
            let mut rx = tx.subscribe();
            drop(pending); // Release lock while waiting

            tracing::debug!(%message_id, "Coalescing with pending article request");
            return rx
                .recv()
                .await
                .map_err(|_| AppError::Internal("Broadcast channel closed".into()))?
                .map_err(|e| AppError::Internal(e.0));
        }

        // 3. Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        pending.insert(message_id.to_string(), tx.clone());
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetArticle {
                message_id: message_id.to_string(),
                response: resp_tx,
            })
            .await
            .map_err(|_| AppError::Internal("Worker pool closed".into()))?;

        // 4. Wait for result
        let result = resp_rx
            .await
            .map_err(|_| AppError::Internal("Worker dropped request".into()))?;

        // 5. Cache on success and broadcast to waiters
        if let Ok(ref article) = result {
            self.article_cache
                .insert(message_id.to_string(), article.clone())
                .await;
        }
        let _ = tx.send(result.clone());

        // 6. Cleanup pending
        self.pending.articles.lock().await.remove(message_id);

        result.map_err(|e| AppError::Internal(e.0))
    }

    /// Fetch recent threads from a newsgroup
    pub async fn get_threads(&self, group: &str, count: u64) -> Result<Vec<ThreadView>, AppError> {
        let cache_key = format!("{}:{}", group, count);

        // 1. Check cache
        if let Some(threads) = self.threads_cache.get(&cache_key).await {
            tracing::debug!(%group, %count, "Threads cache hit");
            return Ok(threads);
        }

        // 2. Check for pending request (coalesce)
        let mut pending = self.pending.threads.lock().await;
        if let Some(tx) = pending.get(&cache_key) {
            let mut rx = tx.subscribe();
            drop(pending);

            tracing::debug!(%group, %count, "Coalescing with pending threads request");
            return rx
                .recv()
                .await
                .map_err(|_| AppError::Internal("Broadcast channel closed".into()))?
                .map_err(|e| AppError::Internal(e.0));
        }

        // 3. Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        pending.insert(cache_key.clone(), tx.clone());
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetThreads {
                group: group.to_string(),
                count,
                response: resp_tx,
            })
            .await
            .map_err(|_| AppError::Internal("Worker pool closed".into()))?;

        // 4. Wait for result
        let result = resp_rx
            .await
            .map_err(|_| AppError::Internal("Worker dropped request".into()))?;

        // 5. Cache on success and broadcast to waiters
        if let Ok(ref threads) = result {
            self.threads_cache.insert(cache_key.clone(), threads.clone()).await;
        }
        let _ = tx.send(result.clone());

        // 6. Cleanup pending
        self.pending.threads.lock().await.remove(&cache_key);

        result.map_err(|e| AppError::Internal(e.0))
    }

    /// Fetch a single thread by group and root message ID
    pub async fn get_thread(&self, group: &str, message_id: &str) -> Result<ThreadView, AppError> {
        let cache_key = format!("{}:{}", group, message_id);

        // 1. Check cache
        if let Some(thread) = self.thread_cache.get(&cache_key).await {
            tracing::debug!(%group, %message_id, "Thread cache hit");
            return Ok(thread);
        }

        // 2. Check for pending request (coalesce)
        let mut pending = self.pending.thread.lock().await;
        if let Some(tx) = pending.get(&cache_key) {
            let mut rx = tx.subscribe();
            drop(pending);

            tracing::debug!(%group, %message_id, "Coalescing with pending thread request");
            return rx
                .recv()
                .await
                .map_err(|_| AppError::Internal("Broadcast channel closed".into()))?
                .map_err(|e| AppError::Internal(e.0));
        }

        // 3. Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        pending.insert(cache_key.clone(), tx.clone());
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetThread {
                group: group.to_string(),
                message_id: message_id.to_string(),
                response: resp_tx,
            })
            .await
            .map_err(|_| AppError::Internal("Worker pool closed".into()))?;

        // 4. Wait for result
        let result = resp_rx
            .await
            .map_err(|_| AppError::Internal("Worker dropped request".into()))?;

        // 5. Cache on success and broadcast to waiters
        if let Ok(ref thread) = result {
            self.thread_cache.insert(cache_key.clone(), thread.clone()).await;
        }
        let _ = tx.send(result.clone());

        // 6. Cleanup pending
        self.pending.thread.lock().await.remove(&cache_key);

        result.map_err(|e| AppError::Internal(e.0))
    }

    /// Fetch the list of available newsgroups
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, AppError> {
        let cache_key = "groups".to_string();

        // 1. Check cache
        if let Some(groups) = self.groups_cache.get(&cache_key).await {
            tracing::debug!("Groups cache hit");
            return Ok(groups);
        }

        // 2. Check for pending request (coalesce)
        let mut pending = self.pending.groups.lock().await;
        if let Some(tx) = pending.as_ref() {
            let mut rx = tx.subscribe();
            drop(pending);

            tracing::debug!("Coalescing with pending groups request");
            return rx
                .recv()
                .await
                .map_err(|_| AppError::Internal("Broadcast channel closed".into()))?
                .map_err(|e| AppError::Internal(e.0));
        }

        // 3. Register pending request and send to worker
        let (tx, _) = broadcast::channel(16);
        *pending = Some(tx.clone());
        drop(pending);

        let (resp_tx, resp_rx) = oneshot::channel();
        self.request_tx
            .send(NntpRequest::GetGroups { response: resp_tx })
            .await
            .map_err(|_| AppError::Internal("Worker pool closed".into()))?;

        // 4. Wait for result
        let result = resp_rx
            .await
            .map_err(|_| AppError::Internal("Worker dropped request".into()))?;

        // 5. Cache on success and broadcast to waiters
        if let Ok(ref groups) = result {
            self.groups_cache.insert(cache_key, groups.clone()).await;
        }
        let _ = tx.send(result.clone());

        // 6. Cleanup pending
        *self.pending.groups.lock().await = None;

        result.map_err(|e| AppError::Internal(e.0))
    }

    /// Invalidate all cached data (useful for testing or admin operations)
    pub fn invalidate_all(&self) {
        self.article_cache.invalidate_all();
        self.threads_cache.invalidate_all();
        self.thread_cache.invalidate_all();
        self.groups_cache.invalidate_all();
    }
}
