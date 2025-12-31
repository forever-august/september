//! Graceful shutdown and signal handling.
//!
//! Handles:
//! - SIGTERM/SIGINT: Graceful shutdown with connection draining
//! - SIGHUP: Certificate reload (manual TLS mode only)

use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;

/// Setup graceful shutdown on SIGTERM and SIGINT.
///
/// When either signal is received, the server will:
/// 1. Stop accepting new connections
/// 2. Wait for existing connections to complete
/// 3. Shutdown gracefully
pub fn setup_shutdown_handler(handle: Handle) {
    tokio::spawn(async move {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                tracing::info!("Received Ctrl+C, initiating graceful shutdown");
            }
            _ = terminate => {
                tracing::info!("Received SIGTERM, initiating graceful shutdown");
            }
        }

        // Trigger graceful shutdown
        handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
        tracing::info!(
            "Graceful shutdown initiated, waiting up to 30 seconds for connections to close"
        );
    });
}

/// Setup SIGHUP handler for certificate reload (manual TLS mode).
///
/// When SIGHUP is received, the server will reload the certificate and key
/// files from disk without restarting.
#[cfg(unix)]
pub fn setup_reload_handler(tls_config: RustlsConfig, cert_path: String, key_path: String) {
    tokio::spawn(async move {
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("Failed to install SIGHUP handler");

        loop {
            sighup.recv().await;
            tracing::info!("Received SIGHUP, reloading TLS certificates");

            match tls_config.reload_from_pem_file(&cert_path, &key_path).await {
                Ok(()) => {
                    tracing::info!(cert = %cert_path, key = %key_path, "TLS certificates reloaded successfully");
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        cert = %cert_path,
                        key = %key_path,
                        "Failed to reload TLS certificates"
                    );
                }
            }
        }
    });
}

/// No-op reload handler for non-Unix platforms.
#[cfg(not(unix))]
pub fn setup_reload_handler(_tls_config: RustlsConfig, _cert_path: String, _key_path: String) {
    tracing::warn!("Certificate hot-reload via SIGHUP not supported on this platform");
}
