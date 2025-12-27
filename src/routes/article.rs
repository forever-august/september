//! Handler for viewing a single article by message-id.
//!
//! Used for direct article links independent of thread context.

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

#[derive(Debug, Deserialize)]
pub struct ViewPath {
    pub message_id: String,
}

#[derive(Deserialize)]
pub struct ViewParams {
    pub back: Option<String>,
}

/// Fetches and displays a single article.
#[instrument(
    name = "article::view",
    skip(state, params, request_id, current_user),
    fields(message_id = %path.message_id)
)]
pub async fn view(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
    Path(path): Path<ViewPath>,
    Query(params): Query<ViewParams>,
) -> Result<Html<String>, AppErrorResponse> {
    // Fetch article (cached + coalesced)
    let article = state
        .nntp
        .get_article(&path.message_id)
        .await
        .with_request_id(&request_id)?;

    // Determine back link based on query param
    let (back_url, back_label, group) = match &params.back {
        Some(back) => {
            let label = extract_back_label(back);
            let group = extract_group_from_back(back);
            (back.clone(), label, group)
        }
        None => ("/".to_string(), "Back".to_string(), None),
    };

    // Check if user can post (needs group and email)
    let can_post = if let Some(ref g) = group {
        if current_user.0.as_ref().map(|u| u.email.is_some()).unwrap_or(false) {
            state.nntp.can_post_to_group(g).await
        } else {
            false
        }
    } else {
        false
    };

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("article", &article);
    context.insert("back_url", &back_url);
    context.insert("back_label", &back_label);
    context.insert("can_post", &can_post);
    if let Some(ref g) = group {
        context.insert("group", g);
    }

    // Auth context for header
    context.insert("oidc_enabled", &state.oidc.is_some());
    if let Some(user) = current_user.0.as_ref() {
        context.insert("user", &serde_json::json!({
            "display_name": user.display_name(),
        }));
    }

    let html = state
        .tera
        .render("article/view.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}

/// Extract a human-readable label from the back URL
fn extract_back_label(back: &str) -> String {
    if back.starts_with("/g/") {
        let parts: Vec<&str> = back.split('/').collect();
        if parts.len() >= 3 {
            let group = parts[2];
            if parts.len() >= 5 && parts[3] == "thread" {
                // /g/{group}/thread/{message_id} -> "Back to thread"
                return "Back to thread".to_string();
            }
            // /g/{group} -> "Back to {group}"
            return format!("Back to {}", group);
        }
    }
    "Back".to_string()
}

/// Extract group name from the back URL if present
fn extract_group_from_back(back: &str) -> Option<String> {
    if back.starts_with("/g/") {
        let parts: Vec<&str> = back.split('/').collect();
        if parts.len() >= 3 {
            return Some(parts[2].to_string());
        }
    }
    None
}
