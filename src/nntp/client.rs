use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;

use nntp_rs::runtime::tokio::NntpClient;
use nntp_rs::threading::{FetchedArticle, NntpClientThreadingExt, ThreadCollection};
use nntp_rs::ListVariant;

use crate::config::NntpConfig;
use crate::error::AppError;

/// Connection pool for NNTP clients
#[derive(Clone)]
pub struct NntpPool {
    config: Arc<NntpConfig>,
    connections: Arc<Mutex<Vec<NntpClient>>>,
}

impl NntpPool {
    pub fn new(config: NntpConfig) -> Self {
        Self {
            config: Arc::new(config),
            connections: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get a client from the pool or create a new one
    pub async fn get(&self) -> Result<PooledClient, AppError> {
        let mut pool = self.connections.lock().await;

        let client = if let Some(client) = pool.pop() {
            client
        } else {
            self.create_client().await?
        };

        Ok(PooledClient {
            client: Some(client),
            pool: self.connections.clone(),
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
    pool: Arc<Mutex<Vec<NntpClient>>>,
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
            let pool = self.pool.clone();
            tokio::spawn(async move {
                let mut pool = pool.lock().await;
                // Keep pool size reasonable
                if pool.len() < 5 {
                    pool.push(client);
                }
            });
        }
    }
}
