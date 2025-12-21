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
use nntp_rs::threading::NntpClientThreadingExt;
use nntp_rs::ListVariant;
use tokio::time::timeout;

use crate::config::{NntpServerConfig, NntpSettings};

use super::messages::{GroupStatsView, NntpError, NntpRequest, NntpResponse};
use super::tls::NntpStream;
use super::{threads_to_views, ArticleView, GroupView};

/// Server capabilities parsed from CAPABILITIES command
#[derive(Debug, Default)]
struct ServerCapabilities {
    /// Supported LIST variants (e.g., "ACTIVE", "NEWSGROUPS", "OVERVIEW.FMT")
    list_variants: HashSet<String>,
    /// Whether capabilities were successfully retrieved
    retrieved: bool,
}

impl ServerCapabilities {
    /// Parse capabilities from the server response
    fn from_capabilities(caps: &[String]) -> Self {
        let mut list_variants = HashSet::new();

        for cap in caps {
            // LIST capability format: "LIST ACTIVE NEWSGROUPS OVERVIEW.FMT ..."
            if cap.starts_with("LIST ") {
                for variant in cap[5..].split_whitespace() {
                    list_variants.insert(variant.to_uppercase());
                }
            } else if cap == "LIST" {
                // Plain LIST support (rare)
                list_variants.insert("BASIC".to_string());
            }
        }

        Self {
            list_variants,
            retrieved: true,
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
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                Err(_) => {
                    tracing::error!(worker = self.id, "Connection timeout to NNTP server");
                    tokio::time::sleep(Duration::from_secs(5)).await;
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
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                }
            }

            // Query server capabilities to determine supported commands
            let capabilities = match client.capabilities().await {
                Ok(caps) => {
                    let server_caps = ServerCapabilities::from_capabilities(&caps);
                    tracing::debug!(
                        worker = self.id,
                        list_variants = ?server_caps.list_variants,
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
                tracing::debug!(worker = self.id, %group, %count, "Fetching threads");
                let collection = client
                    .recent_threads(group, *count)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                let thread_views = threads_to_views(&collection);
                Ok(NntpResponse::Threads(thread_views))
            }

            NntpRequest::GetThread { group, message_id, .. } => {
                tracing::debug!(worker = self.id, %group, %message_id, "Fetching single thread");
                // Fetch recent threads and find the one with matching root message ID
                // Use a larger count to increase chances of finding the thread
                let collection = client
                    .recent_threads(group, 500)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                let thread_views = threads_to_views(&collection);
                let mut thread = thread_views
                    .into_iter()
                    .find(|t| t.root_message_id == *message_id)
                    .ok_or_else(|| NntpError(format!("Thread not found: {}", message_id)))?;

                // Fetch full article bodies for all messages in the thread
                let message_ids = thread.root.collect_message_ids();
                let mut articles: HashMap<String, ArticleView> = HashMap::new();

                for msg_id in message_ids {
                    match client.fetch_article(&msg_id).await {
                        Ok(article) => {
                            articles.insert(msg_id, ArticleView::from(&article));
                        }
                        Err(e) => {
                            tracing::warn!(worker = self.id, %msg_id, error = %e, "Failed to fetch article body");
                        }
                    }
                }

                // Populate bodies into the thread tree
                thread.root.populate_bodies(&articles);

                Ok(NntpResponse::Thread(thread))
            }

            NntpRequest::GetArticle { message_id, .. } => {
                tracing::debug!(worker = self.id, %message_id, "Fetching article");
                let article = client
                    .fetch_article(message_id)
                    .await
                    .map_err(|e| NntpError(e.to_string()))?;

                Ok(NntpResponse::Article(ArticleView::from(&article)))
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
                }))
            }
        }
    }
}
