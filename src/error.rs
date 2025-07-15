//! Error types for the September application

use thiserror::Error;

/// Main error type for the application
#[derive(Error, Debug)]
pub enum SeptemberError {
    #[error("NNTP connection error: {0}")]
    NntpConnection(String),

    #[error("NNTP protocol error: {0}")]
    NntpProtocol(String),

    #[error("HTTP server error: {0}")]
    HttpServer(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

/// Result type alias for convenience
pub type Result<T> = std::result::Result<T, SeptemberError>;
