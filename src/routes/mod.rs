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
pub mod home;
pub mod post;
pub mod privacy;
pub mod threads;

use axum::{middleware, routing::{get, post}, Router};
use http::header::{HeaderValue, CACHE_CONTROL};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

use crate::config::{
    CACHE_CONTROL_ARTICLE, CACHE_CONTROL_HOME, CACHE_CONTROL_STATIC, CACHE_CONTROL_THREAD_LIST,
    CACHE_CONTROL_THREAD_VIEW, STATIC_DIR,
};
use crate::middleware::{auth_layer, request_id_layer};
use crate::state::AppState;

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
    let thread_list_routes = Router::new()
        .route("/g/{group}", get(threads::list))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_THREAD_LIST),
        ));

    // Home/browse - moderate cache
    let home_routes = Router::new()
        .route("/", get(home::index))
        .route("/browse/{*prefix}", get(home::browse))
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_HOME),
        ));

    // Static files - long cache with immutable hint
    let static_routes = Router::new()
        .nest_service("/static", ServeDir::new(STATIC_DIR))
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

    Router::new()
        .merge(article_routes)
        .merge(thread_view_routes)
        .merge(thread_list_routes)
        .merge(home_routes)
        .merge(auth_routes)
        .merge(post_routes)
        .merge(privacy_routes)
        .merge(static_routes)
        .with_state(state.clone())
        // Auth layer - extracts user from session cookie and handles session refresh
        .layer(middleware::from_fn_with_state(state, auth_layer))
        // Request ID middleware - creates root span with request_id for correlation
        .layer(middleware::from_fn(request_id_layer))
}
