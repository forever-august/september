pub mod article;
pub mod home;
pub mod threads;

use axum::{routing::get, Router};
use tower_http::services::ServeDir;

use crate::state::AppState;

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home::index))
        .route("/browse/{*prefix}", get(home::browse))
        .route("/g/{group}", get(threads::list))
        .route("/g/{group}/thread/{message_id}", get(threads::view))
        .route("/a/{message_id}", get(article::view))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
}
