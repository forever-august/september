//! Handlers for posting new articles and replies.
//!
//! Requires authentication with a valid email address.
//! Posts are submitted via NNTP POST command.
//! All post forms are protected by CSRF tokens.

use axum::{
    extract::{Path, State},
    response::{Html, Redirect},
    Extension, Form,
};
use chrono::Utc;
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

use crate::error::{AppError, AppErrorResponse, ResultExt};
use crate::middleware::{RequestId, RequireAuthWithEmail};
use crate::nntp::{compute_preview, compute_timeago, ArticleView};
use crate::state::AppState;

/// Maximum length for subject line (characters)
const MAX_SUBJECT_LENGTH: usize = 500;
/// Maximum length for message body (characters)  
const MAX_BODY_LENGTH: usize = 64000;

/// Form data for composing a new post
#[derive(Debug, Deserialize)]
pub struct ComposeForm {
    pub subject: String,
    pub body: String,
    /// CSRF token for form protection
    pub csrf_token: String,
}

/// Form data for replying to an article
#[derive(Debug, Deserialize)]
pub struct ReplyForm {
    pub body: String,
    /// Group to post to (hidden field)
    pub group: String,
    /// Subject (pre-filled with Re: original subject)
    pub subject: String,
    /// References header (Message-IDs of parent chain)
    pub references: String,
    /// CSRF token for form protection
    pub csrf_token: String,
}

/// Parameters for posting an article and updating cache
struct PostArticleParams<'a> {
    group: &'a str,
    subject: String,
    body: String,
    from: String,
    references: Option<String>,
    root_message_id: Option<&'a str>,
    parent_message_id: Option<&'a str>,
}

/// Format the From header from user info
fn format_from_header(name: Option<&str>, email: &str) -> String {
    match name {
        Some(name) => format!("{} <{}>", name, email),
        None => email.to_string(),
    }
}

/// Generate a Message-ID for a new article
fn generate_message_id(domain: &str) -> String {
    let uuid = Uuid::new_v4();
    format!("<{}.september@{}>", uuid, domain)
}

/// Get the domain from config for Message-ID generation.
/// Extracts a proper domain from site_name (e.g., "news.example.com" -> "example.com")
/// Sanitizes the result to remove spaces and other characters that NNTP servers may normalize.
fn get_domain(state: &AppState) -> String {
    state
        .config
        .ui
        .site_name
        .as_ref()
        .and_then(|s| {
            // Try to extract domain from site_name
            // e.g., "news.example.com" -> "example.com"
            // e.g., "example.com" -> "example.com"
            let parts: Vec<&str> = s.split('.').collect();
            if parts.len() >= 2 {
                // Take last two parts for domain
                Some(format!(
                    "{}.{}",
                    parts[parts.len() - 2],
                    parts[parts.len() - 1]
                ))
            } else if parts.len() == 1 && !parts[0].is_empty() {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "localhost".to_string())
        // Remove spaces - NNTP servers may normalize message IDs by removing spaces
        .replace(' ', "")
}

/// Validate input length constraints
fn validate_input_lengths(subject: &str, body: &str) -> Result<(), AppError> {
    if subject.len() > MAX_SUBJECT_LENGTH {
        return Err(AppError::Internal(format!(
            "Subject too long (max {} characters)",
            MAX_SUBJECT_LENGTH
        )));
    }
    if body.len() > MAX_BODY_LENGTH {
        return Err(AppError::Internal(format!(
            "Message body too long (max {} characters)",
            MAX_BODY_LENGTH
        )));
    }
    Ok(())
}

/// Post an article to NNTP and update cache for immediate visibility.
///
/// This function:
/// 1. Generates message ID and date
/// 2. Posts the article to NNTP server
/// 3. Builds an ArticleView from local data
/// 4. Waits for STAT confirmation that article is indexed
/// 5. Updates cache for immediate visibility after redirect
async fn post_and_update_cache(
    state: &AppState,
    params: PostArticleParams<'_>,
) -> Result<(), AppError> {
    let message_id = generate_message_id(&get_domain(state));
    let date = Utc::now().format("%a, %d %b %Y %H:%M:%S %z").to_string();

    // Build headers
    let mut headers = vec![
        ("From".to_string(), params.from.clone()),
        ("Newsgroups".to_string(), params.group.to_string()),
        ("Subject".to_string(), params.subject.clone()),
        ("Message-ID".to_string(), message_id.clone()),
        ("Date".to_string(), date.clone()),
    ];
    if let Some(refs) = &params.references {
        headers.push(("References".to_string(), refs.clone()));
    }
    headers.push((
        "User-Agent".to_string(),
        format!("September/{}", env!("CARGO_PKG_VERSION")),
    ));

    // Post the article
    state
        .nntp
        .post_article(params.group, headers, params.body.clone())
        .await
        .map_err(|e| AppError::Internal(format!("Failed to post: {}", e)))?;

    // Build ArticleView from local data (no network fetch needed)
    let (body_preview, has_more_content) = compute_preview(&params.body);
    let article = ArticleView {
        message_id,
        subject: params.subject,
        from: params.from,
        date: date.clone(),
        date_relative: compute_timeago(&date),
        body: Some(params.body),
        body_preview: Some(body_preview),
        has_more_content,
        headers: None,
    };

    // Inject into cache after confirming existence via STAT
    state
        .nntp
        .inject_posted_article(
            params.group,
            article,
            params.root_message_id,
            params.parent_message_id,
        )
        .await;

    Ok(())
}

/// Handler for compose form (new post)
#[instrument(
    name = "post::compose",
    skip(state, request_id, auth),
    fields(group = %group)
)]
pub async fn compose(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    auth: RequireAuthWithEmail,
    Path(group): Path<String>,
) -> Result<Html<String>, AppErrorResponse> {
    let RequireAuthWithEmail { user, email } = auth;

    // Check if posting is allowed for this group
    let can_post = state.nntp.can_post_to_group(&group).await;
    if !can_post {
        return Err(AppError::Internal(
            "Posting not allowed to this group".into(),
        ))
        .with_request_id(&request_id);
    }

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &group);
    context.insert(
        "user",
        &serde_json::json!({
            "display_name": user.display_name(),
            "email": email,
        }),
    );
    context.insert("csrf_token", &user.csrf_token);
    context.insert("oidc_enabled", &state.oidc.is_some());

    let html = state
        .tera
        .render("compose.html", &context)
        .map_err(AppError::from)
        .with_request_id(&request_id)?;

    Ok(Html(html))
}

/// Handler for submitting a new post
#[instrument(
    name = "post::submit",
    skip(state, request_id, auth, form),
    fields(group = %group)
)]
pub async fn submit(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    auth: RequireAuthWithEmail,
    Path(group): Path<String>,
    Form(form): Form<ComposeForm>,
) -> Result<Redirect, AppErrorResponse> {
    let RequireAuthWithEmail { user, email } = auth;

    // Validate CSRF token
    if !user.validate_csrf(&form.csrf_token) {
        return Err(AppError::Internal(
            "Invalid form submission. Please try again.".into(),
        ))
        .with_request_id(&request_id);
    }

    // Validate input
    validate_input_lengths(&form.subject, &form.body).with_request_id(&request_id)?;
    if form.subject.trim().is_empty() {
        return Err(AppError::Internal("Subject is required".into())).with_request_id(&request_id);
    }
    if form.body.trim().is_empty() {
        return Err(AppError::Internal("Message body is required".into()))
            .with_request_id(&request_id);
    }

    // Post and update cache
    post_and_update_cache(
        &state,
        PostArticleParams {
            group: &group,
            subject: form.subject.trim().to_string(),
            body: form.body,
            from: format_from_header(user.name.as_deref(), &email),
            references: None,
            root_message_id: None,
            parent_message_id: None,
        },
    )
    .await
    .with_request_id(&request_id)?;

    tracing::info!(group = %group, "New article posted successfully");
    Ok(Redirect::to(&format!("/g/{}", group)))
}

/// Handler for submitting a reply
#[instrument(
    name = "post::reply",
    skip(state, request_id, auth, form),
    fields(message_id = %message_id)
)]
pub async fn reply(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    auth: RequireAuthWithEmail,
    Path(message_id): Path<String>,
    Form(form): Form<ReplyForm>,
) -> Result<Redirect, AppErrorResponse> {
    let RequireAuthWithEmail { user, email } = auth;

    // Validate CSRF token
    if !user.validate_csrf(&form.csrf_token) {
        return Err(AppError::Internal(
            "Invalid form submission. Please try again.".into(),
        ))
        .with_request_id(&request_id);
    }

    // Validate input
    validate_input_lengths(&form.subject, &form.body).with_request_id(&request_id)?;
    if form.body.trim().is_empty() {
        return Err(AppError::Internal("Message body is required".into()))
            .with_request_id(&request_id);
    }

    // Build references chain: parent's References + parent's Message-ID
    let references = if form.references.trim().is_empty() {
        message_id.clone()
    } else {
        format!("{} {}", form.references.trim(), message_id)
    };

    // Determine thread root (first in references chain, or parent if direct reply)
    let root_message_id = if form.references.trim().is_empty() {
        message_id.clone()
    } else {
        form.references
            .split_whitespace()
            .next()
            .unwrap_or(&message_id)
            .to_string()
    };

    // Post and update cache
    post_and_update_cache(
        &state,
        PostArticleParams {
            group: &form.group,
            subject: form.subject.trim().to_string(),
            body: form.body,
            from: format_from_header(user.name.as_deref(), &email),
            references: Some(references),
            root_message_id: Some(&root_message_id),
            parent_message_id: Some(&message_id),
        },
    )
    .await
    .with_request_id(&request_id)?;

    tracing::info!(parent = %message_id, group = %form.group, "Reply posted successfully");
    let encoded_parent = urlencoding::encode(&message_id);
    Ok(Redirect::to(&format!(
        "/g/{}/thread/{}",
        form.group, encoded_parent
    )))
}
