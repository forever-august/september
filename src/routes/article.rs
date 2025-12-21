use axum::{
    extract::{Path, State},
    response::Html,
};
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ViewPath {
    pub group: String,
    pub message_id: String,
}

pub async fn view(
    State(state): State<AppState>,
    Path(path): Path<ViewPath>,
) -> Result<Html<String>, AppError> {
    // Fetch article (cached + coalesced)
    let article = state
        .nntp
        .get_article(&path.message_id)
        .await
        .map_err(|_| AppError::ArticleNotFound(path.message_id.clone()))?;

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &path.group);
    context.insert("article", &article);

    let html = state.tera.render("article/view.html", &context)?;
    Ok(Html(html))
}
