use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("NNTP connection error: {0}")]
    NntpConnection(#[from] nntp_rs::Error),

    #[error("Template rendering error: {0}")]
    Template(#[from] tera::Error),

    #[error("Newsgroup not found: {0}")]
    GroupNotFound(String),

    #[error("Article not found: {0}")]
    ArticleNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::GroupNotFound(_) | AppError::ArticleNotFound(_) => {
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

        (status, Html(body)).into_response()
    }
}
