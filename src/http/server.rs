//! HTTP/HTTPS server startup logic.
//!
//! Supports three TLS modes:
//! - ACME: Automatic Let's Encrypt certificates
//! - Manual: User-provided certificate files
//! - None: Plain HTTP

use std::net::SocketAddr;

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use futures::StreamExt;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig;

use crate::config::{AppConfig, TlsMode};

use super::redirect;
use super::shutdown;

/// Server startup error
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("Failed to bind server: {0}")]
    Bind(#[from] std::io::Error),

    #[error("Failed to load TLS configuration: {0}")]
    TlsConfig(String),

    #[error("Server error: {0}")]
    Server(String),
}

/// Start the HTTP/HTTPS server based on configuration.
///
/// This function blocks until the server shuts down.
pub async fn start_server(app: Router, config: &AppConfig) -> Result<(), ServerError> {
    let addr: SocketAddr = format!("{}:{}", config.http.host, config.http.port)
        .parse()
        .map_err(|e| ServerError::TlsConfig(format!("Invalid http.host or http.port: {}", e)))?;

    let handle = Handle::new();

    match &config.http.tls.mode {
        TlsMode::None => {
            tracing::warn!(
                "TLS disabled - server running on plain HTTP (not recommended for production)"
            );
            start_plain_server(app, addr, handle).await
        }
        TlsMode::Manual => {
            let cert_path = config.http.tls.cert_path.as_ref().unwrap();
            let key_path = config.http.tls.key_path.as_ref().unwrap();
            start_manual_tls_server(app, addr, cert_path, key_path, &config.http.tls, handle).await
        }
        TlsMode::Acme => {
            start_acme_server(app, addr, &config.http.tls, handle).await
        }
    }
}

/// Start a plain HTTP server (no TLS).
async fn start_plain_server(
    app: Router,
    addr: SocketAddr,
    handle: Handle,
) -> Result<(), ServerError> {
    tracing::info!(%addr, "Starting HTTP server (no TLS)");

    // Setup graceful shutdown
    shutdown::setup_shutdown_handler(handle.clone());

    axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .map_err(|e| ServerError::Server(e.to_string()))
}

/// Start HTTPS server with user-provided certificates.
async fn start_manual_tls_server(
    app: Router,
    addr: SocketAddr,
    cert_path: &str,
    key_path: &str,
    tls_config: &crate::config::TlsConfig,
    handle: Handle,
) -> Result<(), ServerError> {
    tracing::info!(%addr, cert = %cert_path, key = %key_path, "Starting HTTPS server (manual certs)");

    // Load TLS configuration
    let rustls_config = RustlsConfig::from_pem_file(cert_path, key_path)
        .await
        .map_err(|e| ServerError::TlsConfig(format!("Failed to load certificates: {}", e)))?;

    // Setup graceful shutdown
    shutdown::setup_shutdown_handler(handle.clone());

    // Setup SIGHUP handler for certificate reload
    shutdown::setup_reload_handler(rustls_config.clone(), cert_path.to_string(), key_path.to_string());

    // Start HTTP->HTTPS redirect if enabled
    if tls_config.redirect_http {
        redirect::spawn_redirect_server(tls_config.redirect_port, addr.port());
    }

    axum_server::bind_rustls(addr, rustls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .map_err(|e| ServerError::Server(e.to_string()))
}

/// Start HTTPS server with automatic ACME (Let's Encrypt) certificates.
async fn start_acme_server(
    app: Router,
    addr: SocketAddr,
    tls_config: &crate::config::TlsConfig,
    handle: Handle,
) -> Result<(), ServerError> {
    let domains = tls_config.acme_domains.clone();
    let email = tls_config.acme_email.clone().unwrap();
    let cache_dir = tls_config.acme_cache_dir.clone();
    let production = tls_config.acme_production;
    let redirect_http = tls_config.redirect_http;
    let redirect_port = tls_config.redirect_port;

    let env_name = if production { "production" } else { "staging" };
    tracing::info!(
        %addr,
        domains = ?domains,
        email = %email,
        cache = %cache_dir,
        environment = %env_name,
        "Starting HTTPS server (ACME)"
    );

    if !production {
        tracing::warn!(
            "Using Let's Encrypt staging environment - certificates will NOT be trusted by browsers. \
             Set acme_production = true for production use."
        );
    }

    // Create cache directory if it doesn't exist
    std::fs::create_dir_all(&cache_dir).map_err(|e| {
        ServerError::TlsConfig(format!("Failed to create ACME cache directory '{}': {}", cache_dir, e))
    })?;

    // Configure ACME
    let mut acme_state = AcmeConfig::new(domains)
        .contact_push(format!("mailto:{}", email))
        .cache(DirCache::new(cache_dir))
        .directory_lets_encrypt(production)
        .state();

    let acceptor = acme_state.axum_acceptor(acme_state.default_rustls_config());

    // Spawn ACME event loop for certificate renewal
    tokio::spawn(async move {
        loop {
            match acme_state.next().await {
                Some(Ok(event)) => {
                    tracing::info!(event = ?event, "ACME event");
                }
                Some(Err(err)) => {
                    tracing::error!(error = %err, "ACME error");
                }
                None => {
                    tracing::debug!("ACME state stream ended");
                    break;
                }
            }
        }
    });

    // Setup graceful shutdown
    shutdown::setup_shutdown_handler(handle.clone());

    // Start HTTP->HTTPS redirect if enabled
    if redirect_http {
        redirect::spawn_redirect_server(redirect_port, addr.port());
    }

    axum_server::bind(addr)
        .handle(handle)
        .acceptor(acceptor)
        .serve(app.into_make_service())
        .await
        .map_err(|e| ServerError::Server(e.to_string()))
}
