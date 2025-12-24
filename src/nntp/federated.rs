//! Federated NNTP Service
//!
//! Provides a unified interface over multiple NNTP servers.
//! Servers are treated as a federated pool sharing the same Usenet backbone.
//! Requests try servers in priority order with fallback on failure.
//! Group lists are merged from all servers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::DateTime;
use moka::future::Cache;
use tokio::sync::{broadcast, RwLock};

use tracing::instrument;

use crate::config::{
    AppConfig, CacheConfig, BROADCAST_CHANNEL_CAPACITY, NEGATIVE_CACHE_SIZE_DIVISOR,
    NNTP_NEGATIVE_CACHE_TTL_SECS, THREAD_CACHE_MULTIPLIER,
};
use crate::error::AppError;

use nntp_rs::OverviewEntry;

use super::messages::GroupStatsView;
use super::service::NntpService;
use super::{merge_articles_into_threads, ArticleView, FlatComment, GroupView, PaginationInfo, ThreadView};

/// Type alias for pending group stats broadcast senders
type PendingGroupStats = HashMap<String, broadcast::Sender<Result<GroupStatsView, String>>>;

/// Cached thread data with high water mark for incremental updates
#[derive(Clone)]
struct CachedThreads {
    threads: Vec<ThreadView>,
    /// Last article number when this cache was populated (high water mark)
    last_article_number: u64,
}

/// Federated NNTP Service that presents multiple servers as one unified source
#[derive(Clone)]
pub struct NntpFederatedService {
    /// Services in priority order (first = primary)
    services: Vec<NntpService>,

    /// Cache for individual articles
    article_cache: Cache<String, ArticleView>,
    /// Cache for not-found articles (negative cache with short TTL)
    article_not_found_cache: Cache<String, ()>,
    /// Cache for thread lists (key: group name)
    /// Stores threads with high water mark for incremental updates
    threads_cache: Cache<String, CachedThreads>,
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
    pending_group_stats: Arc<RwLock<PendingGroupStats>>,

    /// Maximum number of articles to fetch per group (from config)
    max_articles_per_group: u64,
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

        Self::with_services(
            services,
            &config.cache,
            config.nntp.defaults.max_articles_per_group,
        )
    }

    /// Create a federated service with explicit services and cache config
    pub fn with_services(
        services: Vec<NntpService>,
        cache_config: &CacheConfig,
        max_articles_per_group: u64,
    ) -> Self {
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
            .max_capacity(cache_config.max_thread_lists * THREAD_CACHE_MULTIPLIER) // More individual threads than lists
            .time_to_live(Duration::from_secs(cache_config.threads_ttl_seconds))
            .build();

        let groups_cache = Cache::builder()
            .max_capacity(1) // Only one merged groups list
            .time_to_live(Duration::from_secs(cache_config.groups_ttl_seconds))
            .build();

        let group_stats_cache = Cache::builder()
            .max_capacity(cache_config.max_group_stats)
            .time_to_live(Duration::from_secs(cache_config.threads_ttl_seconds))
            .build();

        // Negative cache for not-found articles with short TTL
        let article_not_found_cache = Cache::builder()
            .max_capacity(cache_config.max_articles / NEGATIVE_CACHE_SIZE_DIVISOR) // Quarter the size of positive cache
            .time_to_live(Duration::from_secs(NNTP_NEGATIVE_CACHE_TTL_SECS))
            .build();

        Self {
            services,
            article_cache,
            article_not_found_cache,
            threads_cache,
            thread_cache,
            groups_cache,
            group_stats_cache,
            group_servers: Arc::new(RwLock::new(HashMap::new())),
            pending_group_stats: Arc::new(RwLock::new(HashMap::new())),
            max_articles_per_group,
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

    /// Check if an error indicates a definitive "not found" condition
    /// Returns true for errors that should be negatively cached
    fn is_not_found_error(error: &super::messages::NntpError) -> bool {
        let error_msg = error.0.to_lowercase();
        // NNTP 430 = "No such article"
        // NNTP 423 = "No such article in this group"
        error_msg.contains("430")
            || error_msg.contains("423")
            || error_msg.contains("no such article")
            || error_msg.contains("article not found")
    }

    /// Fetch an article by message ID
    /// Tries each server in order until the article is found
    #[instrument(
        name = "nntp.federated.get_article",
        skip(self),
        fields(cache_hit = false, duration_ms)
    )]
    pub async fn get_article(&self, message_id: &str) -> Result<ArticleView, AppError> {
        let start = Instant::now();
        // Check positive cache first
        if let Some(article) = self.article_cache.get(message_id).await {
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(article);
        }

        // Check negative cache - if we recently determined this article doesn't exist, fail fast
        if self.article_not_found_cache.get(message_id).await.is_some() {
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Err(AppError::ArticleNotFound(message_id.to_string()));
        }

        // Try each server in priority order
        let mut last_error = None;
        let mut all_not_found = true;

        for service in &self.services {
            match service.get_article(message_id).await {
                Ok(article) => {
                    // Cache positive result and return
                    self.article_cache
                        .insert(message_id.to_string(), article.clone())
                        .await;
                    tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(article);
                }
                Err(e) => {

                    // Track if we've seen any non-"not found" errors
                    if !Self::is_not_found_error(&e) {
                        all_not_found = false;
                    }

                    last_error = Some(e);
                }
            }
        }

        // All servers failed - cache negative result if all errors were "not found"
        if all_not_found {
            tracing::debug!(
                %message_id,
                "All servers returned 'not found' - caching negative result"
            );
            self.article_not_found_cache
                .insert(message_id.to_string(), ())
                .await;
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Err(AppError::ArticleNotFound(message_id.to_string()));
        }

        // Had some transient errors - don't cache, just return the error
        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("No NNTP servers configured".into())))
    }

    /// Fetch recent threads from a newsgroup with incremental update support.
    /// On cache hit, checks for new articles and fetches only the delta.
    /// The count parameter is ignored; uses max_articles_per_group from config.
    #[instrument(
        name = "nntp.federated.get_threads",
        skip(self),
        fields(cache_hit = false, duration_ms)
    )]
    pub async fn get_threads(&self, group: &str, _count: u64) -> Result<Vec<ThreadView>, AppError> {
        let start = Instant::now();
        let cache_key = group.to_string();
        let max_articles = self.max_articles_per_group;

        // Check cache first
        if let Some(cached) = self.threads_cache.get(&cache_key).await {
            tracing::Span::current().record("cache_hit", true);

            // Cache hit - check for new articles (incremental update)
            // Try to get new articles since our cached high water mark
            match self.get_new_articles(group, cached.last_article_number).await {
                Ok(new_entries) if new_entries.is_empty() => {
                    // No new articles
                    tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(cached.threads);
                }
                Ok(new_entries) => {
                    // Merge new articles into existing threads
                    let new_hwm = new_entries.iter()
                        .filter_map(|e| e.number())
                        .max()
                        .unwrap_or(cached.last_article_number);

                    let merged = merge_articles_into_threads(&cached.threads, new_entries);

                    // Update cache with merged data
                    self.threads_cache.insert(
                        cache_key,
                        CachedThreads {
                            threads: merged.clone(),
                            last_article_number: new_hwm,
                        },
                    ).await;

                    tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(merged);
                }
                Err(e) => {
                    // Failed to check for new articles, return cached data
                    tracing::warn!(
                        %group,
                        error = %e,
                        "Failed to fetch new articles, returning cached data"
                    );
                    tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(cached.threads);
                }
            }
        }

        // Cache miss - full fetch
        // Get servers for this group (smart dispatch)
        let server_indices = self.get_servers_for_group(group).await;

        // Try only relevant servers
        let mut last_error = None;
        for idx in server_indices {
            let service = &self.services[idx];
            match service.get_threads(group, max_articles).await {
                Ok(threads) => {
                    // Get the high water mark from cached group stats (non-blocking).
                    // If not cached, use 0 and trigger async prefetch.
                    // This prevents blocking thread display on low-priority stats fetch.
                    let last_article_number = self
                        .get_last_article_number_cached(group)
                        .await
                        .unwrap_or_else(|| {
                            // Trigger async prefetch so next request has the HWM
                            self.prefetch_group_stats_if_needed(group);
                            0
                        });

                    // Cache with high water mark
                    self.threads_cache
                        .insert(
                            cache_key,
                            CachedThreads {
                                threads: threads.clone(),
                                last_article_number,
                            },
                        )
                        .await;

                    tracing::Span::current()
                        .record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(threads);
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        // All servers failed
        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("Group not found on any server".into())))
    }

    /// Fetch new articles since a given article number (for incremental updates)
    async fn get_new_articles(
        &self,
        group: &str,
        since_article_number: u64,
    ) -> Result<Vec<OverviewEntry>, AppError> {
        // Get servers for this group
        let server_indices = self.get_servers_for_group(group).await;

        let mut last_error = None;
        for idx in server_indices {
            let service = &self.services[idx];
            match service.get_new_articles(group, since_article_number).await {
                Ok(entries) => {
                    tracing::debug!(
                        %group,
                        since_article_number,
                        server = %service.name(),
                        entry_count = entries.len(),
                        "New articles fetched from server"
                    );
                    return Ok(entries);
                }
                Err(e) => {
                    tracing::debug!(
                        %group,
                        server = %service.name(),
                        error = %e,
                        "Failed to get new articles from server, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .map(|e| AppError::Internal(e.0))
            .unwrap_or_else(|| AppError::Internal("Failed to fetch new articles".into())))
    }

    /// Get the last article number for a group (from cached group stats only).
    /// Returns None if stats are not cached. Does NOT fetch from server to avoid
    /// blocking high-priority operations on low-priority group stats requests.
    async fn get_last_article_number_cached(&self, group: &str) -> Option<u64> {
        if let Some(stats) = self.group_stats_cache.get(group).await {
            return Some(stats.last_article_number);
        }
        None
    }

    /// Trigger async prefetch of group stats if not cached.
    /// Used to populate the high water mark for incremental updates.
    fn prefetch_group_stats_if_needed(&self, group: &str) {
        let group = group.to_string();
        let this = self.clone();
        tokio::spawn(async move {
            // Check cache first to avoid unnecessary work
            if this.group_stats_cache.get(&group).await.is_none() {
                let _ = this.get_group_stats(&group).await;
            }
        });
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
        // Fetch using configured max_articles_per_group
        let mut all_threads = self.get_threads(group, self.max_articles_per_group).await?;

        // Sort threads by last_post_date in reverse-chronological order (newest first)
        // Pre-parse RFC 2822 dates once to avoid O(N log N) parsing overhead
        let mut indexed_threads: Vec<(usize, Option<DateTime<chrono::FixedOffset>>)> = all_threads
            .iter()
            .enumerate()
            .map(|(i, thread)| {
                let parsed = thread.last_post_date.as_ref()
                    .and_then(|d| DateTime::parse_from_rfc2822(d).ok());
                (i, parsed)
            })
            .collect();

        // Sort indices based on pre-parsed dates
        indexed_threads.sort_by(|(_, a_parsed), (_, b_parsed)| {
            match (b_parsed, a_parsed) {
                (Some(b_dt), Some(a_dt)) => b_dt.cmp(a_dt),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        // Reorder original vector based on sorted indices
        let sorted_threads: Vec<ThreadView> = indexed_threads
            .into_iter()
            .map(|(i, _)| all_threads[i].clone())
            .collect();
        all_threads = sorted_threads;

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
    #[instrument(
        name = "nntp.federated.get_thread",
        skip(self),
        fields(cache_hit = false, duration_ms)
    )]
    pub async fn get_thread(&self, group: &str, message_id: &str) -> Result<ThreadView, AppError> {
        let start = Instant::now();
        let cache_key = format!("{}:{}", group, message_id);

        // Check cache first
        if let Some(thread) = self.thread_cache.get(&cache_key).await {
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
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
                    tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(thread);
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        // All servers failed
        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
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

        // Fetch missing bodies concurrently across the worker pool
        // Map each message ID to a fetch future
        let fetch_futures: Vec<_> = needed_ids
            .into_iter()
            .map(|msg_id| async move {
                let result = self.get_article(&msg_id).await;
                (msg_id, result)
            })
            .collect();

        // Execute all fetches concurrently and collect results
        let fetch_results = futures::future::join_all(fetch_futures).await;

        // Process results and populate the bodies map
        for (msg_id, result) in fetch_results {
            match result {
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
    #[instrument(
        name = "nntp.federated.get_groups",
        skip(self),
        fields(cache_hit = false, duration_ms)
    )]
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, AppError> {
        let start = Instant::now();
        let cache_key = "groups".to_string();

        // Check cache first
        if let Some(groups) = self.groups_cache.get(&cache_key).await {
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
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
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Err(AppError::Internal("Failed to fetch groups from any server".into()));
        }

        // Update group-server mapping atomically
        *self.group_servers.write().await = group_to_servers;

        // Sort by name
        all_groups.sort_by(|a, b| a.name.cmp(&b.name));

        // Cache and return
        self.groups_cache.insert(cache_key, all_groups.clone()).await;

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        Ok(all_groups)
    }

    /// Fetch group stats (article count and last article date) from the server.
    /// Tries servers known to carry the group with caching and request coalescing.
    #[instrument(
        name = "nntp.federated.get_group_stats",
        skip(self),
        fields(cache_hit = false, coalesced = false, duration_ms)
    )]
    pub async fn get_group_stats(&self, group: &str) -> Result<GroupStatsView, AppError> {
        let start = Instant::now();
        // Check cache first
        if let Some(stats) = self.group_stats_cache.get(group).await {
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(stats);
        }

        // Check for pending request (coalesce if one is already in flight)
        {
            let pending = self.pending_group_stats.read().await;
            if let Some(tx) = pending.get(group) {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting

                tracing::Span::current().record("coalesced", true);
                return match rx.recv().await {
                    Ok(Ok(stats)) => Ok(stats),
                    Ok(Err(e)) => Err(AppError::Internal(e)),
                    Err(_) => Err(AppError::Internal("Broadcast channel closed".into())),
                };
            }
        }

        // Register pending request
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
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
                    result = Some(stats);
                    break;
                }
                Err(e) => {
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
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                Ok(stats)
            }
            None => {
                let err_msg = last_error
                    .map(|e| e.0)
                    .unwrap_or_else(|| "Group stats not available".into());
                let _ = tx.send(Err(err_msg.clone()));
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                Err(AppError::Internal(err_msg))
            }
        }
    }

    /// Check if group stats are cached (non-blocking, does not fetch)
    pub async fn get_cached_group_stats(&self, group: &str) -> Option<GroupStatsView> {
        self.group_stats_cache.get(group).await
    }

    /// Spawn background tasks to prefetch group stats (fire-and-forget).
    /// Groups that are already cached or have pending requests are handled
    /// efficiently by the existing get_group_stats coalescing logic.
    pub fn prefetch_group_stats(&self, groups: Vec<String>) {
        for group in groups {
            let this = self.clone();
            tokio::spawn(async move {
                // get_group_stats handles caching and request coalescing
                let _ = this.get_group_stats(&group).await;
            });
        }
    }

    /// Get cached thread count for a group from the threads cache.
    /// Returns None if threads haven't been fetched for this group.
    async fn get_cached_thread_count(&self, group: &str) -> Option<usize> {
        if let Some(cached) = self.threads_cache.get(group).await {
            return Some(cached.threads.len());
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
