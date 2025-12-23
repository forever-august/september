//! HTTP route handlers for the web interface.
//!
//! Routes are organized by content type, with per-route Cache-Control headers.
//! Immutable content (articles) uses longer cache durations, while dynamic
//! content (thread lists) uses shorter durations.

pub mod article;
pub mod home;
pub mod threads;

use axum::{routing::get, Router};
use http::header::{HeaderValue, CACHE_CONTROL};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

use crate::config::{
    CACHE_CONTROL_ARTICLE, CACHE_CONTROL_HOME, CACHE_CONTROL_STATIC, CACHE_CONTROL_THREAD_LIST,
    CACHE_CONTROL_THREAD_VIEW, STATIC_DIR,
};
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

    Router::new()
        .merge(article_routes)
        .merge(thread_view_routes)
        .merge(thread_list_routes)
        .merge(home_routes)
        .merge(static_routes)
        .with_state(state)
}
