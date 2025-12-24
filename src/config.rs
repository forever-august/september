//! Configuration loading and constants.
//!
//! Loads application configuration from TOML files and defines constants for
//! HTTP cache TTLs, pagination settings, NNTP timeouts and limits, logging format,
//! and default paths. `AppConfig` is the root configuration struct containing all settings.

use const_format::formatcp;
use serde::{Deserialize, Serialize};
use std::path::Path;

// =============================================================================
// HTTP Response Cache Control
// =============================================================================
// These constants control Cache-Control headers for upstream caches (Varnish, nginx, CDNs).
// All values are in seconds. Directives used:
// - max-age: How long the response is considered fresh
// - stale-while-revalidate: Serve stale while fetching fresh in background
// - stale-if-error: Serve stale content if origin returns 5xx (thundering herd protection)
//
// References:
// - RFC 9111 (HTTP Caching): https://httpwg.org/specs/rfc9111.html
// - RFC 5861 (stale-* extensions): https://httpwg.org/specs/rfc5861.html

/// Home and browse pages - group listings change infrequently
pub const HTTP_CACHE_HOME_MAX_AGE: u32 = 60;
pub const HTTP_CACHE_HOME_SWR: u32 = 30;

/// Thread list - new threads appear regularly
pub const HTTP_CACHE_THREAD_LIST_MAX_AGE: u32 = 30;
pub const HTTP_CACHE_THREAD_LIST_SWR: u32 = 30;

/// Thread view - may receive new replies
pub const HTTP_CACHE_THREAD_VIEW_MAX_AGE: u32 = 120;
pub const HTTP_CACHE_THREAD_VIEW_SWR: u32 = 60;

/// Individual articles - immutable content
pub const HTTP_CACHE_ARTICLE_MAX_AGE: u32 = 3600;
pub const HTTP_CACHE_ARTICLE_SWR: u32 = 60;

/// Static assets (CSS, JS) - long cache with immutable hint
pub const HTTP_CACHE_STATIC_MAX_AGE: u32 = 86400;

/// Error responses - short TTL to prevent thundering herd while allowing quick recovery
pub const HTTP_CACHE_ERROR_MAX_AGE: u32 = 5;

/// Stale-if-error duration - serve stale content during backend failures (5 minutes)
pub const HTTP_CACHE_STALE_IF_ERROR: u32 = 300;

// Pre-formatted Cache-Control header values (compile-time string concatenation)
pub const CACHE_CONTROL_HOME: &str = formatcp!(
    "public, max-age={}, stale-while-revalidate={}, stale-if-error={}",
    HTTP_CACHE_HOME_MAX_AGE,
    HTTP_CACHE_HOME_SWR,
    HTTP_CACHE_STALE_IF_ERROR
);

pub const CACHE_CONTROL_THREAD_LIST: &str = formatcp!(
    "public, max-age={}, stale-while-revalidate={}, stale-if-error={}",
    HTTP_CACHE_THREAD_LIST_MAX_AGE,
    HTTP_CACHE_THREAD_LIST_SWR,
    HTTP_CACHE_STALE_IF_ERROR
);

pub const CACHE_CONTROL_THREAD_VIEW: &str = formatcp!(
    "public, max-age={}, stale-while-revalidate={}, stale-if-error={}",
    HTTP_CACHE_THREAD_VIEW_MAX_AGE,
    HTTP_CACHE_THREAD_VIEW_SWR,
    HTTP_CACHE_STALE_IF_ERROR
);

pub const CACHE_CONTROL_ARTICLE: &str = formatcp!(
    "public, max-age={}, stale-while-revalidate={}, stale-if-error={}",
    HTTP_CACHE_ARTICLE_MAX_AGE,
    HTTP_CACHE_ARTICLE_SWR,
    HTTP_CACHE_STALE_IF_ERROR
);

pub const CACHE_CONTROL_STATIC: &str =
    formatcp!("public, max-age={}, immutable", HTTP_CACHE_STATIC_MAX_AGE);

pub const CACHE_CONTROL_ERROR: &str = formatcp!("public, max-age={}", HTTP_CACHE_ERROR_MAX_AGE);

// =============================================================================
// Template / Preview Constants
// =============================================================================

/// Maximum characters for article preview (hard limit)
pub const PREVIEW_HARD_LIMIT: usize = 1024;

/// Default number of lines for preview filter
pub const DEFAULT_PREVIEW_LINES: usize = 10;

/// Default word count for truncate_words filter
pub const DEFAULT_TRUNCATE_WORDS: usize = 50;

// Time unit constants (in seconds) for timeago filter
/// Seconds in a minute
pub const SECONDS_PER_MINUTE: i64 = 60;
/// Seconds in an hour
pub const SECONDS_PER_HOUR: i64 = 3600;
/// Seconds in a day
pub const SECONDS_PER_DAY: i64 = 86400;
/// Seconds in a 30-day month
pub const SECONDS_PER_MONTH: i64 = 2592000;
/// Seconds in a 365-day year
pub const SECONDS_PER_YEAR: i64 = 31536000;

// =============================================================================
// UI / Pagination Constants
// =============================================================================

/// Pagination window size (pages shown on each side of current page)
pub const PAGINATION_WINDOW: usize = 2;

// =============================================================================
// NNTP Channel and Queue Constants
// =============================================================================

/// Capacity of the high-priority request queue (user-facing operations)
pub const NNTP_HIGH_PRIORITY_QUEUE_CAPACITY: usize = 50;

/// Capacity of the normal-priority request queue (page load operations)
pub const NNTP_NORMAL_PRIORITY_QUEUE_CAPACITY: usize = 50;

/// Capacity of the low-priority request queue (background operations)
pub const NNTP_LOW_PRIORITY_QUEUE_CAPACITY: usize = 100;

/// Aging threshold in seconds: process low-priority requests after this duration
/// of starvation to prevent indefinite delays under sustained high load
pub const NNTP_PRIORITY_AGING_SECS: u64 = 10;

/// Capacity of broadcast channels for request coalescing
pub const BROADCAST_CHANNEL_CAPACITY: usize = 16;

// =============================================================================
// NNTP Retry and Timeout Constants
// =============================================================================

/// Delay in seconds before reconnecting after connection failure
pub const NNTP_RECONNECT_DELAY_SECS: u64 = 5;

/// TTL in seconds for negative cache (article not found)
pub const NNTP_NEGATIVE_CACHE_TTL_SECS: u64 = 30;

// =============================================================================
// NNTP Article Fetch Limits
// =============================================================================

/// Maximum articles to fetch per request (prevents timeout on large groups)
pub const NNTP_MAX_ARTICLES_PER_REQUEST: u64 = 10000;

/// Maximum articles to search when fetching a single thread
pub const NNTP_MAX_ARTICLES_SINGLE_THREAD: u64 = 5000;

/// Maximum articles for HEAD fallback method (slowest path)
pub const NNTP_MAX_ARTICLES_HEAD_FALLBACK: u64 = 1000;

/// Multiplier for individual thread cache capacity (relative to thread_lists)
pub const THREAD_CACHE_MULTIPLIER: u64 = 10;

/// Divisor for negative cache size (relative to article cache)
pub const NEGATIVE_CACHE_SIZE_DIVISOR: u64 = 4;

// =============================================================================
// Default Paths and Strings
// =============================================================================

/// Default configuration file path
pub const DEFAULT_CONFIG_PATH: &str = "config/default.toml";

/// Glob pattern for template files
pub const TEMPLATE_GLOB: &str = "templates/**/*";

/// Directory for static files
pub const STATIC_DIR: &str = "static";

/// Default subject for articles without a subject
pub const DEFAULT_SUBJECT: &str = "(no subject)";

/// Default log filter when RUST_LOG is not set
pub const DEFAULT_LOG_FILTER: &str = "september=debug,tower_http=debug";

/// Default log format (text or json)
pub const DEFAULT_LOG_FORMAT: &str = "text";

/// Default server name for legacy config migration
pub const DEFAULT_SERVER_NAME: &str = "default";

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
    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,
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
            name: DEFAULT_SERVER_NAME.to_string(),
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
    #[serde(default = "NntpDefaults::default_articles_per_page")]
    pub articles_per_page: usize,
    /// Maximum number of articles to fetch per group (default: 500)
    #[serde(default = "NntpDefaults::default_max_articles_per_group")]
    pub max_articles_per_group: u64,
}

impl NntpDefaults {
    fn default_articles_per_page() -> usize {
        20
    }

    fn default_max_articles_per_group() -> u64 {
        500
    }
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
    /// Maximum number of cached group stats (default: 1000)
    #[serde(default = "CacheConfig::default_max_group_stats")]
    pub max_group_stats: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            article_ttl_seconds: Self::default_article_ttl(),
            threads_ttl_seconds: Self::default_threads_ttl(),
            groups_ttl_seconds: Self::default_groups_ttl(),
            max_articles: Self::default_max_articles(),
            max_thread_lists: Self::default_max_thread_lists(),
            max_group_stats: Self::default_max_group_stats(),
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
    fn default_max_group_stats() -> u64 {
        1000
    }
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Log format: "text" (human-readable, default) or "json" (structured)
    #[serde(default = "LoggingConfig::default_format")]
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            format: DEFAULT_LOG_FORMAT.to_string(),
        }
    }
}

impl LoggingConfig {
    fn default_format() -> String {
        DEFAULT_LOG_FORMAT.to_string()
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
