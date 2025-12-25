//! Handlers for thread listing and thread viewing.
//!
//! Supports pagination for both thread lists and article comments.

use axum::{
    extract::{Path, Query, State},
    response::Html,
    Extension,
};
use serde::Deserialize;
use tracing::instrument;

use crate::error::{AppError, AppErrorResponse, ResultExt};
use crate::middleware::{CurrentUser, RequestId};
use crate::state::AppState;

/// Query parameters for thread list pagination.
#[derive(Deserialize)]
pub struct ListParams {
    pub page: Option<usize>,
}

/// Handler for paginated thread list in a newsgroup.
#[instrument(
    name = "threads::list",
    skip(state, params, request_id, current_user),
    fields(group = %group)
)]
pub async fn list(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
    Path(group): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<Html<String>, AppErrorResponse> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = state.config.nntp.defaults.threads_per_page;

    // Fetch paginated threads
    let (threads, pagination) = state
        .nntp
        .get_threads_paginated(&group, page, per_page)
        .await
        .with_request_id(&request_id)?;

    // Fetch and cache group stats (article count and last article date)
    // This runs in the background so it doesn't block page load
    let nntp = state.nntp.clone();
    let group_name = group.clone();
    tokio::spawn(async move {
        let _ = nntp.get_group_stats(&group_name).await;
    });

    // Check if user can post to this group
    let can_post = if current_user.0.as_ref().map(|u| u.email.is_some()).unwrap_or(false) {
        state.nntp.can_post_to_group(&group).await
    } else {
        false
    };

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &group);
    context.insert("threads", &threads);
    context.insert("pagination", &pagination);
    context.insert("can_post", &can_post);
    
    // Auth context for header
    context.insert("oidc_enabled", &state.oidc.is_some());
    if let Some(user) = current_user.0.as_ref() {
        context.insert("user", &serde_json::json!({
            "display_name": user.display_name(),
        }));
    }

    let html = state
        .tera
        .render("threads/list.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}

/// Path parameters for thread view (group and message_id).
#[derive(Debug, Deserialize)]
pub struct ViewPath {
    pub group: String,
    pub message_id: String,
}

/// Query parameters for thread view pagination.
#[derive(Deserialize)]
pub struct ViewParams {
    pub page: Option<usize>,
}

/// Handler for viewing a thread with paginated comments.
#[instrument(
    name = "threads::view",
    skip(state, params, request_id, current_user),
    fields(group = %path.group, message_id = %path.message_id)
)]
pub async fn view(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
    Path(path): Path<ViewPath>,
    Query(params): Query<ViewParams>,
) -> Result<Html<String>, AppErrorResponse> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = state.config.nntp.defaults.articles_per_page;
    let collapse_threshold = state.config.ui.collapse_threshold;

    // Fetch thread with paginated article bodies
    let (thread, comments, pagination) = state
        .nntp
        .get_thread_paginated(&path.group, &path.message_id, page, per_page, collapse_threshold)
        .await
        .map_err(|_| AppError::ArticleNotFound(path.message_id.clone()))
        .with_request_id(&request_id)?;

    // Check if user can post to this group
    let can_post = if current_user.0.as_ref().map(|u| u.email.is_some()).unwrap_or(false) {
        state.nntp.can_post_to_group(&path.group).await
    } else {
        false
    };

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &path.group);
    context.insert("thread", &thread);
    context.insert("comments", &comments);
    context.insert("pagination", &pagination);
    context.insert("can_post", &can_post);

    // Auth context for header
    context.insert("oidc_enabled", &state.oidc.is_some());
    if let Some(user) = current_user.0.as_ref() {
        context.insert("user", &serde_json::json!({
            "display_name": user.display_name(),
        }));
        // Include CSRF token for reply forms
        context.insert("csrf_token", &user.csrf_token);
    }

    let html = state
        .tera
        .render("threads/view.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}
