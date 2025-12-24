//! Request ID middleware for correlating logs with requests.
//!
//! Generates a UUID v4 for each incoming request and creates a tracing span
//! that wraps the entire request lifecycle. All logs emitted during request
//! processing will include the request_id field for correlation.

use std::time::Instant;

use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use tracing::Instrument;
use uuid::Uuid;

/// Extension type for accessing request ID in handlers if needed.
/// The inner Uuid can be extracted from request extensions when needed.
#[derive(Clone, Debug)]
pub struct RequestId(pub Uuid);

/// Middleware that generates a request ID and creates a request span.
///
/// This should be the outermost middleware layer so the span wraps
/// all request processing, including other middleware and handlers.
pub async fn request_id_layer(request: Request, next: Next) -> Response {
    let request_id = Uuid::new_v4();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path();

    // Create the request span with key fields for correlation
    let span = tracing::info_span!(
        "request",
        request_id = %request_id,
        method = %method,
        path = %path,
        duration_ms = tracing::field::Empty,
    );

    let start = Instant::now();

    // Add request ID to extensions for access in handlers if needed
    let mut request = request;
    request.extensions_mut().insert(RequestId(request_id));

    // Process the request within the span
    async move {
        let response = next.run(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Record duration and log completion with status code
        tracing::Span::current().record("duration_ms", duration_ms);
        tracing::info!(
            status = response.status().as_u16(),
            duration_ms,
            "Request completed"
        );

        response
    }
    .instrument(span)
    .await
}
