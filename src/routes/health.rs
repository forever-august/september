//! Health check endpoint for container orchestration.
//!
//! Provides a simple liveness probe that returns 200 OK when the process is running.
//! Used by Kubernetes, ECS, systemd, and load balancers to verify the service is alive.

/// Health check handler.
///
/// Returns a simple "ok" response to indicate the service is running.
/// This is a liveness probe - it only checks that the process can respond to HTTP.
pub async fn health() -> &'static str {
    "ok"
}
