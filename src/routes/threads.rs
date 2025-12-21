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
    let page = params.page.unwrap_or(1).max(1);
    let per_page = state.config.nntp.defaults.threads_per_page;

    // Fetch paginated threads
    let (threads, pagination) = state
        .nntp
        .get_threads_paginated(&group, page, per_page)
        .await?;

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &group);
    context.insert("threads", &threads);
    context.insert("pagination", &pagination);

    let html = state.tera.render("threads/list.html", &context)?;
    Ok(Html(html))
}

#[derive(Deserialize)]
pub struct ViewPath {
    pub group: String,
    pub message_id: String,
}

#[derive(Deserialize)]
pub struct ViewParams {
    pub page: Option<usize>,
}

pub async fn view(
    State(state): State<AppState>,
    Path(path): Path<ViewPath>,
    Query(params): Query<ViewParams>,
) -> Result<Html<String>, AppError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = state.config.nntp.defaults.articles_per_page;
    let collapse_threshold = state.config.ui.collapse_threshold;

    // Fetch thread with paginated article bodies
    let (thread, comments, pagination) = state
        .nntp
        .get_thread_paginated(&path.group, &path.message_id, page, per_page, collapse_threshold)
        .await
        .map_err(|_| AppError::ArticleNotFound(path.message_id.clone()))?;

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &path.group);
    context.insert("thread", &thread);
    context.insert("comments", &comments);
    context.insert("pagination", &pagination);

    let html = state.tera.render("threads/view.html", &context)?;
    Ok(Html(html))
}
