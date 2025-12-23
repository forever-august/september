//! Application error types and their mapping to HTTP responses.
//!
//! Defines `AppError` variants for different failure modes and implements
//! `IntoResponse` to convert errors into appropriate HTTP status codes and
//! user-friendly error pages.
//!
//! Error responses include a request ID reference that users can cite when
//! reporting issues. The ID is displayed in a short form (first 8 chars)
//! with the full UUID available in the title attribute for copying.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use http::header::CACHE_CONTROL;
use std::io;
use uuid::Uuid;

use crate::config::CACHE_CONTROL_ERROR;
use crate::middleware::RequestId;

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

/// Response type that includes request ID for error correlation.
///
/// This wraps an AppError with an optional request ID that gets included
/// in error responses so users can reference it when reporting issues.
pub struct AppErrorResponse {
    pub error: AppError,
    pub request_id: Option<Uuid>,
}

impl AppErrorResponse {
    pub fn new(error: AppError, request_id: Option<Uuid>) -> Self {
        Self { error, request_id }
    }
}

impl From<AppError> for AppErrorResponse {
    fn from(error: AppError) -> Self {
        Self {
            error,
            request_id: None,
        }
    }
}

/// Extension trait for adding request ID context to Results.
///
/// This allows handlers to easily attach request IDs to errors:
/// ```ignore
/// pub async fn handler(
///     Extension(request_id): Extension<RequestId>,
///     // ...
/// ) -> Result<Html<String>, AppErrorResponse> {
///     some_fallible_operation().with_request_id(&request_id)?;
///     Ok(Html("...".into()))
/// }
/// ```
pub trait ResultExt<T> {
    /// Converts a `Result<T, AppError>` to `Result<T, AppErrorResponse>`,
    /// attaching the given request ID to any error.
    fn with_request_id(self, request_id: &RequestId) -> Result<T, AppErrorResponse>;
}

impl<T> ResultExt<T> for Result<T, AppError> {
    fn with_request_id(self, request_id: &RequestId) -> Result<T, AppErrorResponse> {
        self.map_err(|error| AppErrorResponse::new(error, Some(request_id.0)))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        AppErrorResponse::from(self).into_response()
    }
}

impl IntoResponse for AppErrorResponse {
    fn into_response(self) -> Response {
        let (status, message) = match &self.error {
            AppError::ArticleNotFound(_) => (StatusCode::NOT_FOUND, self.error.to_string()),
            AppError::NntpConnection(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "NNTP server unavailable".to_string(),
            ),
            _ => {
                tracing::error!("Internal error: {:?}", self.error);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let request_id_section = match self.request_id {
            Some(id) => {
                let full_id = id.to_string();
                let short_id = &full_id[..8];
                format!(
                    r#"<p class="error-reference">Error Reference: <code title="{}">{}</code></p>"#,
                    full_id, short_id
                )
            }
            None => String::new(),
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
            {}
            <a href="/">Return to homepage</a>
        </div>
    </div>
</body>
</html>"#,
            status.as_u16(),
            status.as_u16(),
            message,
            request_id_section
        );

        (status, [(CACHE_CONTROL, CACHE_CONTROL_ERROR)], Html(body)).into_response()
    }
}
