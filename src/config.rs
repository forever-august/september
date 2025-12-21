use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// HTTP server configuration
    pub http: HttpServerConfig,
    /// Global NNTP settings and defaults
    pub nntp: NntpSettings,
    /// NNTP servers (federated pool)
    #[serde(default)]
    pub server: Vec<NntpServerConfig>,
    pub ui: UiConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

/// HTTP server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct HttpServerConfig {
    pub host: String,
    pub port: u16,
}

/// Global NNTP settings that apply to all servers unless overridden
#[derive(Debug, Clone, Deserialize)]
pub struct NntpSettings {
    /// Connection timeout in seconds (can be overridden per-server)
    #[serde(default = "NntpSettings::default_timeout")]
    pub timeout_seconds: u64,
    /// Request timeout in seconds (can be overridden per-server)
    #[serde(default = "NntpSettings::default_request_timeout")]
    pub request_timeout_seconds: u64,
    /// Default newsgroup and display settings
    pub defaults: NntpDefaults,

    // Legacy fields for backward compatibility (used if no [[server]] sections)
    #[serde(rename = "server")]
    legacy_server: Option<String>,
    #[serde(rename = "port")]
    legacy_port: Option<u16>,
    legacy_worker_count: Option<usize>,
    #[serde(rename = "username")]
    legacy_username: Option<String>,
    #[serde(rename = "password")]
    legacy_password: Option<String>,
}

impl NntpSettings {
    fn default_timeout() -> u64 {
        30
    }

    fn default_request_timeout() -> u64 {
        30
    }
}

/// Configuration for a single NNTP server
#[derive(Debug, Clone, Deserialize)]
pub struct NntpServerConfig {
    /// Server name (used for logging and identification)
    pub name: String,
    /// NNTP server hostname
    pub host: String,
    /// NNTP server port
    pub port: u16,
    /// Connection timeout (overrides global setting)
    pub timeout_seconds: Option<u64>,
    /// Request timeout (overrides global setting)
    pub request_timeout_seconds: Option<u64>,
    /// Number of worker connections for this server (default: 4)
    pub worker_count: Option<usize>,
    /// Username for NNTP authentication (requires TLS)
    pub username: Option<String>,
    /// Password for NNTP authentication (requires TLS)
    pub password: Option<String>,
}

impl NntpServerConfig {
    /// Get effective timeout (server-specific or global default)
    pub fn timeout_seconds(&self, global: &NntpSettings) -> u64 {
        self.timeout_seconds.unwrap_or(global.timeout_seconds)
    }

    /// Get effective request timeout (server-specific or global default)
    pub fn request_timeout_seconds(&self, global: &NntpSettings) -> u64 {
        self.request_timeout_seconds.unwrap_or(global.request_timeout_seconds)
    }

    /// Get worker count (default: 4)
    pub fn worker_count(&self) -> usize {
        self.worker_count.unwrap_or(4)
    }

    /// Check if credentials are configured (both username and password)
    pub fn has_credentials(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }

    /// Create from legacy NntpSettings (backward compatibility)
    fn from_legacy(settings: &NntpSettings) -> Option<Self> {
        let server = settings.legacy_server.as_ref()?;
        let port = settings.legacy_port?;

        Some(Self {
            name: "default".to_string(),
            host: server.clone(),
            port,
            timeout_seconds: Some(settings.timeout_seconds),
            request_timeout_seconds: Some(settings.request_timeout_seconds),
            worker_count: settings.legacy_worker_count,
            username: settings.legacy_username.clone(),
            password: settings.legacy_password.clone(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NntpDefaults {
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
        let mut config: AppConfig = toml::from_str(&contents)?;

        // Backward compatibility: if no [[server]] sections, convert legacy [nntp] config
        if config.server.is_empty() {
            if let Some(legacy_server) = NntpServerConfig::from_legacy(&config.nntp) {
                config.server.push(legacy_server);
            }
        }

        // Validate: at least one server must be configured
        if config.server.is_empty() {
            return Err(ConfigError::Validation(
                "No NNTP servers configured. Add [[server]] sections or legacy [nntp] server/port".to_string()
            ));
        }

        Ok(config)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("Configuration error: {0}")]
    Validation(String),
}
