//! Privacy policy page handler.

use axum::{extract::State, response::Html, Extension};
use tracing::instrument;

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

    // Auth context for header
    context.insert("oidc_enabled", &state.oidc.is_some());
    if let Some(user) = current_user.0.as_ref() {
        context.insert(
            "user",
            &serde_json::json!({
                "display_name": user.display_name(),
            }),
        );
    }

    let html = state
        .tera
        .render("privacy.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;
    Ok(Html(html))
}
