//! September: an NNTP web interface.
//!
//! This is the application entry point. It initializes tracing, loads configuration
//! from TOML files, creates the NNTP federated service, spawns worker connections,
//! sets up the Axum router with all routes, and starts the HTTP server.

mod config;
mod error;
mod nntp;
mod routes;
mod state;
mod templates;

use std::net::SocketAddr;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::{AppConfig, DEFAULT_CONFIG_PATH, DEFAULT_LOG_FILTER};
use nntp::NntpFederatedService;
use routes::create_router;
use state::AppState;
use templates::init_templates;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG_FILTER.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
    let mut config = AppConfig::load(&config_path)?;

    // Default site_name to first server name if not configured
    if config.ui.site_name.is_none() {
        config.ui.site_name = config.server.first().map(|s| s.name.clone());
    }

    tracing::info!("Loaded configuration");

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
