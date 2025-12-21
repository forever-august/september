use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListParams {
    pub page: Option<usize>,
}

pub async fn list(
    State(state): State<AppState>,
    Path(group): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Html<String>, AppError> {
    let page = params.page.unwrap_or(1);
    let threads_per_page = state.config.nntp.defaults.threads_per_page as u64;

    // Fetch threads (cached + coalesced)
    let threads = state.nntp.get_threads(&group, threads_per_page).await?;

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &group);
    context.insert("threads", &threads);
    context.insert("page", &page);

    let html = state.tera.render("threads/list.html", &context)?;
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct ViewPath {
    pub group: String,
    pub message_id: String,
}

pub async fn view(
    State(state): State<AppState>,
    Path(path): Path<ViewPath>,
) -> Result<Html<String>, AppError> {
    // Fetch the full thread with all replies (cached + coalesced)
    let thread = state
        .nntp
        .get_thread(&path.group, &path.message_id)
        .await
        .map_err(|_| AppError::ArticleNotFound(path.message_id.clone()))?;

    // Flatten the thread tree for non-recursive template rendering
    let collapse_threshold = state.config.ui.collapse_threshold;
    let comments = thread.root.flatten(collapse_threshold);

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &path.group);
    context.insert("thread", &thread);
    context.insert("comments", &comments);

    let html = state.tera.render("threads/view.html", &context)?;
    Ok(Html(html))
}
