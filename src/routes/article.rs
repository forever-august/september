//! Handler for viewing a single article by message-id.
//!
//! Used for direct article links independent of thread context.

use axum::{
    extract::{Path, Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ViewPath {
    pub message_id: String,
}

#[derive(Deserialize)]
pub struct ViewParams {
    pub back: Option<String>,
}

/// Fetches and displays a single article.
pub async fn view(
    State(state): State<AppState>,
    Path(path): Path<ViewPath>,
    Query(params): Query<ViewParams>,
) -> Result<Html<String>, AppError> {
    // Fetch article (cached + coalesced)
    let article = state
        .nntp
        .get_article(&path.message_id)
        .await
        .map_err(|_| AppError::ArticleNotFound(path.message_id.clone()))?;

    // Determine back link based on query param
    let (back_url, back_label) = match &params.back {
        Some(back) => (back.clone(), extract_back_label(back)),
        None => ("/".to_string(), "Back".to_string()),
    };

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("article", &article);
    context.insert("back_url", &back_url);
    context.insert("back_label", &back_label);

    let html = state.tera.render("article/view.html", &context)?;
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
