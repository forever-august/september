//! HTTP to HTTPS redirect server.
//!
//! Spawns a lightweight HTTP server on port 80 (or configured port) that redirects
//! all requests to HTTPS.

use std::net::SocketAddr;

use axum::http::{StatusCode, Uri};
use axum::response::Redirect;
use axum::routing::any;
use axum::Router;
use axum_extra::extract::Host;

/// Spawn an HTTP server that redirects all requests to HTTPS.
///
/// This runs in the background and does not block.
pub fn spawn_redirect_server(http_port: u16, https_port: u16) {
    tokio::spawn(async move {
        let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
        
        tracing::info!(
            http_port = %http_port,
            https_port = %https_port,
            "Starting HTTP->HTTPS redirect server"
        );

        let app = Router::new().fallback(any(move |Host(host): Host, uri: Uri| async move {
            redirect_to_https(host, uri, https_port)
        }));

        match axum_server::bind(addr)
            .serve(app.into_make_service())
            .await
        {
            Ok(()) => {
                tracing::debug!("HTTP redirect server stopped");
            }
            Err(e) => {
                tracing::error!(error = %e, "HTTP redirect server failed");
            }
        }
    });
}

/// Generate a redirect response from HTTP to HTTPS.
fn redirect_to_https(host: String, uri: Uri, https_port: u16) -> Result<Redirect, StatusCode> {
    // Remove port from host if present
    let host_without_port = host.split(':').next().unwrap_or(&host);

    // Build HTTPS URL
    let https_url = if https_port == 443 {
        format!("https://{}{}", host_without_port, uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/"))
    } else {
        format!("https://{}:{}{}", host_without_port, https_port, uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/"))
    };

    tracing::debug!(from = %uri, to = %https_url, "Redirecting HTTP to HTTPS");

    Ok(Redirect::permanent(&https_url))
}
