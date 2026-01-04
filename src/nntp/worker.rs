//! NNTP worker that processes requests from priority queues
//!
//! Each worker maintains its own NNTP connection and processes requests
//! from shared priority queues. High-priority requests (user-facing) are
//! processed before normal and low-priority requests. Aging prevents
//! starvation of low-priority requests under sustained high load.
//!
//! Connection strategy:
//! - Try TLS first for all connections
//! - If credentials are configured, TLS is required (no fallback)
//! - If no credentials, fall back to plain TCP if TLS fails

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::Receiver;
use nntp_rs::net_client::NntpClient;
use tokio::time::timeout;

use tracing::{instrument, Span};

use crate::config::{
    NntpServerConfig, NntpSettings, DEFAULT_SUBJECT, NNTP_MAX_ARTICLES_HEAD_FALLBACK,
    NNTP_MAX_ARTICLES_PER_REQUEST, NNTP_PRIORITY_AGING_SECS, NNTP_RECONNECT_DELAY_SECS,
};

use super::messages::{GroupStatsView, NntpError, NntpRequest, NntpResponse};
use super::tls::NntpStream;
use super::{
    build_threads_from_hdr, build_threads_from_overview, parse_article, GroupView, HdrArticleData,
};

/// Method to use for fetching thread data
#[derive(Debug, Clone, Copy, PartialEq)]
enum ThreadFetchMethod {
    /// Use HDR command for each needed header field
    Hdr,
    /// Use OVER/XOVER command (References field available in overview)
    Over,
    /// Use HEAD command for each article (slowest fallback)
    Head,
}

/// Server capabilities parsed from CAPABILITIES command
#[derive(Debug, Default)]
struct ServerCapabilities {
    /// Supported LIST variants (e.g., "ACTIVE", "NEWSGROUPS", "OVERVIEW.FMT")
    list_variants: HashSet<String>,
    /// Whether HDR command is supported
    hdr_supported: bool,
    /// Whether OVER command is supported
    over_supported: bool,
    /// Whether References field is in the overview format
    references_in_overview: bool,
    /// Whether capabilities were successfully retrieved
    retrieved: bool,
    /// Whether POST command is supported (from CAPABILITIES)
    post_supported: bool,
    /// Whether the greeting/MODE READER allows posting
    greeting_allows_post: bool,
}

impl ServerCapabilities {
    /// Parse capabilities from the server response
    fn from_capabilities(caps: &[String]) -> Self {
        let mut list_variants = HashSet::new();
        let mut hdr_supported = false;
        let mut over_supported = false;
        let mut post_supported = false;

        for cap in caps {
            let cap_upper = cap.to_uppercase();

            // LIST capability format: "LIST ACTIVE NEWSGROUPS OVERVIEW.FMT ..."
            if cap_upper.starts_with("LIST ") {
                for variant in cap[5..].split_whitespace() {
                    list_variants.insert(variant.to_uppercase());
                }
            } else if cap_upper == "LIST" {
                // Plain LIST support (rare)
                list_variants.insert("BASIC".to_string());
            } else if cap_upper == "HDR" || cap_upper.starts_with("HDR ") {
                hdr_supported = true;
            } else if cap_upper == "OVER" || cap_upper.starts_with("OVER ") {
                over_supported = true;
            } else if cap_upper == "POST" || cap_upper.starts_with("POST ") {
                post_supported = true;
            }
        }

        Self {
            list_variants,
            hdr_supported,
            over_supported,
            references_in_overview: false, // Will be set after LIST OVERVIEW.FMT
            retrieved: true,
            post_supported,
            greeting_allows_post: false, // Will be set from client.is_posting_allowed()
        }
    }

    /// Determine the best method for fetching thread data
    /// Prefers OVER (1 round-trip) over HDR (5 round-trips) for latency
    fn thread_fetch_method(&self) -> ThreadFetchMethod {
        if self.over_supported && self.references_in_overview {
            ThreadFetchMethod::Over
        } else if self.over_supported {
            // OVER supported but References not confirmed - still try it
            // as most servers include References by default
            ThreadFetchMethod::Over
        } else if self.hdr_supported {
            ThreadFetchMethod::Hdr
        } else {
            ThreadFetchMethod::Head
        }
    }

    /// Get LIST methods to try, ordered by preference
    /// If capabilities were retrieved, only returns advertised variants
    /// Otherwise returns all methods as fallback
    fn get_list_methods(&self) -> Vec<&'static str> {
        if self.retrieved && !self.list_variants.is_empty() {
            // Use only advertised variants, but still try them in preference order
            let mut methods = Vec::new();
            if self.list_variants.contains("ACTIVE") {
                methods.push("LIST ACTIVE");
            }
            if self.list_variants.contains("NEWSGROUPS") {
                methods.push("LIST NEWSGROUPS");
            }
            // If no recognized variants, fall back to trying all
            if methods.is_empty() {
                return Self::all_list_methods();
            }
            methods
        } else {
            // Capabilities not available, try all methods
            Self::all_list_methods()
        }
    }

    fn all_list_methods() -> Vec<&'static str> {
        vec!["LIST ACTIVE", "LIST NEWSGROUPS"]
    }

    /// Check if posting is supported
    /// Requires both the greeting/MODE READER to allow posting AND POST in CAPABILITIES
    fn can_post(&self) -> bool {
        self.greeting_allows_post && self.post_supported
    }
}

/// Priority queue receivers for the worker.
///
/// Groups the three priority-level queue receivers that workers pull requests from.
pub struct WorkerQueues {
    /// High-priority request queue (user-facing: GetArticle, PostArticle)
    pub high: Receiver<NntpRequest>,
    /// Normal-priority request queue (page load: GetThreads, GetGroups)
    pub normal: Receiver<NntpRequest>,
    /// Low-priority request queue (background: GetGroupStats, GetNewArticles)
    pub low: Receiver<NntpRequest>,
}

/// Shared counters for tracking worker pool status.
///
/// These atomic counters are shared across all workers in a service to track
/// aggregate connection and posting capability status.
pub struct WorkerCounters {
    /// Count of workers with active connections
    pub connected: Arc<AtomicUsize>,
    /// Count of workers whose connections allow posting
    pub posting: Arc<AtomicUsize>,
}

/// Worker that processes NNTP requests from priority queues
pub struct NntpWorker {
    id: usize,
    server_name: String,
    server_config: NntpServerConfig,
    global_settings: NntpSettings,
    /// Priority queue receivers
    queues: WorkerQueues,
    /// Shared worker pool counters
    counters: WorkerCounters,
}

impl NntpWorker {
    /// Create a new worker with priority queue receivers and shared counters
    pub fn new(
        id: usize,
        server_config: NntpServerConfig,
        global_settings: NntpSettings,
        queues: WorkerQueues,
        counters: WorkerCounters,
    ) -> Self {
        Self {
            id,
            server_name: server_config.name.clone(),
            server_config,
            global_settings,
            queues,
            counters,
        }
    }

    /// Receive the next request, respecting priority with aging to prevent starvation.
    ///
    /// Priority order: High > Normal > Low
    /// Aging: If low-priority requests have been waiting longer than NNTP_PRIORITY_AGING_SECS,
    /// process one low-priority request to prevent indefinite starvation.
    #[allow(clippy::never_loop)] // Loop is intentional for tokio::select! pattern
    async fn recv_prioritized(
        &self,
        last_low_process: &mut Instant,
    ) -> Result<NntpRequest, async_channel::RecvError> {
        loop {
            // Check for aging: if low-priority queue is non-empty and hasn't been
            // serviced recently, process one low-priority request
            let should_check_aging =
                last_low_process.elapsed().as_secs() >= NNTP_PRIORITY_AGING_SECS;

            if should_check_aging {
                if let Ok(req) = self.queues.low.try_recv() {
                    *last_low_process = Instant::now();
                    tracing::trace!(
                        priority = "low",
                        reason = "aging",
                        "Processing aged low-priority request"
                    );
                    return Ok(req);
                }
            }

            // Try high priority (non-blocking)
            if let Ok(req) = self.queues.high.try_recv() {
                return Ok(req);
            }

            // Try normal priority (non-blocking)
            if let Ok(req) = self.queues.normal.try_recv() {
                return Ok(req);
            }

            // Try low priority (non-blocking)
            if let Ok(req) = self.queues.low.try_recv() {
                *last_low_process = Instant::now();
                return Ok(req);
            }

            // All queues empty - wait for any request using biased select
            // to maintain priority order when multiple arrive simultaneously
            tokio::select! {
                biased;

                result = self.queues.high.recv() => return result,
                result = self.queues.normal.recv() => return result,
                result = self.queues.low.recv() => {
                    *last_low_process = Instant::now();
                    return result;
                }
            }
        }
    }

    /// Run the worker loop - connects to NNTP and processes requests
    #[instrument(
        name = "nntp.worker",
        skip(self),
        fields(worker_id = self.id, server = %self.server_name)
    )]
    pub async fn run(self) {
        tracing::info!("Worker starting");

        loop {
            // Connect/reconnect to NNTP server
            let addr = format!("{}:{}", self.server_config.host, self.server_config.port);
            let connect_timeout =
                Duration::from_secs(self.server_config.timeout_seconds(&self.global_settings));
            let has_credentials = self.server_config.has_credentials();
            let requires_tls = self.server_config.requires_tls_for_credentials();

            // Set TLS requirement flag (credentials require TLS unless allow_insecure_auth is set)
            super::tls::set_tls_required(requires_tls);

            // Connect using NntpClient with our TLS-aware NntpStream
            let mut client =
                match timeout(connect_timeout, NntpClient::<NntpStream>::connect(&addr)).await {
                    Ok(Ok(client)) => {
                        let tls_status = if super::tls::last_connection_was_tls() {
                            "TLS"
                        } else {
                            "plain TCP"
                        };
                        tracing::info!(tls = %tls_status, "Connected to NNTP server");
                        client
                    }
                    Ok(Err(e)) => {
                        tracing::error!(error = %e, "Failed to connect");
                        tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                        continue;
                    }
                    Err(_) => {
                        tracing::error!("Connection timeout");
                        tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                        continue;
                    }
                };

            // Authenticate if credentials are configured
            // Note: TLS is enforced during connect unless allow_insecure_auth is set
            if has_credentials {
                if !requires_tls {
                    tracing::warn!(
                        "Authenticating over plaintext connection (allow_insecure_auth is set)"
                    );
                }
                let username = self.server_config.username.as_ref().unwrap();
                let password = self.server_config.password.as_ref().unwrap();

                match client.authenticate(username, password).await {
                    Ok(()) => {
                        tracing::info!("Authenticated successfully");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Authentication failed");
                        tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                        continue;
                    }
                }
            }

            // Switch to reader mode (RFC 3977 Section 5.3)
            // MODE READER may update posting capability based on authentication state
            match client.mode_reader().await {
                Ok(_status) => {
                    tracing::debug!("MODE READER completed");
                }
                Err(e) => {
                    // MODE READER is required per RFC 3977; failure is fatal for this connection
                    tracing::error!(error = %e, "MODE READER failed");
                    tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                    continue;
                }
            }

            // Query server capabilities to determine supported commands
            let mut capabilities = match client.capabilities().await {
                Ok(caps) => {
                    let server_caps = ServerCapabilities::from_capabilities(&caps);
                    tracing::trace!(
                        list_variants = ?server_caps.list_variants,
                        hdr_supported = server_caps.hdr_supported,
                        over_supported = server_caps.over_supported,
                        "Parsed server capabilities"
                    );
                    server_caps
                }
                Err(e) => {
                    tracing::trace!(
                        error = %e,
                        "Failed to get capabilities, will use fallback behavior"
                    );
                    ServerCapabilities::default()
                }
            };

            // If OVER is supported, check if References is in overview format
            // We need this even if HDR is supported since we prefer OVER for latency
            if capabilities.over_supported {
                if capabilities.list_variants.contains("OVERVIEW.FMT") {
                    match client.list_overview_fmt().await {
                        Ok(format) => {
                            // Check if References is in the overview format
                            // Format fields are like "Subject:", "From:", "References:", etc.
                            capabilities.references_in_overview = format
                                .iter()
                                .any(|field| field.eq_ignore_ascii_case("References:"));
                            tracing::trace!(
                                fields = ?format.iter().collect::<Vec<_>>(),
                                references_found = capabilities.references_in_overview,
                                "OVERVIEW.FMT retrieved"
                            );
                        }
                        Err(e) => {
                            tracing::trace!(
                                error = %e,
                                "Failed to get OVERVIEW.FMT, assuming standard format"
                            );
                            // Standard RFC 3977 format includes References
                            capabilities.references_in_overview = true;
                        }
                    }
                } else {
                    // No OVERVIEW.FMT in capabilities, assume standard format
                    capabilities.references_in_overview = true;
                }
            }

            // Set greeting_allows_post from the client's tracking of greeting/MODE READER response
            capabilities.greeting_allows_post = client.is_posting_allowed();

            // Increment connection counters now that setup is complete
            self.counters.connected.fetch_add(1, Ordering::Relaxed);
            let can_post = capabilities.can_post();
            if can_post {
                self.counters.posting.fetch_add(1, Ordering::Relaxed);
            }

            tracing::info!(
                method = ?capabilities.thread_fetch_method(),
                can_post = can_post,
                "Worker ready"
            );

            // Track when we last processed a low-priority request (for aging)
            let mut last_low_process = Instant::now();

            // Process requests until connection fails or channel closes
            loop {
                let request = match self.recv_prioritized(&mut last_low_process).await {
                    Ok(req) => req,
                    Err(_) => {
                        // Decrement counters before shutting down
                        self.counters.connected.fetch_sub(1, Ordering::Relaxed);
                        if can_post {
                            self.counters.posting.fetch_sub(1, Ordering::Relaxed);
                        }
                        tracing::info!("Request channels closed, worker shutting down");
                        return;
                    }
                };

                // Log queue depths at trace level for monitoring
                tracing::trace!(
                    high_depth = self.queues.high.len(),
                    normal_depth = self.queues.normal.len(),
                    low_depth = self.queues.low.len(),
                    priority = %request.priority(),
                    "Processing request"
                );

                let result = self
                    .handle_request(&mut client, &request, &capabilities)
                    .await;

                // Check if this was a connection error that requires reconnect
                let should_reconnect = result.is_err();

                // Send response
                request.respond(result);

                if should_reconnect {
                    // Decrement counters before reconnecting
                    self.counters.connected.fetch_sub(1, Ordering::Relaxed);
                    if can_post {
                        self.counters.posting.fetch_sub(1, Ordering::Relaxed);
                    }
                    tracing::warn!("Connection error, will reconnect");
                    break;
                }
            }
        }
    }

    /// Handle a single request
    #[instrument(
        name = "nntp.worker.handle_request",
        skip(self, client, request, capabilities),
        fields(operation, duration_ms)
    )]
    async fn handle_request(
        &self,
        client: &mut NntpClient<NntpStream>,
        request: &NntpRequest,
        capabilities: &ServerCapabilities,
    ) -> Result<NntpResponse, NntpError> {
        let start = Instant::now();
        let result = self
            .handle_request_inner(client, request, capabilities)
            .await;
        tracing::Span::current().record("duration_ms", start.elapsed().as_millis() as u64);
        result
    }

    /// Inner request handling logic
    async fn handle_request_inner(
        &self,
        client: &mut NntpClient<NntpStream>,
        request: &NntpRequest,
        capabilities: &ServerCapabilities,
    ) -> Result<NntpResponse, NntpError> {
        match request {
            NntpRequest::GetGroups { .. } => {
                Span::current().record("operation", "get_groups");
                tracing::debug!("Fetching group list");

                // Get LIST methods to try based on server capabilities
                let list_methods = capabilities.get_list_methods();

                let mut last_error: Option<String> = None;
                for method_name in list_methods {
                    let result = match method_name {
                        "LIST ACTIVE" => client
                            .list_active(None)
                            .await
                            .map(|groups| {
                                groups
                                    .iter()
                                    .map(|g| GroupView {
                                        name: g.name.clone(),
                                        description: None,
                                        article_count: None,
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .map_err(|e| e.to_string()),
                        "LIST NEWSGROUPS" => client
                            .list_newsgroups(None)
                            .await
                            .map(|groups| {
                                groups
                                    .iter()
                                    .map(|g| GroupView {
                                        name: g.name.clone(),
                                        description: Some(g.description.clone()),
                                        article_count: None,
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .map_err(|e| e.to_string()),
                        _ => continue,
                    };

                    match result {
                        Ok(group_views) => {
                            tracing::debug!(
                                variant = method_name,
                                count = group_views.len(),
                                "Successfully fetched groups"
                            );
                            return Ok(NntpResponse::Groups(group_views));
                        }
                        Err(e) => {
                            tracing::debug!(
                                variant = method_name,
                                error = %e,
                                "LIST method not supported, trying next"
                            );
                            last_error = Some(e);
                        }
                    }
                }

                // All methods failed
                Err(NntpError(format!(
                    "Server does not support listing groups. Last error: {}",
                    last_error.unwrap_or_default()
                )))
            }

            NntpRequest::GetThreads { group, count, .. } => {
                Span::current().record("operation", "get_threads");
                let method = capabilities.thread_fetch_method();
                tracing::debug!(%group, %count, ?method, "Fetching threads");

                // Select group first
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                // Calculate range for recent articles
                // Use bounded range to avoid timeout with large groups
                let fetch_count = (*count).min(stats.count).min(NNTP_MAX_ARTICLES_PER_REQUEST);
                let start = stats.last.saturating_sub(fetch_count) + 1;
                let range = format!("{}-{}", start, stats.last);

                let mut thread_views = match method {
                    ThreadFetchMethod::Hdr => {
                        // Fetch each header field separately using HDR command
                        // Fall back to OVER if HDR fails (e.g., due to non-UTF-8 data)
                        match self.fetch_threads_via_hdr(client, &range).await {
                            Ok(threads) => threads,
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "HDR fetch failed, falling back to OVER"
                                );
                                let entries = client
                                    .over(Some(range))
                                    .await
                                    .map_err(|e| NntpError(e.to_string()))?;
                                build_threads_from_overview(entries.to_vec())
                            }
                        }
                    }
                    ThreadFetchMethod::Over => {
                        // Fetch overview entries via OVER/XOVER
                        let entries = client
                            .over(Some(range.clone()))
                            .await
                            .map_err(|e| NntpError(e.to_string()))?;
                        build_threads_from_overview(entries.to_vec())
                    }
                    ThreadFetchMethod::Head => {
                        // Fetch HEAD for each article (slowest fallback)
                        self.fetch_threads_via_head(client, start, stats.last)
                            .await?
                    }
                };

                // Sort by last post date (newest first)
                thread_views.sort_by(|a, b| {
                    use chrono::DateTime;
                    match (&b.last_post_date, &a.last_post_date) {
                        (Some(b_d), Some(a_d)) => {
                            let bp = DateTime::parse_from_rfc2822(b_d);
                            let ap = DateTime::parse_from_rfc2822(a_d);
                            match (bp, ap) {
                                (Ok(b), Ok(a)) => b.cmp(&a),
                                _ => std::cmp::Ordering::Equal,
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                });

                Ok(NntpResponse::Threads(thread_views))
            }

            NntpRequest::GetArticle { message_id, .. } => {
                Span::current().record("operation", "get_article");
                tracing::debug!(%message_id, "Fetching article");
                let article = client
                    .article(nntp_rs::ArticleSpec::MessageId(message_id.clone()))
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                Ok(NntpResponse::Article(parse_article(&article)))
            }

            NntpRequest::GetGroupStats { group, .. } => {
                Span::current().record("operation", "get_group_stats");
                tracing::debug!(%group, "Fetching group stats");

                // Select the group to get article range
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                // Get the date header for the last article
                let last_article_date = if stats.last > 0 {
                    // Use HDR command to get just the Date header for the last article
                    match client
                        .hdr("Date".to_string(), Some(stats.last.to_string()))
                        .await
                    {
                        Ok(headers) => headers.first().map(|h| h.value.clone()),
                        Err(e) => {
                            tracing::debug!(
                                %group,
                                error = %e,
                                "HDR command failed, trying HEAD fallback"
                            );
                            // Fallback: fetch full headers with HEAD command
                            match client
                                .head(nntp_rs::ArticleSpec::number_in_group(group, stats.last))
                                .await
                            {
                                Ok(headers_raw) => {
                                    // Parse Date header from raw headers
                                    let headers_str = String::from_utf8_lossy(&headers_raw);
                                    headers_str
                                        .lines()
                                        .find(|line| line.to_lowercase().starts_with("date:"))
                                        .map(|line| line[5..].trim().to_string())
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        %group,
                                        error = %e,
                                        "Failed to get last article date"
                                    );
                                    None
                                }
                            }
                        }
                    }
                } else {
                    None
                };

                Ok(NntpResponse::GroupStats(GroupStatsView {
                    last_article_date,
                    last_article_number: stats.last,
                }))
            }

            NntpRequest::GetNewArticles {
                group,
                since_article_number,
                ..
            } => {
                Span::current().record("operation", "get_new_articles");
                tracing::debug!(%group, %since_article_number, "Fetching new articles");

                // Select the group to get current article range
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                if stats.last <= *since_article_number {
                    // No new articles
                    tracing::debug!(
                        %group,
                        last = stats.last,
                        since = *since_article_number,
                        "No new articles"
                    );
                    return Ok(NntpResponse::NewArticles(vec![]));
                }

                // Fetch only new articles using OVER command with range
                let range = format!("{}-", *since_article_number + 1);
                tracing::debug!(
                    %group,
                    %range,
                    "Fetching overview for range"
                );

                let entries = client
                    .over(Some(range))
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                tracing::debug!(
                    %group,
                    entry_count = entries.len(),
                    "Fetched new article overview entries"
                );

                Ok(NntpResponse::NewArticles(entries.to_vec()))
            }

            NntpRequest::PostArticle { headers, body, .. } => {
                Span::current().record("operation", "post_article");
                tracing::debug!("Posting article");

                // Format the article for POST command
                // Headers is a Vec<(String, String)> of header name/value pairs
                // Body is the article body text

                let mut article_lines: Vec<String> = Vec::new();

                // Add headers
                for (name, value) in headers {
                    article_lines.push(format!("{}: {}", name, value));
                }

                // Blank line between headers and body
                article_lines.push(String::new());

                // Add body lines
                for line in body.lines() {
                    // Dot-stuffing: lines starting with "." get an extra "." prepended
                    if line.starts_with('.') {
                        article_lines.push(format!(".{}", line));
                    } else {
                        article_lines.push(line.to_string());
                    }
                }

                // Join all lines with CRLF for the nntp_rs client's post method
                let article_content = article_lines.join("\r\n");

                // Use the nntp_rs client's post method
                client
                    .post(article_content)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                Ok(NntpResponse::PostResult)
            }

            NntpRequest::CheckArticleExists { message_id, .. } => {
                Span::current().record("operation", "check_article_exists");
                tracing::debug!(%message_id, "Checking article existence with STAT");

                match client
                    .stat(nntp_rs::ArticleSpec::MessageId(message_id.to_string()))
                    .await
                {
                    Ok(_) => Ok(NntpResponse::ArticleExists(true)),
                    Err(e) => {
                        // Check if this is a "not found" error (430 or 423)
                        let err_str = e.to_string();
                        if err_str.contains("430")
                            || err_str.contains("423")
                            || err_str.to_lowercase().contains("no such article")
                        {
                            Ok(NntpResponse::ArticleExists(false))
                        } else {
                            Err(NntpError(err_str))
                        }
                    }
                }
            }
        }
    }

    /// Fetch threads using HDR commands for each required header field.
    /// This is more efficient than OVER for large ranges as each response is smaller.
    async fn fetch_threads_via_hdr(
        &self,
        client: &mut NntpClient<NntpStream>,
        range: &str,
    ) -> Result<Vec<super::ThreadView>, NntpError> {
        tracing::debug!(%range, "Fetching threads via HDR");

        // Fetch each required header field
        let message_ids = client
            .hdr("Message-ID".to_string(), Some(range.to_string()))
            .await
            .map_err(|e| NntpError(format!("HDR Message-ID failed: {}", e)))?;

        let references = client
            .hdr("References".to_string(), Some(range.to_string()))
            .await
            .map_err(|e| NntpError(format!("HDR References failed: {}", e)))?;

        let subjects = client
            .hdr("Subject".to_string(), Some(range.to_string()))
            .await
            .map_err(|e| NntpError(format!("HDR Subject failed: {}", e)))?;

        let froms = client
            .hdr("From".to_string(), Some(range.to_string()))
            .await
            .map_err(|e| NntpError(format!("HDR From failed: {}", e)))?;

        let dates = client
            .hdr("Date".to_string(), Some(range.to_string()))
            .await
            .map_err(|e| NntpError(format!("HDR Date failed: {}", e)))?;

        tracing::trace!(
            message_id_count = message_ids.len(),
            references_count = references.len(),
            subjects_count = subjects.len(),
            froms_count = froms.len(),
            dates_count = dates.len(),
            "HDR responses received"
        );

        // Build lookup maps by article number
        let mut refs_map: HashMap<String, String> = HashMap::new();
        for entry in references.iter() {
            refs_map.insert(entry.article.clone(), entry.value.clone());
        }

        let mut subjects_map: HashMap<String, String> = HashMap::new();
        for entry in subjects.iter() {
            subjects_map.insert(entry.article.clone(), entry.value.clone());
        }

        let mut froms_map: HashMap<String, String> = HashMap::new();
        for entry in froms.iter() {
            froms_map.insert(entry.article.clone(), entry.value.clone());
        }

        let mut dates_map: HashMap<String, String> = HashMap::new();
        for entry in dates.iter() {
            dates_map.insert(entry.article.clone(), entry.value.clone());
        }

        // Combine into HdrArticleData
        let mut articles: Vec<HdrArticleData> = Vec::new();
        for entry in message_ids.iter() {
            let article_num: u64 = entry.article.parse().unwrap_or(0);
            if article_num == 0 {
                continue;
            }

            let references = refs_map.get(&entry.article).cloned();
            let subject = subjects_map
                .get(&entry.article)
                .cloned()
                .unwrap_or_else(|| DEFAULT_SUBJECT.to_string());
            let from = froms_map.get(&entry.article).cloned().unwrap_or_default();
            let date = dates_map.get(&entry.article).cloned().unwrap_or_default();

            articles.push(HdrArticleData {
                message_id: entry.value.clone(),
                references,
                subject,
                from,
                date,
            });
        }

        tracing::trace!(
            article_count = articles.len(),
            "Built article data from HDR responses"
        );

        Ok(build_threads_from_hdr(articles))
    }

    /// Fetch threads using HEAD command for each article (slowest fallback).
    /// Used when neither HDR nor OVER with References is available.
    async fn fetch_threads_via_head(
        &self,
        client: &mut NntpClient<NntpStream>,
        start: u64,
        end: u64,
    ) -> Result<Vec<super::ThreadView>, NntpError> {
        tracing::debug!(start, end, "Fetching threads via HEAD (slow)");

        let mut articles: Vec<HdrArticleData> = Vec::new();

        // Limit the number of HEAD requests to avoid excessive time
        let max_articles = NNTP_MAX_ARTICLES_HEAD_FALLBACK;
        let actual_start = if end - start > max_articles {
            end - max_articles + 1
        } else {
            start
        };

        // We need the group name for ArticleSpec, but since we already selected the group,
        // we can use Current after advancing. However, the simpler approach is to
        // use GroupNumber with an empty group since we've already selected the group.
        // Actually, we'll use the raw number approach via GroupNumber
        for article_num in actual_start..=end {
            // Use GroupNumber with empty string - the number is what matters on the wire
            match client
                .head(nntp_rs::ArticleSpec::GroupNumber {
                    group: String::new(),
                    article_number: article_num,
                })
                .await
            {
                Ok(headers_raw) => {
                    let headers_str = String::from_utf8_lossy(&headers_raw);

                    // Parse headers
                    let mut message_id = String::new();
                    let mut references = None;
                    let mut subject = DEFAULT_SUBJECT.to_string();
                    let mut from = String::new();
                    let mut date = String::new();

                    for line in headers_str.lines() {
                        let line_lower = line.to_lowercase();
                        if line_lower.starts_with("message-id:") {
                            message_id = line[11..].trim().to_string();
                        } else if line_lower.starts_with("references:") {
                            references = Some(line[11..].trim().to_string());
                        } else if line_lower.starts_with("subject:") {
                            subject = line[8..].trim().to_string();
                        } else if line_lower.starts_with("from:") {
                            from = line[5..].trim().to_string();
                        } else if line_lower.starts_with("date:") {
                            date = line[5..].trim().to_string();
                        }
                    }

                    if !message_id.is_empty() {
                        articles.push(HdrArticleData {
                            message_id,
                            references,
                            subject,
                            from,
                            date,
                        });
                    }
                }
                Err(e) => {
                    // Article might be deleted, skip it
                    tracing::trace!(
                        article_num,
                        error = %e,
                        "Failed to fetch HEAD, skipping"
                    );
                }
            }
        }

        tracing::trace!(
            article_count = articles.len(),
            "Built article data from HEAD responses"
        );

        Ok(build_threads_from_hdr(articles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =============================================================================
    // ServerCapabilities tests
    // =============================================================================

    #[test]
    fn test_thread_fetch_method_prefers_over() {
        let caps = ServerCapabilities {
            over_supported: true,
            hdr_supported: true,
            references_in_overview: true,
            ..Default::default()
        };
        assert_eq!(caps.thread_fetch_method(), ThreadFetchMethod::Over);
    }

    #[test]
    fn test_thread_fetch_method_over_without_references() {
        // Even without confirmed References, OVER is still preferred
        let caps = ServerCapabilities {
            over_supported: true,
            hdr_supported: true,
            references_in_overview: false,
            ..Default::default()
        };
        assert_eq!(caps.thread_fetch_method(), ThreadFetchMethod::Over);
    }

    #[test]
    fn test_thread_fetch_method_hdr_fallback() {
        // When OVER is not available, fall back to HDR
        let caps = ServerCapabilities {
            over_supported: false,
            hdr_supported: true,
            ..Default::default()
        };
        assert_eq!(caps.thread_fetch_method(), ThreadFetchMethod::Hdr);
    }

    #[test]
    fn test_thread_fetch_method_head_fallback() {
        // When neither OVER nor HDR is available, fall back to HEAD
        let caps = ServerCapabilities {
            over_supported: false,
            hdr_supported: false,
            ..Default::default()
        };
        assert_eq!(caps.thread_fetch_method(), ThreadFetchMethod::Head);
    }

    #[test]
    fn test_server_capabilities_from_capabilities_parses_hdr() {
        let caps = ServerCapabilities::from_capabilities(&["HDR".to_string()]);
        assert!(caps.hdr_supported);
        assert!(!caps.over_supported);
    }

    #[test]
    fn test_server_capabilities_from_capabilities_parses_over() {
        let caps = ServerCapabilities::from_capabilities(&["OVER".to_string()]);
        assert!(caps.over_supported);
        assert!(!caps.hdr_supported);
    }

    #[test]
    fn test_server_capabilities_from_capabilities_parses_post() {
        let caps = ServerCapabilities::from_capabilities(&["POST".to_string()]);
        assert!(caps.post_supported);
    }

    #[test]
    fn test_server_capabilities_from_capabilities_parses_list_variants() {
        let caps =
            ServerCapabilities::from_capabilities(&["LIST ACTIVE NEWSGROUPS OVERVIEW.FMT".to_string()]);
        assert!(caps.list_variants.contains("ACTIVE"));
        assert!(caps.list_variants.contains("NEWSGROUPS"));
        assert!(caps.list_variants.contains("OVERVIEW.FMT"));
    }

    #[test]
    fn test_server_capabilities_can_post_requires_both() {
        let mut caps = ServerCapabilities::default();

        // Neither set
        assert!(!caps.can_post());

        // Only greeting allows
        caps.greeting_allows_post = true;
        assert!(!caps.can_post());

        // Only POST in capabilities
        caps.greeting_allows_post = false;
        caps.post_supported = true;
        assert!(!caps.can_post());

        // Both set
        caps.greeting_allows_post = true;
        assert!(caps.can_post());
    }

    // =============================================================================
    // Priority aging constant test
    // =============================================================================

    #[test]
    fn test_priority_aging_threshold_is_10_seconds() {
        // Verify the aging threshold constant is 10 seconds as documented
        assert_eq!(NNTP_PRIORITY_AGING_SECS, 10);
    }
}
