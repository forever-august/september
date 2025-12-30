//! September: an NNTP web interface.
//!
//! This is the application entry point. It initializes tracing, loads configuration
//! from TOML files, creates the NNTP federated service, spawns worker connections,
//! sets up the Axum router with all routes, and starts the HTTP server.

mod config;
mod error;
mod http;
mod middleware;
mod nntp;
mod oidc;
mod routes;
mod state;
mod templates;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::{AppConfig, TlsMode, DEFAULT_CONFIG_PATH, DEFAULT_LOG_FILTER};

/// September: A web interface to NNTP servers
#[derive(Parser, Debug)]
#[command(name = "september", version, about)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
    config: String,

    /// Log level filter (e.g., "september=debug,tower_http=info")
    #[arg(short, long)]
    log_level: Option<String>,

    /// Log format: "text" (human-readable) or "json" (structured)
    #[arg(long)]
    log_format: Option<String>,
}
use std::sync::Arc;

use nntp::NntpFederatedService;
use oidc::OidcManager;
use routes::create_router;
use state::AppState;
use templates::init_templates;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Install rustls crypto provider (must be done before any TLS operations)
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Parse command line arguments
    let args = Args::parse();

    // Load configuration first (before tracing, so we can use config for log format)
    let mut config = AppConfig::load(&args.config)?;

    // Default site_name to first server name if not configured
    if config.ui.site_name.is_none() {
        config.ui.site_name = config.server.first().map(|s| s.name.clone());
    }

    // Initialize tracing with priority: CLI > config > env > default
    let log_filter = args
        .log_level
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string());

    // Determine log format: CLI > config file > default ("text")
    let log_format = args.log_format.as_deref().unwrap_or(&config.logging.format);

    // Build the subscriber with appropriate format layer
    let env_filter = tracing_subscriber::EnvFilter::new(&log_filter);

    if log_format == "json" {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    tracing::info!(format = %log_format, "Logging initialized");

    // Log configured servers
    for server in &config.server {
        tracing::info!(
            name = %server.name,
            host = %server.host,
            port = server.port,
            workers = server.worker_count(),
            has_auth = server.has_credentials(),
            "NNTP server configured"
        );
    }

    // Initialize Tera templates with theme support
    let tera = init_templates(&config.theme)?;
    tracing::info!(
        theme = %config.theme.name,
        themes_dir = %config.theme.themes_dir,
        "Initialized templates"
    );

    // Initialize federated NNTP service with caching and worker pools
    let nntp_service = NntpFederatedService::new(&config);
    nntp_service.spawn_workers();
    tracing::info!(
        servers = ?nntp_service.server_names(),
        "Initialized federated NNTP service"
    );

    // Warmup: prefetch and cache the groups list before accepting requests
    // This ensures the first request doesn't pay the NNTP fetch latency
    match nntp_service.get_groups().await {
        Ok(groups) => {
            tracing::info!(count = groups.len(), "Warmed up groups cache");
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to warm up groups cache");
        }
    }

    // Spawn background refresh task for active groups
    Arc::new(nntp_service.clone()).spawn_background_refresh();
    tracing::info!("Spawned background refresh task");

    // Initialize OIDC if configured
    let oidc = if let Some(ref oidc_config) = config.oidc {
        match OidcManager::new(oidc_config).await {
            Ok(manager) => {
                tracing::info!(
                    providers = manager.provider_count(),
                    "Initialized OIDC authentication"
                );
                Some(manager)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize OIDC");
                return Err(e.into());
            }
        }
    } else {
        tracing::info!("OIDC not configured, authentication disabled");
        None
    };

    // Create application state
    let state = AppState::new(config.clone(), tera, nntp_service, oidc);

    // Create router
    let app = create_router(state);

    // Log server startup info based on TLS mode
    match &config.http.tls.mode {
        TlsMode::Acme => {
            tracing::info!(
                host = %config.http.host,
                port = config.http.port,
                domains = ?config.http.tls.acme_domains,
                "Starting HTTPS server with ACME (Let's Encrypt)"
            );
        }
        TlsMode::Manual => {
            tracing::info!(
                host = %config.http.host,
                port = config.http.port,
                cert = config.http.tls.cert_path.as_deref().unwrap_or(""),
                "Starting HTTPS server with manual certificates"
            );
        }
        TlsMode::None => {
            tracing::info!(
                "Starting server at http://{}:{}",
                config.http.host,
                config.http.port
            );
        }
    }

    // Start server using the http module
    http::start_server(app, &config).await?;

    Ok(())
}
