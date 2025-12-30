//! HTTP server module with TLS support.
//!
//! This module provides HTTPS server functionality with three modes:
//! - **ACME (default)**: Automatic certificate provisioning via Let's Encrypt
//! - **Manual**: User-provided certificate and key files
//! - **None**: Plain HTTP (explicit opt-out for development/reverse proxy)
//!
//! The server includes:
//! - HTTP to HTTPS redirect (when TLS enabled)
//! - Graceful shutdown on SIGTERM/SIGINT
//! - Certificate hot-reload via SIGHUP (manual mode)

mod redirect;
mod server;
mod shutdown;

pub use server::start_server;
