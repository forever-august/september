//! Request ID and authentication middleware.
//!
//! Provides:
//! - Request ID generation for log correlation
//! - Session extraction and refresh (sliding window)

use std::time::Duration;
use std::time::Instant;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use http::header::SET_COOKIE;
use time::Duration as TimeDuration;

use crate::oidc::session::{cookie_names, User};
use crate::state::AppState;
use tracing::Instrument;
use uuid::Uuid;

/// Extension type for accessing request ID in handlers if needed.
/// The inner Uuid can be extracted from request extensions when needed.
#[derive(Clone, Debug)]
pub struct RequestId(pub Uuid);

/// Extension type for accessing the current authenticated user.
/// Extracted from session cookie by auth_layer middleware.
#[derive(Clone, Debug)]
pub struct CurrentUser(pub Option<User>);

impl CurrentUser {
    /// Get a reference to the user, if authenticated
    pub fn user(&self) -> Option<&User> {
        self.0.as_ref()
    }
}

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

/// Middleware that extracts user session from signed cookie.
///
/// This reads the session cookie, validates it, injects CurrentUser into
/// request extensions, and optionally refreshes the session (sliding window).
pub async fn auth_layer(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    let session_lifetime = state
        .oidc
        .as_ref()
        .map(|o| o.session_lifetime())
        .unwrap_or(Duration::from_secs(30 * 24 * 60 * 60)); // 30 days default

    let (user, needs_refresh) = extract_user_from_cookie(&jar, session_lifetime);

    // Insert user into request extensions
    request.extensions_mut().insert(CurrentUser(user.clone()));

    // Process the request
    let response = next.run(request).await;

    // If session needs refresh, update the cookie
    if let (Some(mut user), true) = (user, needs_refresh) {
        user.refresh(session_lifetime);

        if let Ok(user_json) = serde_json::to_string(&user) {
            let session_cookie = Cookie::build((cookie_names::SESSION, user_json))
                .path("/")
                .http_only(true)
                .same_site(SameSite::Lax)
                .max_age(TimeDuration::seconds(session_lifetime.as_secs() as i64))
                .build();

            let jar = jar.add(session_cookie);

            // Merge the Set-Cookie header into the response
            let (mut parts, body) = response.into_parts();
            for cookie in jar.iter() {
                if let Ok(value) = cookie.to_string().parse() {
                    parts.headers.append(SET_COOKIE, value);
                }
            }
            return Response::from_parts(parts, body);
        }
    }

    response
}

/// Extract and validate user from session cookie.
/// Returns (user, needs_refresh) tuple.
fn extract_user_from_cookie(
    jar: &PrivateCookieJar,
    session_lifetime: Duration,
) -> (Option<User>, bool) {
    let cookie = match jar.get(cookie_names::SESSION) {
        Some(c) => c,
        None => return (None, false),
    };

    let user: User = match serde_json::from_str(cookie.value()) {
        Ok(u) => u,
        Err(_) => return (None, false),
    };

    // Check if session has expired
    if user.is_expired() {
        return (None, false);
    }

    // Check if session should be refreshed (sliding window)
    let needs_refresh = user.should_refresh(session_lifetime);

    (Some(user), needs_refresh)
}
