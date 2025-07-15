//! September HTTP to NNTP Bridge Server
//!
//! Main server binary for the HTTP to NNTP bridge.

use axum::{response::Html, routing::get, Router};
use leptos::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use september::app::App;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting September HTTP to NNTP Bridge Server");

    // Build our application with routes
    let conf = get_configuration(Some("Cargo.toml")).await.unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;

    // Generate the list of routes in your Leptos App
    let routes = generate_route_list(App);

    let app = Router::new()
        .leptos_routes(&leptos_options, routes, App)
        .route("/api/health", get(health_check))
        .fallback(file_and_error_handler)
        .layer(ServiceBuilder::new().layer(CorsLayer::permissive()))
        .with_state(leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", &addr);

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn health_check() -> Html<&'static str> {
    Html("<h1>September Server is running!</h1>")
}

async fn file_and_error_handler(
    uri: axum::http::Uri,
) -> axum::response::Result<axum::response::Response> {
    use axum::response::IntoResponse;

    let path = uri.path();
    warn!("File not found: {}", path);

    let body = format!("File not found: {}", path);
    Ok(axum::response::Html(body).into_response())
}
