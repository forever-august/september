//! Configuration loading and constants.
//!
//! Loads application configuration from TOML files and defines constants for
//! HTTP cache TTLs, pagination settings, NNTP timeouts and limits, logging format,
//! and default paths. `AppConfig` is the root configuration struct containing all settings.

use const_format::formatcp;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
pub const HTTP_CACHE_THREAD_LIST_MAX_AGE: u32 = 2;
pub const HTTP_CACHE_THREAD_LIST_SWR: u32 = 5;

/// Thread view - may receive new replies
pub const HTTP_CACHE_THREAD_VIEW_MAX_AGE: u32 = 2;
pub const HTTP_CACHE_THREAD_VIEW_SWR: u32 = 5;

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

/// Maximum articles for HEAD fallback method (slowest path)
pub const NNTP_MAX_ARTICLES_HEAD_FALLBACK: u64 = 1000;

/// Multiplier for individual thread cache capacity (relative to thread_lists)
pub const THREAD_CACHE_MULTIPLIER: u64 = 10;

/// Divisor for negative cache size (relative to article cache)
pub const NEGATIVE_CACHE_SIZE_DIVISOR: u64 = 4;

// =============================================================================
// Incremental Update Constants
// =============================================================================

/// Debounce interval for incremental update checks (milliseconds)
/// Prevents checking for new articles more than once per second per group
pub const INCREMENTAL_DEBOUNCE_MS: u64 = 1000;

/// Minimum background refresh period for very active groups (seconds)
/// At 10,000 requests/second, refresh every 1 second
pub const BACKGROUND_REFRESH_MIN_PERIOD_SECS: u64 = 1;

/// Maximum background refresh period for barely active groups (seconds)  
/// Any activity at all = refresh every 30 seconds
pub const BACKGROUND_REFRESH_MAX_PERIOD_SECS: u64 = 30;

/// Moving average window for request rate calculation (seconds)
pub const ACTIVITY_WINDOW_SECS: u64 = 300; // 5 minutes

/// Number of buckets for activity tracking within the window
/// Bucket granularity = ACTIVITY_WINDOW_SECS / ACTIVITY_BUCKET_COUNT
/// e.g., 300s / 150 buckets = 2 seconds per bucket
pub const ACTIVITY_BUCKET_COUNT: u64 = 150;

/// High request rate threshold (requests/second) for minimum refresh period
pub const ACTIVITY_HIGH_RPS: f64 = 10000.0;

/// Interval between group stats background refreshes (1 hour)
pub const GROUP_STATS_REFRESH_INTERVAL_SECS: u64 = 3600;

/// Maximum polling attempts when waiting for a posted article to appear.
/// After posting, we poll the NNTP server until the article is found.
pub const POST_POLL_MAX_ATTEMPTS: u32 = 15;

/// Interval between polling attempts (milliseconds).
/// Total max wait time = POST_POLL_MAX_ATTEMPTS * POST_POLL_INTERVAL_MS
pub const POST_POLL_INTERVAL_MS: u64 = 10;

// =============================================================================
// Default Paths and Strings
// =============================================================================

/// Default configuration file path
pub const DEFAULT_CONFIG_PATH: &str = "dist/config/default.toml";

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
    /// Theme configuration
    #[serde(default)]
    pub theme: ThemeConfig,
    /// OpenID Connect authentication (optional)
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
}

/// HTTP server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct HttpServerConfig {
    pub host: String,
    pub port: u16,
    /// TLS configuration (ACME by default for secure-by-default)
    #[serde(default)]
    pub tls: TlsConfig,
}

/// TLS mode for HTTP server
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Automatic certificate provisioning via Let's Encrypt (default)
    #[default]
    Acme,
    /// User-provided certificate and key files
    Manual,
    /// No TLS - plain HTTP (explicit opt-out)
    None,
}

/// Default ACME cache directory
fn default_acme_cache_dir() -> String {
    "./acme-cache".to_string()
}

/// Default to enabling HTTP redirect when TLS is active
fn default_redirect_http() -> bool {
    true
}

/// Default HTTP redirect port
fn default_redirect_port() -> u16 {
    80
}

/// TLS configuration for the HTTP server
#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    /// TLS mode: "acme" (default), "manual", or "none"
    #[serde(default)]
    pub mode: TlsMode,

    // === Manual mode options ===
    /// Path to PEM-encoded certificate file
    pub cert_path: Option<String>,
    /// Path to PEM-encoded private key file
    pub key_path: Option<String>,

    // === ACME mode options ===
    /// Domain names for certificate (required for ACME mode)
    #[serde(default)]
    pub acme_domains: Vec<String>,
    /// Contact email for Let's Encrypt notifications (required for ACME mode)
    pub acme_email: Option<String>,
    /// Directory to cache certificates and account info
    #[serde(default = "default_acme_cache_dir")]
    pub acme_cache_dir: String,
    /// Use Let's Encrypt production (false = staging for testing)
    #[serde(default)]
    pub acme_production: bool,

    // === HTTP redirect options ===
    /// Enable HTTP->HTTPS redirect (default: true when TLS enabled)
    #[serde(default = "default_redirect_http")]
    pub redirect_http: bool,
    /// Port for HTTP redirect listener (default: 80)
    #[serde(default = "default_redirect_port")]
    pub redirect_port: u16,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            mode: TlsMode::default(),
            cert_path: None,
            key_path: None,
            acme_domains: Vec::new(),
            acme_email: None,
            acme_cache_dir: default_acme_cache_dir(),
            acme_production: false,
            redirect_http: default_redirect_http(),
            redirect_port: default_redirect_port(),
        }
    }
}

impl TlsConfig {
    /// Validate TLS configuration based on mode
    pub fn validate(&self) -> Result<(), ConfigError> {
        match self.mode {
            TlsMode::Acme => {
                if self.acme_domains.is_empty() {
                    return Err(ConfigError::Validation(
                        "TLS mode 'acme' (default) requires [http.tls] acme_domains. \
                         Set domains or use mode = 'none' to disable TLS."
                            .to_string(),
                    ));
                }
                if self.acme_email.is_none() {
                    return Err(ConfigError::Validation(
                        "TLS mode 'acme' requires acme_email for Let's Encrypt notifications."
                            .to_string(),
                    ));
                }
            }
            TlsMode::Manual => {
                if self.cert_path.is_none() {
                    return Err(ConfigError::Validation(
                        "TLS mode 'manual' requires cert_path.".to_string(),
                    ));
                }
                if self.key_path.is_none() {
                    return Err(ConfigError::Validation(
                        "TLS mode 'manual' requires key_path.".to_string(),
                    ));
                }
            }
            TlsMode::None => {
                // No validation needed, but we'll log a warning at startup
            }
        }
        Ok(())
    }

    /// Check if TLS is enabled (any mode except None)
    pub fn is_enabled(&self) -> bool {
        self.mode != TlsMode::None
    }
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
    /// Username for NNTP authentication (requires TLS unless allow_insecure_auth is set)
    pub username: Option<String>,
    /// Password for NNTP authentication (requires TLS unless allow_insecure_auth is set)
    pub password: Option<String>,
    /// Allow authentication over plaintext connections (INSECURE - only for testing)
    #[serde(default)]
    pub allow_insecure_auth: bool,
}

impl NntpServerConfig {
    /// Get effective timeout (server-specific or global default)
    pub fn timeout_seconds(&self, global: &NntpSettings) -> u64 {
        self.timeout_seconds.unwrap_or(global.timeout_seconds)
    }

    /// Get effective request timeout (server-specific or global default)
    pub fn request_timeout_seconds(&self, global: &NntpSettings) -> u64 {
        self.request_timeout_seconds
            .unwrap_or(global.request_timeout_seconds)
    }

    /// Get worker count (default: 4)
    pub fn worker_count(&self) -> usize {
        self.worker_count.unwrap_or(4)
    }

    /// Check if credentials are configured (both username and password)
    pub fn has_credentials(&self) -> bool {
        self.username.is_some() && self.password.is_some()
    }

    /// Check if TLS is required for credentials
    /// Returns true if credentials are configured AND allow_insecure_auth is false
    pub fn requires_tls_for_credentials(&self) -> bool {
        self.has_credentials() && !self.allow_insecure_auth
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
            allow_insecure_auth: false,
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
        1800 // 30 minutes
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

/// Theme configuration for templates and static assets.
///
/// Themes are stored in `{themes_dir}/{name}/` with `templates/` and `static/`
/// subdirectories. The active theme can selectively override files from the
/// default theme - any files not present fall back to the default theme.
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeConfig {
    /// Active theme name (default: "default")
    #[serde(default = "ThemeConfig::default_name")]
    pub name: String,

    /// Base directory containing themes.
    /// Production default: "/etc/september/themes"
    /// Development: typically "dist/themes"
    #[serde(default = "ThemeConfig::default_themes_dir")]
    pub themes_dir: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: Self::default_name(),
            themes_dir: Self::default_themes_dir(),
        }
    }
}

impl ThemeConfig {
    fn default_name() -> String {
        "default".to_string()
    }

    fn default_themes_dir() -> String {
        "/etc/september/themes".to_string()
    }

    /// Get path to templates for a specific theme.
    pub fn templates_path(&self, theme_name: &str) -> PathBuf {
        Path::new(&self.themes_dir)
            .join(theme_name)
            .join("templates")
    }

    /// Get path to static files for a specific theme.
    pub fn static_path(&self, theme_name: &str) -> PathBuf {
        Path::new(&self.themes_dir).join(theme_name).join("static")
    }

    /// Validate the theme configuration.
    ///
    /// Checks that the themes directory exists and contains the required
    /// default theme and active theme directories with templates and static subdirs.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let themes_dir = Path::new(&self.themes_dir);
        if !themes_dir.exists() {
            return Err(ConfigError::Validation(format!(
                "Themes directory not found: {}",
                self.themes_dir
            )));
        }

        // Validate default theme exists
        let default_templates = self.templates_path("default");
        if !default_templates.exists() {
            return Err(ConfigError::Validation(format!(
                "Default theme templates not found: {}",
                default_templates.display()
            )));
        }

        let default_static = self.static_path("default");
        if !default_static.exists() {
            return Err(ConfigError::Validation(format!(
                "Default theme static files not found: {}",
                default_static.display()
            )));
        }

        // If using a non-default theme, validate it exists
        if self.name != "default" {
            let theme_dir = themes_dir.join(&self.name);
            if !theme_dir.exists() {
                return Err(ConfigError::Validation(format!(
                    "Theme '{}' not found at: {}",
                    self.name,
                    theme_dir.display()
                )));
            }
        }

        Ok(())
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
                "No NNTP servers configured. Add [[server]] sections or legacy [nntp] server/port"
                    .to_string(),
            ));
        }

        // Validate OIDC providers if configured
        if let Some(ref oidc) = config.oidc {
            if oidc.providers.is_empty() {
                return Err(ConfigError::Validation(
                    "OIDC configured but no providers defined. Add [[oidc.provider]] sections."
                        .to_string(),
                ));
            }
            for provider in &oidc.providers {
                provider.validate()?;
            }
        }

        // Validate TLS configuration
        config.http.tls.validate()?;

        // Validate theme configuration
        config.theme.validate()?;

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
    #[error("Secret resolution failed: {0}")]
    SecretResolution(String),
}

/// Resolve a secret value from various sources.
/// Supports:
/// - `env:VAR_NAME` - read from environment variable
/// - `file:/path/to/secret` - read from file (trimmed)
/// - literal value - used as-is
pub fn resolve_secret(value: &str) -> Result<String, ConfigError> {
    if let Some(var_name) = value.strip_prefix("env:") {
        std::env::var(var_name).map_err(|e| {
            ConfigError::SecretResolution(format!(
                "Failed to read environment variable '{}': {}",
                var_name, e
            ))
        })
    } else if let Some(file_path) = value.strip_prefix("file:") {
        std::fs::read_to_string(file_path)
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                ConfigError::SecretResolution(format!(
                    "Failed to read secret file '{}': {}",
                    file_path, e
                ))
            })
    } else {
        Ok(value.to_string())
    }
}

/// OpenID Connect configuration (optional section)
#[derive(Debug, Clone, Deserialize)]
pub struct OidcConfig {
    /// Secret for signing session cookies.
    /// Supports: env:VAR_NAME, file:/path, or literal value (64+ chars recommended)
    pub cookie_secret: String,

    /// Session lifetime in days (default: 30)
    #[serde(default = "OidcConfig::default_session_lifetime")]
    pub session_lifetime_days: u64,

    /// Optional override for redirect URI base URL.
    /// If not set, auto-detected from request Host header.
    pub redirect_uri_base: Option<String>,

    /// OIDC/OAuth2 providers
    #[serde(default, rename = "provider")]
    pub providers: Vec<OidcProviderConfig>,
}

impl OidcConfig {
    fn default_session_lifetime() -> u64 {
        30
    }

    /// Resolve the cookie secret from env/file/literal
    pub fn resolve_cookie_secret(&self) -> Result<String, ConfigError> {
        resolve_secret(&self.cookie_secret)
    }
}

/// Configuration for a single OIDC/OAuth2 provider
#[derive(Debug, Clone, Deserialize)]
pub struct OidcProviderConfig {
    /// URL-safe identifier (used in routes like /auth/login/google)
    pub name: String,

    /// Human-readable name shown on login page
    pub display_name: String,

    // === Discovery mode (preferred for OIDC providers) ===
    /// OIDC issuer URL - endpoints discovered automatically via .well-known/openid-configuration
    pub issuer_url: Option<String>,

    // === Manual mode (fallback for OAuth2-only providers like GitHub) ===
    /// Authorization endpoint URL
    pub auth_url: Option<String>,
    /// Token endpoint URL  
    pub token_url: Option<String>,
    /// UserInfo endpoint URL
    pub userinfo_url: Option<String>,

    /// OAuth2 client ID
    pub client_id: String,

    /// OAuth2 client secret.
    /// Supports: env:VAR_NAME, file:/path, or literal value
    pub client_secret: String,

    /// Field name for subject ID in userinfo response (default: "sub")
    /// GitHub uses "id" instead of "sub"
    #[serde(default = "OidcProviderConfig::default_sub_field")]
    pub userinfo_sub_field: String,
}

impl OidcProviderConfig {
    fn default_sub_field() -> String {
        "sub".to_string()
    }

    /// Check if this provider uses OIDC discovery mode
    pub fn uses_discovery(&self) -> bool {
        self.issuer_url.is_some()
    }

    /// Check if this provider uses manual endpoint configuration
    pub fn uses_manual_endpoints(&self) -> bool {
        self.auth_url.is_some() || self.token_url.is_some() || self.userinfo_url.is_some()
    }

    /// Validate the provider configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        let has_discovery = self.issuer_url.is_some();
        let has_manual =
            self.auth_url.is_some() || self.token_url.is_some() || self.userinfo_url.is_some();

        if has_discovery && has_manual {
            return Err(ConfigError::Validation(format!(
                "Provider '{}': cannot specify both issuer_url and manual endpoints (auth_url/token_url/userinfo_url)",
                self.name
            )));
        }

        if !has_discovery && !has_manual {
            return Err(ConfigError::Validation(format!(
                "Provider '{}': must specify either issuer_url (for OIDC discovery) or all manual endpoints (auth_url, token_url, userinfo_url)",
                self.name
            )));
        }

        if has_manual {
            if self.auth_url.is_none() {
                return Err(ConfigError::Validation(format!(
                    "Provider '{}': manual mode requires auth_url",
                    self.name
                )));
            }
            if self.token_url.is_none() {
                return Err(ConfigError::Validation(format!(
                    "Provider '{}': manual mode requires token_url",
                    self.name
                )));
            }
            if self.userinfo_url.is_none() {
                return Err(ConfigError::Validation(format!(
                    "Provider '{}': manual mode requires userinfo_url",
                    self.name
                )));
            }
        }

        // Validate name is URL-safe (alphanumeric, dash, underscore only)
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ConfigError::Validation(format!(
                "Provider '{}': name must contain only alphanumeric characters, dashes, and underscores",
                self.name
            )));
        }

        Ok(())
    }

    /// Resolve the client secret from env/file/literal
    pub fn resolve_client_secret(&self) -> Result<String, ConfigError> {
        resolve_secret(&self.client_secret)
    }
}
