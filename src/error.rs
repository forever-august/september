//! Application error types and their mapping to HTTP responses.
//!
//! Defines `AppError` variants for different failure modes and implements
//! `IntoResponse` to convert errors into appropriate HTTP status codes and
//! user-friendly error pages.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use http::header::CACHE_CONTROL;
use std::io;

use crate::config::CACHE_CONTROL_ERROR;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// NNTP server connection or protocol errors.
    #[error("NNTP connection error: {0}")]
    NntpConnection(#[from] nntp_rs::Error),

    /// Tera template rendering errors.
    #[error("Template rendering error: {0}")]
    Template(#[from] tera::Error),

    /// Requested article does not exist.
    #[error("Article not found: {0}")]
    ArticleNotFound(String),

    /// File system or I/O errors.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Catch-all for unexpected errors.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::ArticleNotFound(_) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            AppError::NntpConnection(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "NNTP server unavailable".to_string(),
            ),
            _ => {
                tracing::error!("Internal error: {:?}", self);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let body = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>Error {}</title>
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <div class="container">
        <div class="error-page">
            <h1>Error {}</h1>
            <p>{}</p>
            <a href="/">Return to homepage</a>
        </div>
    </div>
</body>
</html>"#,
            status.as_u16(),
            status.as_u16(),
            message
        );

        (status, [(CACHE_CONTROL, CACHE_CONTROL_ERROR)], Html(body)).into_response()
    }
}
