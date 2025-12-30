//! Privacy policy page handler.

use axum::{extract::State, response::Html, Extension};
use tracing::instrument;

use super::insert_auth_context;
use crate::error::{AppError, AppErrorResponse, ResultExt};
use crate::middleware::{CurrentUser, RequestId};
use crate::state::AppState;

/// Privacy policy page handler.
#[instrument(name = "privacy::privacy", skip(state, request_id, current_user))]
pub async fn privacy(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(current_user): Extension<CurrentUser>,
) -> Result<Html<String>, AppErrorResponse> {
    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);

    insert_auth_context(&mut context, &state, &current_user, false);

    let html = state
        .tera
        .render("privacy.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}
