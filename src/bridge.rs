//! Bridge functionality for connecting HTTP requests to NNTP operations

use crate::error::{Result, SeptemberError};
use crate::nntp::{Article, Newsgroup, NntpClient};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Configuration for the NNTP bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub nntp_server: String,
    pub nntp_port: u16,
    pub cache_timeout_seconds: u64,
    pub max_articles_per_request: usize,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            nntp_server: "news.example.com".to_string(),
            nntp_port: 119,
            cache_timeout_seconds: 300, // 5 minutes
            max_articles_per_request: 50,
        }
    }
}

/// The main bridge service that handles HTTP to NNTP translation
pub struct NntpBridge {
    config: BridgeConfig,
    client: Arc<RwLock<Option<NntpClient>>>,
}

impl NntpBridge {
    /// Create a new NNTP bridge with the given configuration
    pub fn new(config: BridgeConfig) -> Self {
        Self {
            config,
            client: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the bridge and connect to the NNTP server
    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing NNTP bridge");

        let mut client = NntpClient::new(self.config.nntp_server.clone(), self.config.nntp_port);

        client.connect().await.map_err(|e| {
            error!("Failed to connect to NNTP server: {}", e);
            e
        })?;

        let mut client_guard = self.client.write().await;
        *client_guard = Some(client);

        info!("NNTP bridge initialized successfully");
        Ok(())
    }

    /// Get list of available newsgroups
    pub async fn get_newsgroups(&self) -> Result<Vec<Newsgroup>> {
        debug!("Fetching newsgroups via bridge");

        let client_guard = self.client.read().await;
        match client_guard.as_ref() {
            Some(client) => client.list_groups().await.map_err(|e| {
                error!("Failed to fetch newsgroups: {}", e);
                e
            }),
            None => Err(SeptemberError::NntpConnection(
                "NNTP client not initialized".to_string(),
            )),
        }
    }

    /// Get articles from a specific newsgroup
    pub async fn get_articles(&self, group: &str, limit: Option<usize>) -> Result<Vec<Article>> {
        let limit = limit
            .unwrap_or(self.config.max_articles_per_request)
            .min(self.config.max_articles_per_request);

        debug!("Fetching articles from group: {}, limit: {}", group, limit);

        let client_guard = self.client.read().await;
        match client_guard.as_ref() {
            Some(client) => client.fetch_articles(group, limit).await.map_err(|e| {
                error!("Failed to fetch articles from {}: {}", group, e);
                e
            }),
            None => Err(SeptemberError::NntpConnection(
                "NNTP client not initialized".to_string(),
            )),
        }
    }

    /// Get a specific article by ID
    pub async fn get_article(&self, article_id: &str) -> Result<Article> {
        debug!("Fetching article: {}", article_id);

        let client_guard = self.client.read().await;
        match client_guard.as_ref() {
            Some(client) => client.fetch_article(article_id).await.map_err(|e| {
                error!("Failed to fetch article {}: {}", article_id, e);
                e
            }),
            None => Err(SeptemberError::NntpConnection(
                "NNTP client not initialized".to_string(),
            )),
        }
    }

    /// Health check for the bridge
    pub async fn health_check(&self) -> bool {
        let client_guard = self.client.read().await;
        client_guard.is_some()
    }

    /// Shutdown the bridge and disconnect from NNTP
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down NNTP bridge");

        let mut client_guard = self.client.write().await;
        if let Some(mut client) = client_guard.take() {
            client.disconnect().await?;
        }

        info!("NNTP bridge shutdown completed");
        Ok(())
    }
}
