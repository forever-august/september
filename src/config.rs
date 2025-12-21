use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub nntp: NntpConfig,
    pub ui: UiConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NntpConfig {
    pub server: String,
    pub port: u16,
    pub timeout_seconds: u64,
    pub defaults: NntpDefaults,
    /// Number of NNTP worker clients (defaults to 4)
    pub worker_count: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NntpDefaults {
    pub default_group: String,
    pub threads_per_page: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    /// Site title shown in header and page titles. Defaults to NNTP server name.
    pub site_name: Option<String>,
    pub collapse_threshold: usize,
    /// Version string, populated at runtime
    #[serde(skip_deserializing, default = "UiConfig::default_version")]
    pub version: String,
}

impl UiConfig {
    fn default_version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    /// TTL for cached articles in seconds (default: 24 hours)
    #[serde(default = "CacheConfig::default_article_ttl")]
    pub article_ttl_seconds: u64,
    /// TTL for cached thread lists in seconds (default: 5 minutes)
    #[serde(default = "CacheConfig::default_threads_ttl")]
    pub threads_ttl_seconds: u64,
    /// TTL for cached group list in seconds (default: 1 hour)
    #[serde(default = "CacheConfig::default_groups_ttl")]
    pub groups_ttl_seconds: u64,
    /// Maximum number of cached articles (default: 10000)
    #[serde(default = "CacheConfig::default_max_articles")]
    pub max_articles: u64,
    /// Maximum number of cached thread lists (default: 100)
    #[serde(default = "CacheConfig::default_max_thread_lists")]
    pub max_thread_lists: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            article_ttl_seconds: Self::default_article_ttl(),
            threads_ttl_seconds: Self::default_threads_ttl(),
            groups_ttl_seconds: Self::default_groups_ttl(),
            max_articles: Self::default_max_articles(),
            max_thread_lists: Self::default_max_thread_lists(),
        }
    }
}

impl CacheConfig {
    fn default_article_ttl() -> u64 {
        86400 // 24 hours
    }
    fn default_threads_ttl() -> u64 {
        300 // 5 minutes
    }
    fn default_groups_ttl() -> u64 {
        3600 // 1 hour
    }
    fn default_max_articles() -> u64 {
        10000
    }
    fn default_max_thread_lists() -> u64 {
        100
    }
}

impl AppConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&contents)?;
        Ok(config)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
}
