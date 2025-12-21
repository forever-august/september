mod config;
mod error;
mod nntp;
mod routes;
mod state;
mod templates;

use std::net::SocketAddr;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::AppConfig;
use nntp::NntpService;
use routes::create_router;
use state::AppState;
use templates::init_templates;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "september=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config/default.toml".to_string());
    let mut config = AppConfig::load(&config_path)?;

    // Default site_name to NNTP server if not configured
    if config.ui.site_name.is_none() {
        config.ui.site_name = Some(config.nntp.server.clone());
    }

    tracing::info!("Loaded configuration");
    tracing::info!("NNTP server: {}:{}", config.nntp.server, config.nntp.port);

    // Initialize Tera templates
    let tera = init_templates()?;
    tracing::info!("Initialized templates");

    // Initialize NNTP service with caching and worker pool
    let nntp_service = NntpService::new(&config);
    let worker_count = config.nntp.worker_count.unwrap_or(4);
    nntp_service.spawn_workers(worker_count);
    tracing::info!("Initialized NNTP service with {} workers", worker_count);

    // Create application state
    let state = AppState::new(config.clone(), tera, nntp_service);

    // Create router
    let app = create_router(state);

    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server.port));
    tracing::info!("Starting server at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
