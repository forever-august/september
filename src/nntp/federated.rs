//! Federated NNTP Service
//!
//! Provides a unified interface over multiple NNTP servers.
//! Servers are treated as a federated pool sharing the same Usenet backbone.
//! Requests try servers in priority order with fallback on failure.
//! Group lists are merged from all servers.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::DateTime;
use moka::future::Cache;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

use tracing::instrument;

use crate::config::{
    AppConfig, CacheConfig, ACTIVITY_BUCKET_COUNT, ACTIVITY_HIGH_RPS, ACTIVITY_WINDOW_SECS,
    BACKGROUND_REFRESH_MAX_PERIOD_SECS, BACKGROUND_REFRESH_MIN_PERIOD_SECS,
    BROADCAST_CHANNEL_CAPACITY, GROUP_STATS_REFRESH_INTERVAL_SECS, INCREMENTAL_DEBOUNCE_MS,
    NEGATIVE_CACHE_SIZE_DIVISOR, NNTP_NEGATIVE_CACHE_TTL_SECS, POST_POLL_INTERVAL_MS,
    POST_POLL_MAX_ATTEMPTS, THREAD_CACHE_MULTIPLIER,
};
use crate::error::AppError;

use nntp_rs::OverviewEntry;

use super::messages::GroupStatsView;
use super::service::NntpService;
use super::{
    add_reply_to_node, compute_timeago, merge_articles_into_thread, merge_articles_into_threads,
    ArticleView, FlatComment, GroupView, PaginationInfo, ThreadNodeView, ThreadView,
};

/// Type alias for pending group stats broadcast senders
type PendingGroupStats = HashMap<String, broadcast::Sender<Result<GroupStatsView, String>>>;

/// Type alias for pending incremental update broadcast senders
type PendingIncremental =
    HashMap<String, broadcast::Sender<Result<Arc<Vec<OverviewEntry>>, String>>>;

/// Type alias for pending groups list broadcast sender (single global request)
type PendingGroups = Option<broadcast::Sender<Result<Vec<GroupView>, String>>>;

/// Tracks request activity for a single group using a circular buffer of time buckets.
/// Enables calculation of a 5-minute moving average request rate.
struct GroupActivity {
    /// Circular buffer of request counts
    buckets: Vec<u32>,
    /// Index of the current bucket
    current_bucket: usize,
    /// Bucket index corresponding to bucket_start_secs (for tracking time progression)
    bucket_start_idx: u64,
    /// Total requests in all buckets (for fast average calculation)
    total_requests: u64,
    /// Handle to the group's refresh task (for cancellation on activity change)
    refresh_task: Option<tokio::task::JoinHandle<()>>,
}

/// Seconds per bucket = window size / bucket count
const BUCKET_GRANULARITY_SECS: u64 = ACTIVITY_WINDOW_SECS / ACTIVITY_BUCKET_COUNT;

impl GroupActivity {
    fn new() -> Self {
        Self {
            buckets: vec![0; ACTIVITY_BUCKET_COUNT as usize],
            current_bucket: 0,
            bucket_start_idx: 0,
            total_requests: 0,
            refresh_task: None,
        }
    }

    /// Convert seconds to bucket index
    fn secs_to_bucket_idx(secs: u64) -> u64 {
        secs / BUCKET_GRANULARITY_SECS
    }

    /// Record a request, advancing buckets if necessary.
    /// `now_secs` is seconds since an arbitrary epoch (we use Instant-based).
    fn record_request(&mut self, now_secs: u64) {
        self.advance_to(now_secs);
        self.buckets[self.current_bucket] = self.buckets[self.current_bucket].saturating_add(1);
        self.total_requests += 1;
    }

    /// Advance the bucket pointer to the given time, clearing old buckets.
    fn advance_to(&mut self, now_secs: u64) {
        let now_idx = Self::secs_to_bucket_idx(now_secs);

        if self.bucket_start_idx == 0 && self.total_requests == 0 {
            // First request - initialize
            self.bucket_start_idx = now_idx;
            return;
        }

        let elapsed_buckets = now_idx.saturating_sub(self.bucket_start_idx);
        if elapsed_buckets == 0 {
            return; // Still in the same bucket
        }

        // Clear buckets for elapsed time periods
        let buckets_to_clear = elapsed_buckets.min(ACTIVITY_BUCKET_COUNT) as usize;
        for i in 1..=buckets_to_clear {
            let idx = (self.current_bucket + i) % ACTIVITY_BUCKET_COUNT as usize;
            self.total_requests = self.total_requests.saturating_sub(self.buckets[idx] as u64);
            self.buckets[idx] = 0;
        }

        // Move to the new bucket
        self.current_bucket =
            (self.current_bucket + (elapsed_buckets as usize)) % ACTIVITY_BUCKET_COUNT as usize;
        self.bucket_start_idx = now_idx;
    }

    /// Calculate requests per second (5-minute moving average).
    fn requests_per_second(&mut self, now_secs: u64) -> f64 {
        self.advance_to(now_secs);
        self.total_requests as f64 / ACTIVITY_WINDOW_SECS as f64
    }

    /// Check if the group is inactive (no requests in the window).
    fn is_inactive(&mut self, now_secs: u64) -> bool {
        self.advance_to(now_secs);
        self.total_requests == 0
    }
}

/// Tracks activity for all groups
#[derive(Default)]
struct ActivityTracker {
    groups: HashMap<String, GroupActivity>,
    /// Epoch for calculating seconds (set on first use)
    epoch: Option<Instant>,
}

impl ActivityTracker {
    fn new() -> Self {
        Self {
            groups: HashMap::new(),
            epoch: None,
        }
    }

    /// Get seconds since our epoch
    fn now_secs(&mut self) -> u64 {
        let now = Instant::now();
        match self.epoch {
            Some(epoch) => now.duration_since(epoch).as_secs(),
            None => {
                self.epoch = Some(now);
                0
            }
        }
    }

    /// Record a request for a group
    fn record_request(&mut self, group: &str) {
        let now_secs = self.now_secs();
        self.groups
            .entry(group.to_string())
            .or_insert_with(GroupActivity::new)
            .record_request(now_secs);
    }

    /// Get the requests per second for a group
    fn requests_per_second(&mut self, group: &str) -> f64 {
        let now_secs = self.now_secs();
        self.groups
            .get_mut(group)
            .map(|a| a.requests_per_second(now_secs))
            .unwrap_or(0.0)
    }

    /// Get all active groups (with any activity in the window)
    fn active_groups(&mut self) -> Vec<String> {
        let now_secs = self.now_secs();
        self.groups
            .retain(|_, activity| !activity.is_inactive(now_secs));
        self.groups.keys().cloned().collect()
    }

    /// Set the refresh task handle for a group
    fn set_refresh_task(&mut self, group: &str, task: tokio::task::JoinHandle<()>) {
        if let Some(activity) = self.groups.get_mut(group) {
            // Cancel existing task if any
            if let Some(old_task) = activity.refresh_task.take() {
                old_task.abort();
            }
            activity.refresh_task = Some(task);
        }
    }

    /// Check if a group has a running refresh task
    fn has_refresh_task(&self, group: &str) -> bool {
        self.groups
            .get(group)
            .and_then(|a| a.refresh_task.as_ref())
            .map(|t| !t.is_finished())
            .unwrap_or(false)
    }
}

/// Cached thread data with high water mark for incremental updates
#[derive(Clone)]
struct CachedThreads {
    threads: Vec<ThreadView>,
    /// Last article number when this cache was populated (high water mark)
    last_article_number: u64,
}

/// Cached single thread data with group info for incremental updates
#[derive(Clone)]
struct CachedThread {
    thread: ThreadView,
    /// Group name for incremental update queries (stored for potential future use)
    #[allow(dead_code)]
    group: String,
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
    thread_cache: Cache<String, CachedThread>,
    /// Cache for group list (merged from all servers)
    groups_cache: Cache<String, Vec<GroupView>>,
    /// Cache for group stats (article count and last article date)
    group_stats_cache: Cache<String, GroupStatsView>,

    /// Maps group name -> server indices that carry it
    /// Used for smart dispatch of group-specific requests
    group_servers: Arc<RwLock<HashMap<String, Vec<usize>>>>,

    /// Maps group name -> server indices that allow posting
    /// Updated during group fetch when POST capability is detected
    posting_servers: Arc<RwLock<HashMap<String, Vec<usize>>>>,

    /// Pending group stats requests for coalescing at federated level
    pending_group_stats: Arc<RwLock<PendingGroupStats>>,

    /// Per-group high water mark (last known article number)
    group_hwm: Arc<RwLock<HashMap<String, u64>>>,

    /// Last incremental check time per group (for debouncing)
    last_incremental_check: Arc<RwLock<HashMap<String, Instant>>>,

    /// Pending incremental update requests for coalescing (key: group name)
    pending_incremental: Arc<RwLock<PendingIncremental>>,

    /// Activity tracker for background refresh scheduling
    activity_tracker: Arc<RwLock<ActivityTracker>>,

    /// Task handles for per-group stats refresh (for cleanup when groups are removed)
    group_stats_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,

    /// Maximum number of articles to fetch per group (from config)
    max_articles_per_group: u64,

    /// Last time we refreshed the groups list (for stale-while-revalidate debouncing)
    last_groups_refresh: Arc<RwLock<Option<Instant>>>,

    /// Pending groups list request for coalescing (only one can be in flight)
    pending_groups: Arc<RwLock<PendingGroups>>,
}

impl NntpFederatedService {
    /// Create a new federated service from configuration
    pub fn new(config: &AppConfig) -> Self {
        let services: Vec<NntpService> = config
            .server
            .iter()
            .map(|server_config| NntpService::new(server_config.clone(), config.nntp.clone()))
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
            posting_servers: Arc::new(RwLock::new(HashMap::new())),
            pending_group_stats: Arc::new(RwLock::new(HashMap::new())),
            group_hwm: Arc::new(RwLock::new(HashMap::new())),
            last_incremental_check: Arc::new(RwLock::new(HashMap::new())),
            pending_incremental: Arc::new(RwLock::new(HashMap::new())),
            activity_tracker: Arc::new(RwLock::new(ActivityTracker::new())),
            group_stats_tasks: Arc::new(RwLock::new(HashMap::new())),
            max_articles_per_group,
            last_groups_refresh: Arc::new(RwLock::new(None)),
            pending_groups: Arc::new(RwLock::new(None)),
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

    /// Check if an error indicates a "group not found" condition
    /// NNTP 411 = "No such newsgroup"
    fn is_group_not_found_error(error: &super::messages::NntpError) -> bool {
        let error_msg = error.0.to_lowercase();
        error_msg.contains("411")
            || error_msg.contains("no such newsgroup")
            || error_msg.contains("group not found")
    }

    /// Convert an NNTP error to an appropriate AppError
    fn nntp_error_to_app_error(error: super::messages::NntpError, group: &str) -> AppError {
        if Self::is_group_not_found_error(&error) {
            AppError::GroupNotFound(group.to_string())
        } else {
            AppError::Internal(error.0)
        }
    }

    // =========================================================================
    // Incremental Update Helpers
    // =========================================================================

    /// Check if we should perform an incremental update check for a group.
    /// Returns true if the debounce period has elapsed, and updates the timestamp.
    /// This ensures at most one NNTP check per second per group.
    async fn should_check_incremental(&self, group: &str) -> bool {
        let now = Instant::now();
        let debounce_duration = Duration::from_millis(INCREMENTAL_DEBOUNCE_MS);

        let mut last_check = self.last_incremental_check.write().await;

        if let Some(last) = last_check.get(group) {
            if now.duration_since(*last) < debounce_duration {
                tracing::trace!(%group, "Incremental check debounced");
                return false;
            }
        }

        last_check.insert(group.to_string(), now);
        true
    }

    /// Mark a group as active (for background refresh tracking).
    /// Called when users view thread listings or threads in a group.
    /// Also records the request for activity-proportional refresh rate calculation.
    async fn mark_group_active(&self, group: &str) {
        let mut tracker = self.activity_tracker.write().await;
        tracker.record_request(group);

        // Check if we need to spawn/update a refresh task for this group
        if !tracker.has_refresh_task(group) {
            drop(tracker); // Release lock before spawning
            self.spawn_group_refresh_task(group.to_string()).await;
        }
    }

    /// Get the current high water mark for a group, or 0 if unknown.
    async fn get_group_hwm(&self, group: &str) -> u64 {
        self.group_hwm.read().await.get(group).copied().unwrap_or(0)
    }

    /// Update the high water mark for a group (takes the max of current and new).
    async fn update_group_hwm(&self, group: &str, new_hwm: u64) {
        let mut hwm = self.group_hwm.write().await;
        let current = hwm.get(group).copied().unwrap_or(0);
        if new_hwm > current {
            hwm.insert(group.to_string(), new_hwm);
        }
    }

    /// Fetch new articles for a group with request coalescing.
    /// Multiple concurrent requests for the same group will share a single NNTP request.
    #[instrument(
        name = "nntp.federated.get_new_articles_coalesced",
        skip(self),
        fields(group = %group, coalesced = false, debounced = false, new_count, duration_ms)
    )]
    async fn get_new_articles_coalesced(
        &self,
        group: &str,
    ) -> Result<Vec<OverviewEntry>, AppError> {
        let start = Instant::now();

        // Check debounce first
        if !self.should_check_incremental(group).await {
            tracing::Span::current().record("debounced", true);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(Vec::new());
        }

        // Get current HWM for this group
        let hwm = self.get_group_hwm(group).await;
        if hwm == 0 {
            // No HWM yet - trigger stats fetch and return empty
            // This happens on first access before any full fetch
            self.prefetch_group_stats_if_needed(group);
            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(Vec::new());
        }

        // Check for pending request (coalesce if one is already in flight)
        {
            let pending = self.pending_incremental.read().await;
            if let Some(tx) = pending.get(group) {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting

                tracing::Span::current().record("coalesced", true);
                let result = match rx.recv().await {
                    Ok(Ok(entries)) => Ok((*entries).clone()),
                    Ok(Err(e)) => Err(AppError::Internal(e)),
                    Err(_) => Err(AppError::Internal("Broadcast channel closed".into())),
                };
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                return result;
            }
        }

        // Register pending request
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        {
            let mut pending = self.pending_incremental.write().await;
            // Double-check after acquiring write lock
            if let Some(existing_tx) = pending.get(group) {
                let mut rx = existing_tx.subscribe();
                drop(pending);
                tracing::Span::current().record("coalesced", true);
                let result = match rx.recv().await {
                    Ok(Ok(entries)) => Ok((*entries).clone()),
                    Ok(Err(e)) => Err(AppError::Internal(e)),
                    Err(_) => Err(AppError::Internal("Broadcast channel closed".into())),
                };
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                return result;
            }
            pending.insert(group.to_string(), tx.clone());
        }

        // Perform the actual fetch
        let result = self.get_new_articles(group, hwm).await;

        // Update HWM on success
        if let Ok(ref entries) = result {
            if let Some(max_num) = entries.iter().filter_map(|e| e.number()).max() {
                self.update_group_hwm(group, max_num).await;
            }
            tracing::Span::current().record("new_count", entries.len());
        }

        // Broadcast result to waiters and cleanup
        {
            let mut pending = self.pending_incremental.write().await;
            pending.remove(group);
        }

        let broadcast_result = result
            .as_ref()
            .map(|v| Arc::new(v.clone()))
            .map_err(|e| e.to_string());
        let _ = tx.send(broadcast_result);

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Get list of currently active groups (with any activity in the window).
    /// Also cleans up stale entries.
    #[allow(dead_code)] // Useful for debugging/monitoring
    pub async fn get_active_groups(&self) -> Vec<String> {
        self.activity_tracker.write().await.active_groups()
    }

    /// Calculate refresh period based on request rate using log10 scale.
    /// - 10,000 requests/second -> 1 second refresh period
    /// - Any activity at all -> 30 second refresh period  
    /// - Scales logarithmically between these extremes
    fn calculate_refresh_period(requests_per_second: f64) -> Duration {
        if requests_per_second <= 0.0 {
            return Duration::from_secs(BACKGROUND_REFRESH_MAX_PERIOD_SECS);
        }

        // log10(10000) = 4 -> 1s
        // log10(1/300) ≈ -2.48 -> 30s (minimum activity = 1 request in 5 minutes)
        // We use the formula: period = max - (max - min) * (log10(rps) - log_min) / (log_max - log_min)

        let log_rps = requests_per_second.log10();
        let log_min = (1.0 / ACTIVITY_WINDOW_SECS as f64).log10(); // ~-2.48 for 300s window
        let log_max = ACTIVITY_HIGH_RPS.log10(); // 4.0 for 10k rps

        // Clamp to range
        let log_clamped = log_rps.clamp(log_min, log_max);

        // Linear interpolation in log space
        let ratio = (log_clamped - log_min) / (log_max - log_min);
        let period_secs = BACKGROUND_REFRESH_MAX_PERIOD_SECS as f64
            - ratio
                * (BACKGROUND_REFRESH_MAX_PERIOD_SECS - BACKGROUND_REFRESH_MIN_PERIOD_SECS) as f64;

        Duration::from_secs_f64(period_secs.max(BACKGROUND_REFRESH_MIN_PERIOD_SECS as f64))
    }

    /// Spawn a per-group refresh task that runs at an activity-proportional rate.
    async fn spawn_group_refresh_task(&self, group: String) {
        let this = self.clone();
        let group_clone = group.clone();

        tracing::debug!(group = %group, "Spawning background refresh task");

        let task = tokio::spawn(async move {
            loop {
                // Get current request rate and calculate refresh period
                let rps = {
                    let mut tracker = this.activity_tracker.write().await;
                    tracker.requests_per_second(&group_clone)
                };

                let period = Self::calculate_refresh_period(rps);

                tracing::debug!(
                    group = %group_clone,
                    rps = %format!("{:.2}", rps),
                    period_secs = %period.as_secs_f64(),
                    "Group refresh scheduled"
                );

                tokio::time::sleep(period).await;

                // Check if group is still active before refreshing
                let still_active = {
                    let mut tracker = this.activity_tracker.write().await;
                    let active = tracker.active_groups();
                    active.contains(&group_clone)
                };

                if !still_active {
                    tracing::debug!(group = %group_clone, "Group inactive, stopping refresh task");
                    break;
                }

                // Perform the refresh
                this.trigger_incremental_update(&group_clone).await;
            }
        });

        // Store the task handle
        self.activity_tracker
            .write()
            .await
            .set_refresh_task(&group, task);
    }

    /// Trigger an incremental update for a group (used by background refresh).
    /// Updates the threads cache if new articles are found.
    pub async fn trigger_incremental_update(&self, group: &str) {
        let span = tracing::info_span!("background.group_refresh", %group);
        let _guard = span.enter();

        match self.get_new_articles_coalesced(group).await {
            Ok(new_entries) if new_entries.is_empty() => {
                tracing::trace!(%group, "No new articles");
            }
            Ok(new_entries) => {
                tracing::debug!(%group, count = new_entries.len(), "Found new articles");

                // Update threads cache if it exists
                if let Some(cached) = self.threads_cache.get(group).await {
                    let new_hwm = new_entries
                        .iter()
                        .filter_map(|e| e.number())
                        .max()
                        .unwrap_or(cached.last_article_number);

                    let merged = super::merge_articles_into_threads(&cached.threads, new_entries);

                    self.threads_cache
                        .insert(
                            group.to_string(),
                            CachedThreads {
                                threads: merged,
                                last_article_number: new_hwm,
                            },
                        )
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!(%group, error = %e, "Failed to fetch new articles");
            }
        }
    }

    /// Check if an article exists on any configured server using STAT command.
    ///
    /// This is faster than get_article as it doesn't transfer content.
    async fn check_article_exists(&self, message_id: &str) -> bool {
        for service in &self.services {
            if service
                .check_article_exists(message_id)
                .await
                .unwrap_or(false)
            {
                return true;
            }
        }
        false
    }

    /// Inject a pre-built article into cache after confirming server-side existence.
    ///
    /// Polls with STAT command until article exists, then injects the pre-built
    /// ArticleView into caches. This is faster than fetching the full article
    /// since we already have all the article data from the post submission.
    ///
    /// # Arguments
    /// * `group` - The newsgroup the article was posted to
    /// * `article` - Pre-built ArticleView from post data
    /// * `root_message_id` - For replies, the root thread's message ID (for cache key)
    /// * `parent_message_id` - For replies, the direct parent's message ID (for tree insertion)
    pub async fn inject_posted_article(
        &self,
        group: &str,
        article: ArticleView,
        root_message_id: Option<&str>,
        parent_message_id: Option<&str>,
    ) {
        let message_id = &article.message_id;

        for attempt in 1..=POST_POLL_MAX_ATTEMPTS {
            // Invalidate negative cache before each attempt
            self.article_not_found_cache.invalidate(message_id).await;

            if self.check_article_exists(message_id).await {
                tracing::debug!(
                    %group,
                    %message_id,
                    attempt,
                    "Article confirmed via STAT, injecting into cache"
                );

                // Cache the article for future fetches
                self.article_cache
                    .insert(message_id.to_string(), article.clone())
                    .await;

                // Inject into threads/thread caches
                self.inject_article_into_caches(group, article, root_message_id, parent_message_id)
                    .await;
                return;
            }

            if attempt < POST_POLL_MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(POST_POLL_INTERVAL_MS)).await;
            }
        }

        // Timeout - article didn't appear within polling window
        // Fall back to triggering incremental update
        tracing::warn!(
            %group,
            %message_id,
            max_attempts = POST_POLL_MAX_ATTEMPTS,
            "Article not confirmed via STAT after polling, falling back to incremental update"
        );

        // Invalidate thread cache if this was a reply so next request fetches fresh
        if let Some(root_id) = root_message_id {
            let cache_key = format!("{}:{}", group, root_id);
            self.thread_cache.invalidate(&cache_key).await;
        }

        self.trigger_incremental_update(group).await;
    }

    /// Inject a fetched article into threads_cache and thread_cache.
    async fn inject_article_into_caches(
        &self,
        group: &str,
        article: ArticleView,
        root_message_id: Option<&str>,
        parent_message_id: Option<&str>,
    ) {
        let message_id = article.message_id.clone();

        match (root_message_id, parent_message_id) {
            (None, None) => {
                // New thread - add to threads_cache
                self.inject_new_thread(group, article).await;
            }
            (Some(root_id), Some(parent_id)) => {
                // Reply - update both caches
                self.inject_reply(group, root_id, parent_id, article).await;
            }
            _ => {
                // Shouldn't happen, but fall back to incremental update
                tracing::warn!(
                    %group,
                    %message_id,
                    "Unexpected cache injection state, triggering incremental update"
                );
                self.trigger_incremental_update(group).await;
            }
        }
    }

    /// Inject a new thread (root post) into threads_cache.
    async fn inject_new_thread(&self, group: &str, article: ArticleView) {
        let date_relative = Some(compute_timeago(&article.date));

        // Create a new ThreadView for this article
        let new_thread = ThreadView {
            subject: article.subject.clone(),
            root_message_id: article.message_id.clone(),
            article_count: 1,
            root: ThreadNodeView {
                message_id: article.message_id.clone(),
                article: Some(article.clone()),
                replies: Vec::new(),
                descendant_count: 0,
            },
            last_post_date: Some(article.date.clone()),
            last_post_date_relative: date_relative,
        };

        // Get existing cache or create empty base
        let (mut threads, last_article_number) =
            if let Some(cached) = self.threads_cache.get(group).await {
                (cached.threads.clone(), cached.last_article_number)
            } else {
                // No cache exists - start fresh with just this thread
                // Note: last_article_number of 0 will trigger a full refresh on next incremental check,
                // which is fine since we're bootstrapping the cache
                (Vec::new(), 0)
            };

        // Prepend to thread list (newest first)
        threads.insert(0, new_thread);

        tracing::debug!(
            %group,
            message_id = %article.message_id,
            "Injected new thread into cache"
        );

        self.threads_cache
            .insert(
                group.to_string(),
                CachedThreads {
                    threads,
                    last_article_number,
                },
            )
            .await;
    }

    /// Inject a reply into both threads_cache and thread_cache.
    async fn inject_reply(
        &self,
        group: &str,
        root_msg_id: &str,
        parent_msg_id: &str,
        article: ArticleView,
    ) {
        let new_node = ThreadNodeView {
            message_id: article.message_id.clone(),
            article: Some(article.clone()),
            replies: Vec::new(),
            descendant_count: 0,
        };

        // Update thread_cache
        let cache_key = format!("{}:{}", group, root_msg_id);
        if let Some(cached) = self.thread_cache.get(&cache_key).await {
            let mut thread = cached.thread.clone();

            // Add reply to the appropriate parent node
            if add_reply_to_node(&mut thread.root, parent_msg_id, new_node.clone()) {
                thread.article_count += 1;
                thread.last_post_date = Some(article.date.clone());
                thread.last_post_date_relative = Some(compute_timeago(&article.date));

                tracing::debug!(
                    %group,
                    %root_msg_id,
                    message_id = %article.message_id,
                    "Injected reply into thread_cache"
                );

                self.thread_cache
                    .insert(
                        cache_key,
                        CachedThread {
                            thread,
                            group: group.to_string(),
                        },
                    )
                    .await;
            }
        }

        // Update threads_cache (for reply count/last post date in list view)
        if let Some(cached) = self.threads_cache.get(group).await {
            let mut threads = cached.threads.clone();

            if let Some(thread) = threads
                .iter_mut()
                .find(|t| t.root_message_id == root_msg_id)
            {
                // Add reply to thread tree
                if add_reply_to_node(&mut thread.root, parent_msg_id, new_node) {
                    thread.article_count += 1;
                    thread.last_post_date = Some(article.date.clone());
                    thread.last_post_date_relative = Some(compute_timeago(&article.date));

                    tracing::debug!(
                        %group,
                        %root_msg_id,
                        message_id = %article.message_id,
                        "Injected reply into threads_cache"
                    );
                }
            }

            self.threads_cache
                .insert(
                    group.to_string(),
                    CachedThreads {
                        threads,
                        last_article_number: cached.last_article_number,
                    },
                )
                .await;
        }
    }

    /// Initialize background refresh system.
    /// With activity-proportional refresh, individual group tasks are spawned
    /// on-demand when groups become active. This method is kept for API compatibility
    /// and logs that the system is ready.
    pub fn spawn_background_refresh(self: Arc<Self>) {
        tracing::info!(
            "Activity-proportional background refresh enabled: \
             {}-{}s refresh period based on request rate",
            BACKGROUND_REFRESH_MIN_PERIOD_SECS,
            BACKGROUND_REFRESH_MAX_PERIOD_SECS
        );
        // Per-group refresh tasks are spawned on-demand in mark_group_active()

        // Spawn hourly group stats refresh
        self.spawn_group_stats_refresh();
    }

    /// Spawn a periodic task to refresh stats for a single group.
    /// Runs forever, refreshing once per hour.
    fn spawn_group_stats_refresh_task(&self, group: String) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let _ = this.get_group_stats(&group).await;
                tokio::time::sleep(Duration::from_secs(GROUP_STATS_REFRESH_INTERVAL_SECS)).await;
            }
        })
    }

    /// Spawn background refresh coordinator for group stats.
    /// Monitors for new/removed groups and manages per-group refresh tasks.
    fn spawn_group_stats_refresh(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                if let Ok(groups) = self.get_groups().await {
                    let current_names: HashSet<String> =
                        groups.iter().map(|g| g.name.clone()).collect();

                    let mut tasks = self.group_stats_tasks.write().await;

                    // Abort tasks for removed groups
                    tasks.retain(|name, handle| {
                        if current_names.contains(name) {
                            true
                        } else {
                            handle.abort();
                            tracing::debug!(group = %name, "Stopped stats refresh for removed group");
                            false
                        }
                    });

                    // Spawn tasks for new groups
                    for name in current_names {
                        if let Entry::Vacant(entry) = tasks.entry(name.clone()) {
                            tracing::debug!(group = %name, "Starting stats refresh for group");
                            let handle = self.spawn_group_stats_refresh_task(name);
                            entry.insert(handle);
                        }
                    }
                }

                tokio::time::sleep(Duration::from_secs(GROUP_STATS_REFRESH_INTERVAL_SECS)).await;
            }
        });
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
                    tracing::Span::current()
                        .record("duration_ms", start.elapsed().as_millis() as u64);
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

            // Stale-while-revalidate: return cached data immediately,
            // trigger background refresh if debounce period has elapsed
            if self.should_check_incremental(group).await {
                // Spawn background task to check for new articles
                let self_clone = self.clone();
                let group_clone = group.to_string();
                let cache_key_clone = cache_key.clone();
                tokio::spawn(async move {
                    if let Ok(new_entries) = self_clone
                        .get_new_articles(&group_clone, cached.last_article_number)
                        .await
                    {
                        if !new_entries.is_empty() {
                            let new_hwm = new_entries
                                .iter()
                                .filter_map(|e| e.number())
                                .max()
                                .unwrap_or(cached.last_article_number);

                            // Re-fetch cached data to merge (it may have been updated)
                            if let Some(current) =
                                self_clone.threads_cache.get(&cache_key_clone).await
                            {
                                let merged =
                                    merge_articles_into_threads(&current.threads, new_entries);
                                self_clone
                                    .threads_cache
                                    .insert(
                                        cache_key_clone,
                                        CachedThreads {
                                            threads: merged,
                                            last_article_number: new_hwm,
                                        },
                                    )
                                    .await;
                            }

                            self_clone.update_group_hwm(&group_clone, new_hwm).await;
                        }
                    }
                });
            }

            // Mark group as active (non-blocking via spawn if needed)
            self.mark_group_active(group).await;

            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(cached.threads);
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

                    // Update shared HWM
                    self.update_group_hwm(group, last_article_number).await;

                    // Mark group as active
                    self.mark_group_active(group).await;

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
            .map(|e| Self::nntp_error_to_app_error(e, group))
            .unwrap_or_else(|| AppError::GroupNotFound(group.to_string())))
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
                let parsed = thread
                    .last_post_date
                    .as_ref()
                    .and_then(|d| DateTime::parse_from_rfc2822(d).ok());
                (i, parsed)
            })
            .collect();

        // Sort indices based on pre-parsed dates
        indexed_threads.sort_by(|(_, a_parsed), (_, b_parsed)| match (b_parsed, a_parsed) {
            (Some(b_dt), Some(a_dt)) => b_dt.cmp(a_dt),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
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
        if let Some(cached) = self.thread_cache.get(&cache_key).await {
            tracing::Span::current().record("cache_hit", true);

            // Stale-while-revalidate: return cached data immediately,
            // trigger background refresh if debounce period has elapsed
            if self.should_check_incremental(group).await {
                // Spawn background task to check for new articles
                let self_clone = self.clone();
                let group_clone = group.to_string();
                let cache_key_clone = cache_key.clone();
                let cached_thread = cached.thread.clone();
                tokio::spawn(async move {
                    // Get HWM for the group
                    let hwm = self_clone.get_group_hwm(&group_clone).await;
                    if hwm > 0 {
                        if let Ok(new_entries) =
                            self_clone.get_new_articles(&group_clone, hwm).await
                        {
                            if !new_entries.is_empty() {
                                // Merge new articles into this specific thread
                                let merged =
                                    merge_articles_into_thread(&cached_thread, new_entries);

                                // Update cache if thread was modified
                                if merged.article_count > cached_thread.article_count {
                                    self_clone
                                        .thread_cache
                                        .insert(
                                            cache_key_clone,
                                            CachedThread {
                                                thread: merged,
                                                group: group_clone.clone(),
                                            },
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                });
            }

            // Mark group as active (non-blocking)
            self.mark_group_active(group).await;

            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(cached.thread);
        }

        // Ensure threads_cache is populated for this group.
        // This blocks on first access but subsequent requests use cache,
        // and background refresh handles incremental updates.
        if self.threads_cache.get(group).await.is_none() {
            self.get_threads(group, 0).await?;
        }

        // Look up the thread from threads_cache
        let cached_threads = self
            .threads_cache
            .get(group)
            .await
            .ok_or_else(|| AppError::Internal("Failed to populate threads cache".into()))?;

        let thread = cached_threads
            .threads
            .iter()
            .find(|t| t.root_message_id == *message_id || t.root.contains_message_id(message_id))
            .cloned()
            .ok_or_else(|| {
                AppError::ArticleNotFound(format!("Thread not found: {}", message_id))
            })?;

        // Cache in thread_cache for direct future lookups
        self.thread_cache
            .insert(
                cache_key,
                CachedThread {
                    thread: thread.clone(),
                    group: group.to_string(),
                },
            )
            .await;

        // Mark group as active
        self.mark_group_active(group).await;

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        Ok(thread)
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
            thread
                .root
                .flatten_paginated(page, per_page, collapse_threshold);

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
        let page_ids_set: std::collections::HashSet<String> = page_msg_ids.into_iter().collect();
        let start = (page - 1) * per_page;
        let end = (start + per_page).min(comments.len());

        for (i, comment) in comments.iter_mut().enumerate() {
            if i >= start && i < end && page_ids_set.contains(&comment.message_id) {
                if let Some(fetched) = bodies.get(&comment.message_id) {
                    if let Some(ref mut article) = comment.article {
                        article.body = fetched.body.clone();
                        article.body_preview = fetched.body_preview.clone();
                        article.has_more_content = fetched.has_more_content;
                    }
                }
            }
        }

        Ok((thread, comments, pagination))
    }

    /// Check if we should refresh the groups list (debounced).
    /// Returns true if the debounce period has elapsed, and updates the timestamp.
    async fn should_refresh_groups(&self) -> bool {
        let now = Instant::now();
        // Use the same debounce period as incremental checks
        let debounce_duration = Duration::from_millis(INCREMENTAL_DEBOUNCE_MS);

        let mut last_refresh = self.last_groups_refresh.write().await;

        if let Some(last) = *last_refresh {
            if now.duration_since(last) < debounce_duration {
                return false;
            }
        }

        *last_refresh = Some(now);
        true
    }

    /// Fetch groups from all servers and update caches.
    /// This is the actual fetch logic, separated for reuse in background refresh.
    async fn fetch_groups_from_servers(&self) -> Result<Vec<GroupView>, AppError> {
        let cache_key = "groups".to_string();

        // Collect groups from all servers AND track server associations
        let mut all_groups: Vec<GroupView> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut group_to_servers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut posting_to_servers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut any_success = false;

        for (server_idx, service) in self.services.iter().enumerate() {
            match service.get_groups().await {
                Ok(groups) => {
                    any_success = true;
                    let server_allows_posting = service.is_posting_allowed();
                    let group_count = groups.len();

                    for group in groups {
                        // Track which servers carry this group
                        group_to_servers
                            .entry(group.name.clone())
                            .or_default()
                            .push(server_idx);

                        // Track which servers allow posting to this group
                        if server_allows_posting {
                            posting_to_servers
                                .entry(group.name.clone())
                                .or_default()
                                .push(server_idx);
                        }

                        // Add to all_groups if first time seeing this group
                        if seen_names.insert(group.name.clone()) {
                            all_groups.push(group);
                        }
                    }

                    tracing::debug!(
                        server = %service.name(),
                        posting_allowed = server_allows_posting,
                        group_count,
                        "Fetched groups from server"
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
            return Err(AppError::Internal(
                "Failed to fetch groups from any server".into(),
            ));
        }

        // Update group-server mapping atomically
        *self.group_servers.write().await = group_to_servers;

        // Update posting servers mapping
        let posting_server_count = posting_to_servers.len();
        *self.posting_servers.write().await = posting_to_servers;

        tracing::info!(
            total_groups = all_groups.len(),
            groups_with_posting = posting_server_count,
            "Group list updated"
        );

        // Sort by name
        all_groups.sort_by(|a, b| a.name.cmp(&b.name));

        // Cache the result
        self.groups_cache
            .insert(cache_key, all_groups.clone())
            .await;

        Ok(all_groups)
    }

    /// Fetch the list of available newsgroups
    /// Merges groups from all servers (union) and tracks which servers carry each group
    #[instrument(
        name = "nntp.federated.get_groups",
        skip(self),
        fields(cache_hit = false, coalesced = false, duration_ms)
    )]
    pub async fn get_groups(&self) -> Result<Vec<GroupView>, AppError> {
        let start = Instant::now();
        let cache_key = "groups".to_string();

        // Check cache first
        if let Some(groups) = self.groups_cache.get(&cache_key).await {
            tracing::Span::current().record("cache_hit", true);

            // Stale-while-revalidate: return cached data immediately,
            // trigger background refresh if debounce period has elapsed
            if self.should_refresh_groups().await {
                let self_clone = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = self_clone.fetch_groups_from_servers().await {
                        tracing::warn!(error = %e, "Background groups refresh failed");
                    }
                });
            }

            tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
            return Ok(groups);
        }

        // Cache miss - check for pending request (coalesce if one is already in flight)
        {
            let pending = self.pending_groups.read().await;
            if let Some(ref tx) = *pending {
                let mut rx = tx.subscribe();
                drop(pending); // Release lock while waiting

                tracing::Span::current().record("coalesced", true);
                return match rx.recv().await {
                    Ok(Ok(groups)) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Ok(groups)
                    }
                    Ok(Err(e)) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Err(AppError::Internal(e))
                    }
                    Err(_) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Err(AppError::Internal("Broadcast channel closed".into()))
                    }
                };
            }
        }

        // Register pending request
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        {
            let mut pending = self.pending_groups.write().await;
            // Double-check cache and pending after acquiring write lock
            if let Some(groups) = self.groups_cache.get(&cache_key).await {
                tracing::Span::current().record("cache_hit", true);
                tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
                return Ok(groups);
            }
            if let Some(ref existing_tx) = *pending {
                let mut rx = existing_tx.subscribe();
                drop(pending);
                tracing::Span::current().record("coalesced", true);
                return match rx.recv().await {
                    Ok(Ok(groups)) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Ok(groups)
                    }
                    Ok(Err(e)) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Err(AppError::Internal(e))
                    }
                    Err(_) => {
                        tracing::Span::current()
                            .record("duration_ms", start.elapsed().as_millis() as u64);
                        Err(AppError::Internal("Broadcast channel closed".into()))
                    }
                };
            }
            *pending = Some(tx.clone());
        }

        // Fetch from servers
        let result = self.fetch_groups_from_servers().await;

        // Broadcast result to waiters and cleanup
        {
            let mut pending = self.pending_groups.write().await;
            *pending = None;
        }

        match &result {
            Ok(groups) => {
                let _ = tx.send(Ok(groups.clone()));
            }
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
            }
        }

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
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
                    self.group_stats_cache
                        .insert(group.to_string(), stats.clone())
                        .await;
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

    /// Get cached group stats for multiple groups in parallel.
    /// Returns: (map of group name -> stats, list of uncached groups)
    pub async fn get_all_cached_group_stats(
        &self,
        group_names: &[String],
    ) -> (HashMap<String, Option<String>>, Vec<String>) {
        let futures: Vec<_> = group_names
            .iter()
            .map(|name| {
                let cache = &self.group_stats_cache;
                let name = name.clone();
                async move {
                    let stats = cache.get(&name).await;
                    (name, stats)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut group_stats: HashMap<String, Option<String>> = HashMap::new();
        let mut needs_prefetch: Vec<String> = Vec::new();

        for (name, stats) in results {
            if let Some(s) = stats {
                group_stats.insert(name, s.last_article_date);
            } else {
                needs_prefetch.push(name);
            }
        }

        (group_stats, needs_prefetch)
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

    /// Get cached thread counts for a list of group names in parallel.
    /// Returns a map of group name to thread count.
    pub async fn get_all_cached_thread_counts_for(
        &self,
        group_names: &[String],
    ) -> HashMap<String, usize> {
        let futures: Vec<_> = group_names
            .iter()
            .map(|name| {
                let cache = &self.threads_cache;
                let name = name.clone();
                async move {
                    let count = cache.get(&name).await.map(|c| c.threads.len());
                    (name, count)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        results
            .into_iter()
            .filter_map(|(name, count)| count.map(|c| (name, c)))
            .collect()
    }

    /// Check if posting is allowed for a group
    /// Returns true if at least one server carries this group
    /// (actual POST capability is checked at post time)
    pub async fn can_post_to_group(&self, group: &str) -> bool {
        // First check if we have explicit posting servers
        let posting = self.posting_servers.read().await;
        if posting.get(group).map(|v| !v.is_empty()).unwrap_or(false) {
            return true;
        }
        drop(posting);

        // Fall back to checking if any server carries this group
        let servers = self.group_servers.read().await;
        servers.get(group).map(|v| !v.is_empty()).unwrap_or(false)
    }

    /// Post a new article or reply
    /// Tries servers that support posting to the target group
    #[instrument(
        name = "nntp.federated.post_article",
        skip(self, headers, body),
        fields(duration_ms)
    )]
    pub async fn post_article(
        &self,
        group: &str,
        headers: Vec<(String, String)>,
        body: String,
    ) -> Result<(), AppError> {
        let start = Instant::now();

        // Get servers that support posting to this group
        let server_indices = {
            let servers = self.posting_servers.read().await;
            servers.get(group).cloned().unwrap_or_default()
        };

        // If no posting servers known, fall back to all servers for this group
        let server_indices = if server_indices.is_empty() {
            self.get_servers_for_group(group).await
        } else {
            server_indices
        };

        if server_indices.is_empty() {
            return Err(AppError::Internal(
                "No servers support posting to this group".into(),
            ));
        }

        // Try each server that supports posting
        let mut last_error = None;
        for idx in server_indices {
            let service = &self.services[idx];
            match service.post_article(headers.clone(), body.clone()).await {
                Ok(()) => {
                    tracing::info!(
                        group = %group,
                        server = %service.name(),
                        "Article posted successfully"
                    );
                    tracing::Span::current()
                        .record("duration_ms", start.elapsed().as_millis() as u64);
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        group = %group,
                        server = %service.name(),
                        error = %e,
                        "Failed to post article, trying next server"
                    );
                    last_error = Some(e);
                }
            }
        }

        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        Err(last_error
            .map(|e| AppError::Internal(format!("Failed to post article: {}", e.0)))
            .unwrap_or_else(|| AppError::Internal("Failed to post article".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ACTIVITY_HIGH_RPS, ACTIVITY_WINDOW_SECS, BACKGROUND_REFRESH_MAX_PERIOD_SECS,
        BACKGROUND_REFRESH_MIN_PERIOD_SECS,
    };

    // =============================================================================
    // calculate_refresh_period tests
    // =============================================================================

    #[test]
    fn test_calculate_refresh_period_high_activity() {
        // At 10,000 requests/second, should return ~1 second
        let period = NntpFederatedService::calculate_refresh_period(ACTIVITY_HIGH_RPS);
        assert!(
            period.as_secs_f64() <= BACKGROUND_REFRESH_MIN_PERIOD_SECS as f64 + 0.5,
            "High activity ({} rps) should give ~{}s refresh, got {:?}",
            ACTIVITY_HIGH_RPS,
            BACKGROUND_REFRESH_MIN_PERIOD_SECS,
            period
        );
    }

    #[test]
    fn test_calculate_refresh_period_low_activity() {
        // Minimal activity (1 request in 5 minutes = 1/300 rps) should return ~30 seconds
        let min_rps = 1.0 / ACTIVITY_WINDOW_SECS as f64;
        let period = NntpFederatedService::calculate_refresh_period(min_rps);
        assert!(
            period.as_secs_f64() >= BACKGROUND_REFRESH_MAX_PERIOD_SECS as f64 - 1.0,
            "Low activity ({:.4} rps) should give ~{}s refresh, got {:?}",
            min_rps,
            BACKGROUND_REFRESH_MAX_PERIOD_SECS,
            period
        );
    }

    #[test]
    fn test_calculate_refresh_period_log_scale() {
        // At 100 rps, should return value between 1s and 30s using log10 interpolation
        let period = NntpFederatedService::calculate_refresh_period(100.0);
        let period_secs = period.as_secs_f64();

        assert!(
            period_secs > BACKGROUND_REFRESH_MIN_PERIOD_SECS as f64,
            "100 rps should give period > {}s, got {}s",
            BACKGROUND_REFRESH_MIN_PERIOD_SECS,
            period_secs
        );
        assert!(
            period_secs < BACKGROUND_REFRESH_MAX_PERIOD_SECS as f64,
            "100 rps should give period < {}s, got {}s",
            BACKGROUND_REFRESH_MAX_PERIOD_SECS,
            period_secs
        );

        // Verify logarithmic behavior: 100 rps is closer to 10000 in log scale
        // log10(100) = 2, log10(10000) = 4, log10(1/300) ≈ -2.48
        // So 100 rps should be roughly in the middle-to-lower portion of the range
        assert!(
            period_secs < 20.0,
            "100 rps should be in lower half of range due to log scale, got {}s",
            period_secs
        );
    }

    #[test]
    fn test_calculate_refresh_period_zero_activity() {
        // Zero activity should return max period
        let period = NntpFederatedService::calculate_refresh_period(0.0);
        assert_eq!(
            period.as_secs(),
            BACKGROUND_REFRESH_MAX_PERIOD_SECS,
            "Zero activity should give max refresh period"
        );
    }

    #[test]
    fn test_calculate_refresh_period_negative_activity() {
        // Negative (invalid) activity should return max period
        let period = NntpFederatedService::calculate_refresh_period(-1.0);
        assert_eq!(
            period.as_secs(),
            BACKGROUND_REFRESH_MAX_PERIOD_SECS,
            "Negative activity should give max refresh period"
        );
    }

    // =============================================================================
    // GroupActivity tests
    // =============================================================================

    #[test]
    fn test_group_activity_advance_clears_buckets() {
        let mut activity = GroupActivity::new();

        // Record some requests at time 0
        activity.record_request(0);
        activity.record_request(0);
        activity.record_request(0);
        assert_eq!(activity.total_requests, 3);

        // Advance beyond the activity window (300+ seconds)
        // This should clear all old buckets and reset total
        activity.advance_to(ACTIVITY_WINDOW_SECS + 10);
        assert_eq!(
            activity.total_requests, 0,
            "Old buckets should be cleared after window elapses"
        );
    }

    #[test]
    fn test_group_activity_partial_advance() {
        let mut activity = GroupActivity::new();

        // Record requests at time 0
        activity.record_request(0);
        activity.record_request(0);

        // Advance by half the window
        let half_window = ACTIVITY_WINDOW_SECS / 2;
        activity.advance_to(half_window);

        // Record more requests
        activity.record_request(half_window);

        // Total should include both old and new
        assert_eq!(
            activity.total_requests, 3,
            "Requests within window should be counted"
        );

        // Advance past the first set of requests but not the second
        activity.advance_to(ACTIVITY_WINDOW_SECS + 5);

        // Only the newer request should remain
        assert_eq!(
            activity.total_requests, 1,
            "Only recent requests should remain after partial window advance"
        );
    }

    #[test]
    fn test_group_activity_requests_per_second() {
        let mut activity = GroupActivity::new();

        // Record 300 requests (should give 1 rps over 300s window)
        for i in 0..300 {
            activity.record_request(i);
        }

        let rps = activity.requests_per_second(300);
        assert!(
            (rps - 1.0).abs() < 0.01,
            "300 requests over 300s should give ~1 rps, got {}",
            rps
        );
    }

    #[test]
    fn test_group_activity_is_inactive() {
        let mut activity = GroupActivity::new();

        // Initially inactive (no requests)
        assert!(
            activity.is_inactive(0),
            "New activity tracker should be inactive"
        );

        // Record a request
        activity.record_request(0);
        assert!(
            !activity.is_inactive(0),
            "Should be active after recording request"
        );

        // Advance past the window
        assert!(
            activity.is_inactive(ACTIVITY_WINDOW_SECS + 10),
            "Should be inactive after window elapses"
        );
    }
}
