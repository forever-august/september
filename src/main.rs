//! September: an NNTP web interface.
//!
//! This is the application entry point. It initializes tracing, loads configuration
//! from TOML files, creates the NNTP federated service, spawns worker connections,
//! sets up the Axum router with all routes, and starts the HTTP server.

mod config;
mod error;
mod middleware;
mod nntp;
mod routes;
mod state;
mod templates;

use std::net::SocketAddr;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::{AppConfig, DEFAULT_CONFIG_PATH, DEFAULT_LOG_FILTER};

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
use nntp::NntpFederatedService;
use routes::create_router;
use state::AppState;
use templates::init_templates;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let log_format = args
        .log_format
        .as_deref()
        .unwrap_or(&config.logging.format);

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

    // Initialize Tera templates
    let tera = init_templates()?;
    tracing::info!("Initialized templates");

    // Initialize federated NNTP service with caching and worker pools
    let nntp_service = NntpFederatedService::new(&config);
    nntp_service.spawn_workers();
    tracing::info!(
        servers = ?nntp_service.server_names(),
        "Initialized federated NNTP service"
    );

    // Create application state
    let state = AppState::new(config.clone(), tera, nntp_service);

    // Create router
    let app = create_router(state);

    // Start server
    let addr: SocketAddr = format!("{}:{}", config.http.host, config.http.port)
        .parse()
        .expect("Invalid http.host or http.port in config");
    tracing::info!("Starting server at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
