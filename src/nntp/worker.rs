//! NNTP worker that processes requests from the queue
//!
//! Each worker maintains its own NNTP connection and processes requests
//! from a shared async_channel queue.
//!
//! Connection strategy:
//! - Try TLS first for all connections
//! - If credentials are configured, TLS is required (no fallback)
//! - If no credentials, fall back to plain TCP if TLS fails

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use async_channel::Receiver;
use nntp_rs::net_client::NntpClient;
use nntp_rs::ListVariant;
use tokio::time::timeout;

use crate::config::{
    NntpServerConfig, NntpSettings, DEFAULT_SUBJECT, NNTP_MAX_ARTICLES_HEAD_FALLBACK,
    NNTP_MAX_ARTICLES_PER_REQUEST, NNTP_MAX_ARTICLES_SINGLE_THREAD, NNTP_RECONNECT_DELAY_SECS,
};

use super::messages::{GroupStatsView, NntpError, NntpRequest, NntpResponse};
use super::tls::NntpStream;
use super::{build_threads_from_hdr, build_threads_from_overview, parse_article, GroupView, HdrArticleData};

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
}

impl ServerCapabilities {
    /// Parse capabilities from the server response
    fn from_capabilities(caps: &[String]) -> Self {
        let mut list_variants = HashSet::new();
        let mut hdr_supported = false;
        let mut over_supported = false;

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
            }
        }

        Self {
            list_variants,
            hdr_supported,
            over_supported,
            references_in_overview: false, // Will be set after LIST OVERVIEW.FMT
            retrieved: true,
        }
    }

    /// Determine the best method for fetching thread data
    fn thread_fetch_method(&self) -> ThreadFetchMethod {
        if self.hdr_supported {
            ThreadFetchMethod::Hdr
        } else if self.over_supported && self.references_in_overview {
            ThreadFetchMethod::Over
        } else if self.over_supported {
            // OVER supported but References not in format - still try it
            // as most servers include References by default
            ThreadFetchMethod::Over
        } else {
            ThreadFetchMethod::Head
        }
    }

    /// Get LIST variants to try, ordered by preference
    /// If capabilities were retrieved, only returns advertised variants
    /// Otherwise returns all variants as fallback
    fn get_list_variants(&self) -> Vec<(&'static str, ListVariant)> {
        if self.retrieved && !self.list_variants.is_empty() {
            // Use only advertised variants, but still try them in preference order
            let mut variants = Vec::new();
            if self.list_variants.contains("ACTIVE") {
                variants.push(("LIST ACTIVE", ListVariant::Active(None)));
            }
            if self.list_variants.contains("NEWSGROUPS") {
                variants.push(("LIST NEWSGROUPS", ListVariant::Newsgroups(None)));
            }
            if self.list_variants.contains("BASIC") || self.list_variants.is_empty() {
                variants.push(("LIST", ListVariant::Basic(None)));
            }
            // If no recognized variants, fall back to trying all
            if variants.is_empty() {
                return Self::all_list_variants();
            }
            variants
        } else {
            // Capabilities not available, try all variants
            Self::all_list_variants()
        }
    }

    fn all_list_variants() -> Vec<(&'static str, ListVariant)> {
        vec![
            ("LIST ACTIVE", ListVariant::Active(None)),
            ("LIST NEWSGROUPS", ListVariant::Newsgroups(None)),
            ("LIST", ListVariant::Basic(None)),
        ]
    }
}

/// Worker that processes NNTP requests from a shared queue
pub struct NntpWorker {
    id: usize,
    server_name: String,
    server_config: NntpServerConfig,
    global_settings: NntpSettings,
    requests: Receiver<NntpRequest>,
}

impl NntpWorker {
    /// Create a new worker
    pub fn new(
        id: usize,
        server_config: NntpServerConfig,
        global_settings: NntpSettings,
        requests: Receiver<NntpRequest>,
    ) -> Self {
        Self {
            id,
            server_name: server_config.name.clone(),
            server_config,
            global_settings,
            requests,
        }
    }

    /// Run the worker loop - connects to NNTP and processes requests
    pub async fn run(self) {
        tracing::info!(worker = self.id, server = %self.server_name, "NNTP worker starting");

        loop {
            // Connect/reconnect to NNTP server
            let addr = format!("{}:{}", self.server_config.host, self.server_config.port);
            let connect_timeout = Duration::from_secs(
                self.server_config.timeout_seconds(&self.global_settings)
            );
            let has_credentials = self.server_config.has_credentials();

            // Set TLS requirement flag (credentials require TLS)
            super::tls::set_tls_required(has_credentials);

            // Connect using NntpClient with our TLS-aware NntpStream
            let mut client = match timeout(connect_timeout, NntpClient::<NntpStream>::connect(&addr)).await {
                Ok(Ok(client)) => {
                    let tls_status = if super::tls::last_connection_was_tls() { "TLS" } else { "plain TCP" };
                    tracing::info!(worker = self.id, server = %addr, tls = %tls_status, "Connected to NNTP server");
                    client
                }
                Ok(Err(e)) => {
                    tracing::error!(worker = self.id, error = %e, "Failed to connect to NNTP server");
                    tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                    continue;
                }
                Err(_) => {
                    tracing::error!(worker = self.id, "Connection timeout to NNTP server");
                    tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                    continue;
                }
            };

            // Authenticate if credentials are configured
            // (safe because credentials require TLS, which was enforced during connect)
            if has_credentials {
                let username = self.server_config.username.as_ref().unwrap();
                let password = self.server_config.password.as_ref().unwrap();

                match client.authenticate(username, password).await {
                    Ok(()) => {
                        tracing::info!(worker = self.id, "Authenticated successfully");
                    }
                    Err(e) => {
                        tracing::error!(worker = self.id, error = %e, "Authentication failed");
                        tokio::time::sleep(Duration::from_secs(NNTP_RECONNECT_DELAY_SECS)).await;
                        continue;
                    }
                }
            }

            // Query server capabilities to determine supported commands
            let mut capabilities = match client.capabilities().await {
                Ok(caps) => {
                    let server_caps = ServerCapabilities::from_capabilities(&caps);
                    tracing::debug!(
                        worker = self.id,
                        list_variants = ?server_caps.list_variants,
                        hdr_supported = server_caps.hdr_supported,
                        over_supported = server_caps.over_supported,
                        "Parsed server capabilities"
                    );
                    server_caps
                }
                Err(e) => {
                    tracing::debug!(
                        worker = self.id,
                        error = %e,
                        "Failed to get capabilities, will use fallback behavior"
                    );
                    ServerCapabilities::default()
                }
            };

            // If OVER is supported but not HDR, check if References is in overview format
            if capabilities.over_supported && !capabilities.hdr_supported {
                if capabilities.list_variants.contains("OVERVIEW.FMT") {
                    match client.list(ListVariant::OverviewFmt).await {
                        Ok(_) => {
                            // The list() method returns NewsgroupList which is empty for OverviewFmt
                            // We need to check the raw response. For now, assume References is present
                            // as it's part of the standard RFC 3977 overview format.
                            capabilities.references_in_overview = true;
                            tracing::debug!(
                                worker = self.id,
                                "OVERVIEW.FMT retrieved, assuming References field present"
                            );
                        }
                        Err(e) => {
                            tracing::debug!(
                                worker = self.id,
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

            tracing::info!(
                worker = self.id,
                method = ?capabilities.thread_fetch_method(),
                "Thread fetch method selected"
            );

            // Process requests until connection fails or channel closes
            loop {
                let request = match self.requests.recv().await {
                    Ok(req) => req,
                    Err(_) => {
                        tracing::info!(worker = self.id, "Request channel closed, worker shutting down");
                        return;
                    }
                };

                let result = self.handle_request(&mut client, &request, &capabilities).await;

                // Check if this was a connection error that requires reconnect
                let should_reconnect = result.is_err();

                // Send response
                request.respond(result);

                if should_reconnect {
                    tracing::warn!(worker = self.id, "Connection error, will reconnect");
                    break;
                }
            }
        }
    }

    /// Handle a single request
    async fn handle_request(
        &self,
        client: &mut NntpClient<NntpStream>,
        request: &NntpRequest,
        capabilities: &ServerCapabilities,
    ) -> Result<NntpResponse, NntpError> {
        match request {
            NntpRequest::GetGroups { .. } => {
                tracing::debug!(worker = self.id, "Fetching group list");

                // Get LIST variants to try based on server capabilities
                let list_variants = capabilities.get_list_variants();

                let mut last_error = None;
                for (variant_name, variant) in list_variants {
                    match client.list(variant).await {
                        Ok(groups) => {
                            tracing::debug!(
                                worker = self.id,
                                variant = variant_name,
                                count = groups.len(),
                                "Successfully fetched groups"
                            );
                            let group_views: Vec<GroupView> = groups
                                .iter()
                                .map(|g| GroupView {
                                    name: g.name.clone(),
                                    description: None,
                                    article_count: None,
                                })
                                .collect();
                            return Ok(NntpResponse::Groups(group_views));
                        }
                        Err(e) => {
                            tracing::debug!(
                                worker = self.id,
                                variant = variant_name,
                                error = %e,
                                "LIST variant not supported, trying next"
                            );
                            last_error = Some(e);
                        }
                    }
                }

                // All variants failed
                Err(NntpError(format!(
                    "Server does not support listing groups. Last error: {}",
                    last_error.map(|e| e.to_string()).unwrap_or_default()
                )))
            }

            NntpRequest::GetThreads { group, count, .. } => {
                let method = capabilities.thread_fetch_method();
                tracing::debug!(worker = self.id, %group, %count, ?method, "Fetching threads");

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
                                    worker = self.id,
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
                            .over(Some(range))
                            .await
                            .map_err(|e| NntpError(e.to_string()))?;
                        build_threads_from_overview(entries.to_vec())
                    }
                    ThreadFetchMethod::Head => {
                        // Fetch HEAD for each article (slowest fallback)
                        self.fetch_threads_via_head(client, start, stats.last).await?
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

            NntpRequest::GetThread { group, message_id, .. } => {
                let method = capabilities.thread_fetch_method();
                tracing::debug!(worker = self.id, %group, %message_id, ?method, "Fetching single thread");

                // Select group
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                // Fetch overview entries (larger range to find thread)
                // Use bounded range to avoid timeout with large groups
                let fetch_count = NNTP_MAX_ARTICLES_SINGLE_THREAD.min(stats.count);
                let start = stats.last.saturating_sub(fetch_count) + 1;
                let range = format!("{}-{}", start, stats.last);

                // Use the same method selection for single thread fetching
                let thread_views = match method {
                    ThreadFetchMethod::Hdr => {
                        // Fall back to OVER if HDR fails (e.g., due to non-UTF-8 data)
                        match self.fetch_threads_via_hdr(client, &range).await {
                            Ok(threads) => threads,
                            Err(e) => {
                                tracing::warn!(
                                    worker = self.id,
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
                        let entries = client
                            .over(Some(range))
                            .await
                            .map_err(|e| NntpError(e.to_string()))?;
                        build_threads_from_overview(entries.to_vec())
                    }
                    ThreadFetchMethod::Head => {
                        self.fetch_threads_via_head(client, start, stats.last).await?
                    }
                };

                // Find the thread containing the requested message_id
                // The message_id could be the root or any reply in the thread
                let thread = thread_views
                    .into_iter()
                    .find(|t| t.root_message_id == *message_id || t.root.contains_message_id(message_id))
                    .ok_or_else(|| NntpError(format!("Thread not found: {}", message_id)))?;

                Ok(NntpResponse::Thread(thread))
            }

            NntpRequest::GetArticle { message_id, .. } => {
                tracing::debug!(worker = self.id, %message_id, "Fetching article");
                let article = client
                    .article(nntp_rs::ArticleSpec::MessageId(message_id.clone()))
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                Ok(NntpResponse::Article(parse_article(&article)))
            }

            NntpRequest::GetGroupStats { group, .. } => {
                tracing::debug!(worker = self.id, %group, "Fetching group stats");

                // Select the group to get article range
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                // Get the date header for the last article
                let last_article_date = if stats.last > 0 {
                    // Use HDR command to get just the Date header for the last article
                    match client.hdr("Date".to_string(), Some(stats.last.to_string())).await {
                        Ok(headers) => {
                            headers.first().map(|h| h.value.clone())
                        }
                        Err(e) => {
                            tracing::debug!(
                                worker = self.id,
                                %group,
                                error = %e,
                                "HDR command failed, trying HEAD fallback"
                            );
                            // Fallback: fetch full headers with HEAD command
                            match client.head(nntp_rs::ArticleSpec::number_in_group(group, stats.last)).await {
                                Ok(headers_raw) => {
                                    // Parse Date header from raw headers
                                    let headers_str = String::from_utf8_lossy(&headers_raw);
                                    headers_str.lines()
                                        .find(|line| line.to_lowercase().starts_with("date:"))
                                        .map(|line| line[5..].trim().to_string())
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        worker = self.id,
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
                    article_count: stats.count,
                    last_article_date,
                    last_article_number: stats.last,
                }))
            }

            NntpRequest::GetNewArticles { group, since_article_number, .. } => {
                tracing::debug!(worker = self.id, %group, %since_article_number, "Fetching new articles");

                // Select the group to get current article range
                let stats = client
                    .group(group)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                if stats.last <= *since_article_number {
                    // No new articles
                    tracing::debug!(
                        worker = self.id,
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
                    worker = self.id,
                    %group,
                    %range,
                    "Fetching overview for range"
                );

                let entries = client
                    .over(Some(range))
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                tracing::debug!(
                    worker = self.id,
                    %group,
                    entry_count = entries.len(),
                    "Fetched new article overview entries"
                );

                Ok(NntpResponse::NewArticles(entries.to_vec()))
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
        tracing::debug!(worker = self.id, %range, "Fetching threads via HDR");

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

        tracing::debug!(
            worker = self.id,
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
            let from = froms_map
                .get(&entry.article)
                .cloned()
                .unwrap_or_default();
            let date = dates_map
                .get(&entry.article)
                .cloned()
                .unwrap_or_default();

            articles.push(HdrArticleData {
                message_id: entry.value.clone(),
                references,
                subject,
                from,
                date,
            });
        }

        tracing::debug!(
            worker = self.id,
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
        tracing::debug!(worker = self.id, start, end, "Fetching threads via HEAD (slow)");

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
            match client.head(nntp_rs::ArticleSpec::GroupNumber {
                group: String::new(),
                article_number: article_num,
            }).await {
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
                        worker = self.id,
                        article_num,
                        error = %e,
                        "Failed to fetch HEAD, skipping"
                    );
                }
            }
        }

        tracing::debug!(
            worker = self.id,
            article_count = articles.len(),
            "Built article data from HEAD responses"
        );

        Ok(build_threads_from_hdr(articles))
    }
}
