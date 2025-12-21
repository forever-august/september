//! Federated NNTP Service
//!
//! Provides a unified interface over multiple NNTP servers.
//! Servers are treated as a federated pool sharing the same Usenet backbone.
//! Requests try servers in priority order with fallback on failure.
//! Group lists are merged from all servers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use tokio::sync::{broadcast, RwLock};

use crate::config::{AppConfig, CacheConfig};
use crate::error::AppError;

use super::messages::GroupStatsView;
use super::service::NntpService;
use super::{ArticleView, FlatComment, GroupView, PaginationInfo, ThreadView};

/// Federated NNTP Service that presents multiple servers as one unified source
#[derive(Clone)]
pub struct NntpFederatedService {
    /// Services in priority order (first = primary)
    services: Vec<NntpService>,

    /// Cache for individual articles
    article_cache: Cache<String, ArticleView>,
    /// Cache for thread lists (key: "group:count")
    threads_cache: Cache<String, Vec<ThreadView>>,
    /// Cache for single threads (key: "group:message_id")
    thread_cache: Cache<String, ThreadView>,
    /// Cache for group list (merged from all servers)
    groups_cache: Cache<String, Vec<GroupView>>,
    /// Cache for group stats (article count and last article date)
    group_stats_cache: Cache<String, GroupStatsView>,

    /// Maps group name -> server indices that carry it
    /// Used for smart dispatch of group-specific requests
    group_servers: Arc<RwLock<HashMap<String, Vec<usize>>>>,

    /// Pending group stats requests for coalescing at federated level
    pending_group_stats: Arc<RwLock<HashMap<String, broadcast::Sender<Result<GroupStatsView, String>>>>>,
}

impl NntpFederatedService {
    /// Create a new federated service from configuration
    pub fn new(config: &AppConfig) -> Self {
        let services: Vec<NntpService> = config
            .server
            .iter()
            .map(|server_config| {
                NntpService::new(server_config.clone(), config.nntp.clone())
            })
            .collect();

        Self::with_services(services, &config.cache)
    }

    /// Create a federated service with explicit services and cache config
    pub fn with_services(services: Vec<NntpService>, cache_config: &CacheConfig) -> Self {
        // Build caches with TTL and size limits
        let article_cache = Cache::builder()
            .max_capacity(cache_config.max_articles)
            .time_to_live(Duration::from_secs(cache_config.article_ttl_seconds))
            .build();

        let threads_cache = Cache::builder()
            .max_capacity(cache_config.max_thread_lists)
            .time_to_live(Duration::from_secs(cache_config.threads_ttl_seconds))
            .build();

        let thread_cache = Cache::builder()
            .max_capacity(cache_config.max_thread_lists * 10) // More individual threads than lists
            .time_to_live(Duration::from_secs(cache_config.threads_ttl_seconds))
            .build();

        let groups_cache = Cache::builder()
            .max_capacity(1) // Only one merged groups list
            .time_to_live(Duration::from_secs(cache_config.groups_ttl_seconds))
            .build();

        let group_stats_cache = Cache::builder()
            .max_capacity(cache_config.max_thread_lists) // One per group
            .time_to_live(Duration::from_secs(cache_config.threads_ttl_seconds))
            .build();

        Self {
            services,
            article_cache,
            threads_cache,
            thread_cache,
            groups_cache,
            group_stats_cache,
            group_servers: Arc::new(RwLock::new(HashMap::new())),
            pending_group_stats: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn workers for all servers
    pub fn spawn_workers(&self) {
        for service in &self.services {
            service.spawn_workers();
        }
    }

    /// Get server names for logging/debugging
    pub fn server_names(&self) -> Vec<&str> {
        self.services.iter().map(|s| s.name()).collect()
    }

    /// Get server indices for a group, or all servers if group is unknown
    async fn get_servers_for_group(&self, group: &str) -> Vec<usize> {
        let mapping = self.group_servers.read().await;
        if let Some(indices) = mapping.get(group) {
            tracing::debug!(
                %group,
                servers = ?indices,
                "Dispatching to servers known to carry group"
            );
            indices.clone()
        } else {
            // Group not in mapping - return all servers for fallback/discovery
            tracing::debug!(
                %group,
                "Group not in mapping, trying all servers"
            );
            (0..self.services.len()).collect()
        }
    }

    /// Fetch an article by message ID
    /// Tries each server in order until the article is found
    pub async fn get_article(&self, message_id: &str) -> Result<ArticleView, AppError> {
        // Check cache first
        if let Some(article) = self.article_cache.get(message_id).await {
            tracing::debug!(%message_id, "Article cache hit");
            return Ok(article);
        }

        // Try each server in priority order
        let mut last_error = None;
        for service in &self.services {
            match service.get_article(message_id).await {
                Ok(article) => {
                    // Cache and return
                    self.article_cache
                        .insert(message_id.to_string(), article.clone())
                        .await;
                    tracing::debug!(
                        %message_id,
                        server = %service.name(),
                        "Article fetched from server"
                    );
                    return Ok(article);
                }
                Err(e) => {
                    tracing::debug!(
                        %message_id,
                        server = %service.name(),
                        error = %e,
                        "Article not found on server, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        // All servers failed
        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("No NNTP servers configured".into())))
    }

    /// Fetch recent threads from a newsgroup
    /// Tries only servers known to carry the group (or all servers if group is unknown)
    pub async fn get_threads(&self, group: &str, count: u64) -> Result<Vec<ThreadView>, AppError> {
        let cache_key = format!("{}:{}", group, count);

        // Check cache first
        if let Some(threads) = self.threads_cache.get(&cache_key).await {
            tracing::debug!(%group, %count, "Threads cache hit");
            return Ok(threads);
        }

        // Get servers for this group (smart dispatch)
        let server_indices = self.get_servers_for_group(group).await;

        // Try only relevant servers
        let mut last_error = None;
        for idx in server_indices {
            let service = &self.services[idx];
            match service.get_threads(group, count).await {
                Ok(threads) => {
                    // Cache and return
                    self.threads_cache.insert(cache_key, threads.clone()).await;
                    tracing::debug!(
                        %group,
                        %count,
                        server = %service.name(),
                        thread_count = threads.len(),
                        "Threads fetched from server"
                    );
                    return Ok(threads);
                }
                Err(e) => {
                    tracing::debug!(
                        %group,
                        server = %service.name(),
                        error = %e,
                        "Failed to get threads from server, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        // All servers failed
        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("Group not found on any server".into())))
    }

    /// Fetch paginated threads from a newsgroup.
    /// Fetches a larger batch and returns the requested page slice.
    /// Threads are sorted in reverse-chronological order by last reply date.
    pub async fn get_threads_paginated(
        &self,
        group: &str,
        page: usize,
        per_page: usize,
    ) -> Result<(Vec<ThreadView>, PaginationInfo), AppError> {
        // Fetch a larger batch to enable pagination (e.g., 500 threads)
        const MAX_FETCH: u64 = 500;

        let mut all_threads = self.get_threads(group, MAX_FETCH).await?;

        // Sort threads by last_post_date in reverse-chronological order (newest first)
        // Parse RFC 2822 dates for proper comparison
        all_threads.sort_by(|a, b| {
            let a_parsed = a.last_post_date.as_ref()
                .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok());
            let b_parsed = b.last_post_date.as_ref()
                .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok());
            
            match (b_parsed, a_parsed) {
                (Some(b_dt), Some(a_dt)) => b_dt.cmp(&a_dt),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        let total = all_threads.len();
        let pagination = PaginationInfo::new(page, total, per_page);

        // Slice for current page
        let start = (page - 1) * per_page;
        let end = (start + per_page).min(total);

        let page_threads = if start < total {
            all_threads[start..end].to_vec()
        } else {
            Vec::new()
        };

        Ok((page_threads, pagination))
    }

    /// Fetch a single thread by group and root message ID
    /// Tries only servers known to carry the group (or all servers if group is unknown)
    pub async fn get_thread(&self, group: &str, message_id: &str) -> Result<ThreadView, AppError> {
        let cache_key = format!("{}:{}", group, message_id);

        // Check cache first
        if let Some(thread) = self.thread_cache.get(&cache_key).await {
            tracing::debug!(%group, %message_id, "Thread cache hit");
            return Ok(thread);
        }

        // Get servers for this group (smart dispatch)
        let server_indices = self.get_servers_for_group(group).await;

        // Try only relevant servers
        let mut last_error = None;
        for idx in server_indices {
            let service = &self.services[idx];
            match service.get_thread(group, message_id).await {
                Ok(thread) => {
                    // Cache and return
                    self.thread_cache.insert(cache_key, thread.clone()).await;
                    tracing::debug!(
                        %group,
                        %message_id,
                        server = %service.name(),
                        "Thread fetched from server"
                    );
                    return Ok(thread);
                }
                Err(e) => {
                    tracing::debug!(
                        %group,
                        %message_id,
                        server = %service.name(),
                        error = %e,
                        "Thread not found on server, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        // All servers failed
        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("Thread not found on any server".into())))
    }

    /// Fetch a thread with paginated article bodies.
    /// Only fetches bodies for articles on the current page.
    pub async fn get_thread_paginated(
        &self,
        group: &str,
        message_id: &str,
        page: usize,
        per_page: usize,
        collapse_threshold: usize,
    ) -> Result<(ThreadView, Vec<FlatComment>, PaginationInfo), AppError> {
        // Get thread metadata (uses existing cache)
        let thread = self.get_thread(group, message_id).await?;

        // Flatten and determine which message IDs need bodies
        let (mut comments, pagination, page_msg_ids) =
            thread.root.flatten_paginated(page, per_page, collapse_threshold);

        // Collect bodies: check article cache first, then fetch missing ones
        let mut bodies: HashMap<String, ArticleView> = HashMap::new();
        let mut needed_ids: Vec<String> = Vec::new();

        for msg_id in &page_msg_ids {
            if let Some(article) = self.article_cache.get(msg_id).await {
                bodies.insert(msg_id.clone(), article);
            } else {
                needed_ids.push(msg_id.clone());
            }
        }

        // Fetch missing bodies
        for msg_id in needed_ids {
            match self.get_article(&msg_id).await {
                Ok(article) => {
                    bodies.insert(msg_id, article);
                }
                Err(e) => {
                    tracing::warn!(%msg_id, error = %e, "Failed to fetch article body");
                }
            }
        }

        // Populate bodies in the flattened comments for current page only
        let page_ids_set: std::collections::HashSet<String> =
            page_msg_ids.into_iter().collect();
        let start = (page - 1) * per_page;
        let end = (start + per_page).min(comments.len());

        for (i, comment) in comments.iter_mut().enumerate() {
            if i >= start && i < end && page_ids_set.contains(&comment.message_id) {
                if let Some(fetched) = bodies.get(&comment.message_id) {
                    if let Some(ref mut article) = comment.article {
                        article.body = fetched.body.clone();
                    }
                }
            }
        }

        Ok((thread, comments, pagination))
    }

    /// Fetch the list of available newsgroups
    /// Merges groups from all servers (union) and tracks which servers carry each group
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, AppError> {
        let cache_key = "groups".to_string();

        // Check cache first
        if let Some(groups) = self.groups_cache.get(&cache_key).await {
            tracing::debug!("Groups cache hit");
            return Ok(groups);
        }

        // Collect groups from all servers AND track server associations
        let mut all_groups: Vec<GroupView> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut group_to_servers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut any_success = false;

        for (server_idx, service) in self.services.iter().enumerate() {
            match service.get_groups().await {
                Ok(groups) => {
                    any_success = true;
                    for group in groups {
                        // Track which servers carry this group
                        group_to_servers
                            .entry(group.name.clone())
                            .or_default()
                            .push(server_idx);

                        // Add to all_groups if first time seeing this group
                        if seen_names.insert(group.name.clone()) {
                            all_groups.push(group);
                        }
                    }
                    tracing::debug!(
                        server = %service.name(),
                        server_idx,
                        group_count = all_groups.len(),
                        "Merged groups from server"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %service.name(),
                        error = %e,
                        "Failed to get groups from server"
                    );
                }
            }
        }

        if !any_success {
            return Err(AppError::Internal("Failed to fetch groups from any server".into()));
        }

        // Update group-server mapping atomically
        *self.group_servers.write().await = group_to_servers;

        // Sort by name
        all_groups.sort_by(|a, b| a.name.cmp(&b.name));

        // Cache and return
        self.groups_cache.insert(cache_key, all_groups.clone()).await;
        tracing::debug!(
            total_groups = all_groups.len(),
            server_count = self.services.len(),
            "Groups merged and cached with server associations"
        );

        Ok(all_groups)
    }

    /// Fetch group stats (article count and last article date) from the server.
    /// Tries servers known to carry the group with caching and request coalescing.
    pub async fn get_group_stats(&self, group: &str) -> Result<GroupStatsView, AppError> {
        // Check cache first
        if let Some(stats) = self.group_stats_cache.get(group).await {
            tracing::debug!(%group, "Group stats cache hit");
            return Ok(stats);
        }

        // Check for pending request (coalesce if one is already in flight)
        {
            let pending = self.pending_group_stats.read().await;
            if let Some(tx) = pending.get(group) {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting

                tracing::debug!(%group, "Coalescing with pending federated group stats request");
                return match rx.recv().await {
                    Ok(Ok(stats)) => Ok(stats),
                    Ok(Err(e)) => Err(AppError::Internal(e)),
                    Err(_) => Err(AppError::Internal("Broadcast channel closed".into())),
                };
            }
        }

        // Register pending request
        let (tx, _) = broadcast::channel(16);
        {
            let mut pending = self.pending_group_stats.write().await;
            // Double-check cache and pending after acquiring write lock
            if let Some(stats) = self.group_stats_cache.get(group).await {
                return Ok(stats);
            }
            if let Some(existing_tx) = pending.get(group) {
                let mut rx = existing_tx.subscribe();
                drop(pending);
                return match rx.recv().await {
                    Ok(Ok(stats)) => Ok(stats),
                    Ok(Err(e)) => Err(AppError::Internal(e)),
                    Err(_) => Err(AppError::Internal("Broadcast channel closed".into())),
                };
            }
            pending.insert(group.to_string(), tx.clone());
        }

        // Get servers for this group (smart dispatch)
        let server_indices = self.get_servers_for_group(group).await;

        // Try only relevant servers
        let mut last_error = None;
        let mut result: Option<GroupStatsView> = None;

        for idx in server_indices {
            let service = &self.services[idx];
            match service.get_group_stats(group).await {
                Ok(stats) => {
                    // Cache the result
                    self.group_stats_cache.insert(group.to_string(), stats.clone()).await;
                    tracing::debug!(
                        %group,
                        server = %service.name(),
                        article_count = stats.article_count,
                        "Group stats fetched from server"
                    );
                    result = Some(stats);
                    break;
                }
                Err(e) => {
                    tracing::debug!(
                        %group,
                        server = %service.name(),
                        error = %e,
                        "Failed to get group stats from server, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        // Broadcast result to waiters and cleanup
        {
            let mut pending = self.pending_group_stats.write().await;
            pending.remove(group);
        }

        match result {
            Some(stats) => {
                let _ = tx.send(Ok(stats.clone()));
                Ok(stats)
            }
            None => {
                let err_msg = last_error
                    .map(|e| e.0)
                    .unwrap_or_else(|| "Group stats not available".into());
                let _ = tx.send(Err(err_msg.clone()));
                Err(AppError::Internal(err_msg))
            }
        }
    }

    /// Get cached thread count for a group from the threads cache.
    /// Returns None if threads haven't been fetched for this group.
    async fn get_cached_thread_count(&self, group: &str) -> Option<usize> {
        // Check various cache keys that could contain this group's threads
        for count in [500u64, 100, 50, 25, 20, 10] {
            let cache_key = format!("{}:{}", group, count);
            if let Some(threads) = self.threads_cache.get(&cache_key).await {
                return Some(threads.len());
            }
        }
        None
    }

    /// Get cached thread counts for a list of group names.
    /// Returns a map of group name to thread count.
    pub async fn get_all_cached_thread_counts_for(&self, group_names: &[String]) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for name in group_names {
            if let Some(count) = self.get_cached_thread_count(name).await {
                counts.insert(name.clone(), count);
            }
        }
        counts
    }
}
