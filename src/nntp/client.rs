use std::sync::Arc;
use std::time::Duration;

use async_channel::{bounded, Receiver, Sender};
use tokio::time::timeout;
use nntp_rs::runtime::tokio::NntpClient;
use nntp_rs::threading::{FetchedArticle, NntpClientThreadingExt, ThreadCollection};
use nntp_rs::ListVariant;

use crate::config::{NntpConfig, NNTP_MAX_POOL_SIZE};
use crate::error::AppError;

/// Connection pool for NNTP clients using lock-free async channels
#[derive(Clone)]
pub struct NntpPool {
    config: Arc<NntpConfig>,
    /// Sender for returning connections to the pool
    pool_tx: Sender<NntpClient>,
    /// Receiver for getting connections from the pool
    pool_rx: Receiver<NntpClient>,
}

impl NntpPool {
    pub fn new(config: NntpConfig) -> Self {
        // Create a bounded channel with capacity for max pool size
        // This provides natural backpressure and limits pool growth
        let (pool_tx, pool_rx) = bounded(NNTP_MAX_POOL_SIZE);
        Self {
            config: Arc::new(config),
            pool_tx,
            pool_rx,
        }
    }

    /// Get a client from the pool or create a new one
    pub async fn get(&self) -> Result<PooledClient, AppError> {
        // Try to get an existing connection from the pool (non-blocking)
        let client = match self.pool_rx.try_recv() {
            Ok(client) => client,
            Err(_) => {
                // No connection available, create a new one
                self.create_client().await?
            }
        };

        Ok(PooledClient {
            client: Some(client),
            pool_tx: self.pool_tx.clone(),
        })
    }

    async fn create_client(&self) -> Result<NntpClient, AppError> {
        let addr = format!("{}:{}", self.config.server, self.config.port);
        let connect_timeout = Duration::from_secs(self.config.timeout_seconds);

        let client = timeout(connect_timeout, NntpClient::connect(&addr))
            .await
            .map_err(|_| {
                AppError::Internal(format!(
                    "Connection timeout to NNTP server: {}",
                    self.config.server
                ))
            })??;

        Ok(client)
    }
}

/// RAII wrapper that returns connection to pool on drop
pub struct PooledClient {
    client: Option<NntpClient>,
    pool_tx: Sender<NntpClient>,
}

impl PooledClient {
    /// Fetch recent threads from a newsgroup
    pub async fn recent_threads(
        &mut self,
        group: &str,
        count: u64,
    ) -> Result<ThreadCollection, AppError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| AppError::Internal("Connection not available".to_string()))?;

        Ok(client.recent_threads(group, count).await?)
    }

    /// Fetch a single article by message ID
    pub async fn fetch_article(&mut self, message_id: &str) -> Result<FetchedArticle, AppError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| AppError::Internal("Connection not available".to_string()))?;

        Ok(client.fetch_article(message_id).await?)
    }

    /// List available newsgroups
    pub async fn list_groups(&mut self) -> Result<Vec<(String, Option<String>)>, AppError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| AppError::Internal("Connection not available".to_string()))?;

        // Use Active variant to list newsgroups (NewsGroup has no description field)
        let groups = client.list(ListVariant::Active(None)).await?;
        Ok(groups
            .iter()
            .map(|g| (g.name.clone(), None))
            .collect())
    }
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            // Use try_send which is non-blocking and doesn't require async
            // If the channel is full (pool at capacity), the connection is dropped
            let _ = self.pool_tx.try_send(client);
        }
    }
}
