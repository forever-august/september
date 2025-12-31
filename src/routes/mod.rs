//! HTTP route handlers for the web interface.
//!
//! Routes are organized by content type, with per-route Cache-Control headers.
//! Immutable content (articles) uses longer cache durations, while dynamic
//! content (thread lists) uses shorter durations.
//!
//! Request tracing is enabled via middleware that generates a unique request ID
//! for each incoming request, allowing correlation of all logs within a request.

pub mod article;
pub mod auth;
pub mod health;
pub mod home;
pub mod post;
pub mod privacy;
pub mod threads;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use http::header::{HeaderValue, CACHE_CONTROL};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::{
    CACHE_CONTROL_ARTICLE, CACHE_CONTROL_HOME, CACHE_CONTROL_STATIC, CACHE_CONTROL_THREAD_LIST,
    CACHE_CONTROL_THREAD_VIEW,
};
use crate::http::static_files::create_static_service;
use crate::middleware::{auth_layer, request_id_layer, CurrentUser};
use crate::state::AppState;

/// Insert authentication-related context for template rendering.
///
/// This helper consolidates the common pattern of adding auth context to templates:
/// - `oidc_enabled`: Whether OIDC authentication is configured
/// - `user.display_name`: The authenticated user's display name (if logged in)
/// - `csrf_token`: CSRF token for form submissions (if `include_csrf` is true)
///
/// # Arguments
/// * `context` - The Tera template context to modify
/// * `state` - Application state containing OIDC configuration
/// * `current_user` - The current user extracted from session
/// * `include_csrf` - Whether to include CSRF token (needed for forms)
pub fn insert_auth_context(
    context: &mut tera::Context,
    state: &AppState,
    current_user: &CurrentUser,
    include_csrf: bool,
) {
    context.insert("oidc_enabled", &state.oidc.is_some());
    if let Some(user) = current_user.0.as_ref() {
        context.insert(
            "user",
            &serde_json::json!({
                "display_name": user.display_name(),
            }),
        );
        if include_csrf {
            context.insert("csrf_token", &user.csrf_token);
        }
    }
}

/// Check if the current user can post to a group.
///
/// This combines two checks:
/// 1. The user must be authenticated with a valid email address
/// 2. The group must allow posting (checked via NNTP server capabilities)
///
/// # Arguments
/// * `current_user` - The current user extracted from session
/// * `state` - Application state for NNTP service access
/// * `group` - The newsgroup name to check
///
/// # Returns
/// `true` if the user can post to the group, `false` otherwise.
pub async fn can_post_to_group(current_user: &CurrentUser, state: &AppState, group: &str) -> bool {
    if current_user
        .0
        .as_ref()
        .map(|u| u.email.is_some())
        .unwrap_or(false)
    {
        state.nntp.can_post_to_group(group).await
    } else {
        false
    }
}

/// Creates the Axum router with all routes and cache headers.
pub fn create_router(state: AppState) -> Router {
    // Articles - longest cache, content is immutable
    let article_routes = Router::new()
        .route("/a/{message_id}", get(article::view))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_ARTICLE),
        ));

    // Thread view - medium cache, may get new replies
    let thread_view_routes = Router::new()
        .route("/g/{group}/thread/{message_id}", get(threads::view))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_THREAD_VIEW),
        ));

    // Thread list - shorter cache, new threads appear regularly
    let thread_list_routes = Router::new().route("/g/{group}", get(threads::list)).layer(
        SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_THREAD_LIST),
        ),
    );

    // Home/browse - moderate cache
    let home_routes = Router::new()
        .route("/", get(home::index))
        .route("/browse/{*prefix}", get(home::browse))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_HOME),
        ));

    // Static files - long cache with immutable hint, with theme fallback
    let static_routes = Router::new()
        .nest_service("/static", create_static_service(&state.config.theme))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_STATIC),
        ));

    // Auth routes - no caching (stateful)
    let auth_routes = Router::new()
        .route("/auth/login", get(auth::login))
        .route("/auth/login/{provider}", get(auth::login_provider))
        .route("/auth/callback/{provider}", get(auth::callback))
        .route("/auth/logout", post(auth::logout));

    // Post routes - no caching (stateful)
    let post_routes = Router::new()
        .route("/g/{group}/compose", get(post::compose))
        .route("/g/{group}/post", post(post::submit))
        .route("/a/{message_id}/reply", post(post::reply));

    // Privacy policy - static content, can use home cache duration
    let privacy_routes = Router::new()
        .route("/privacy", get(privacy::privacy))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_HOME),
        ));

    // Health check - no caching, always fresh for liveness probes
    let health_routes = Router::new().route("/health", get(health::health));

    Router::new()
        .merge(article_routes)
        .merge(thread_view_routes)
        .merge(thread_list_routes)
        .merge(home_routes)
        .merge(auth_routes)
        .merge(post_routes)
        .merge(privacy_routes)
        .merge(health_routes)
        .merge(static_routes)
        .with_state(state.clone())
        // Auth layer - extracts user from session cookie and handles session refresh
        .layer(middleware::from_fn_with_state(state, auth_layer))
        // Request ID middleware - creates root span with request_id for correlation
        .layer(middleware::from_fn(request_id_layer))
}
