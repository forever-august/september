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
fn get_domain(state: &AppState) -> String {
    state.config.ui.site_name
        .as_ref()
        .and_then(|s| {
            // Try to extract domain from site_name
            // e.g., "news.example.com" -> "example.com"
            // e.g., "example.com" -> "example.com"
            let parts: Vec<&str> = s.split('.').collect();
            if parts.len() >= 2 {
                // Take last two parts for domain
                Some(format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]))
            } else if parts.len() == 1 && !parts[0].is_empty() {
                Some(parts[0].to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "localhost".to_string())
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
        return Err(AppError::Internal("Posting not allowed to this group".into()))
            .with_request_id(&request_id);
    }
    
    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("group", &group);
    context.insert("user", &serde_json::json!({
        "display_name": user.display_name(),
        "email": email,
    }));
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
        return Err(AppError::Internal("Invalid form submission. Please try again.".into()))
            .with_request_id(&request_id);
    }
    
    // Validate input lengths
    validate_input_lengths(&form.subject, &form.body)
        .with_request_id(&request_id)?;
    
    // Validate form content
    if form.subject.trim().is_empty() {
        return Err(AppError::Internal("Subject is required".into()))
            .with_request_id(&request_id);
    }
    if form.body.trim().is_empty() {
        return Err(AppError::Internal("Message body is required".into()))
            .with_request_id(&request_id);
    }
    
    // Build headers
    let from = format_from_header(user.name.as_deref(), &email);
    let message_id = generate_message_id(&get_domain(&state));
    let date = Utc::now().format("%a, %d %b %Y %H:%M:%S %z").to_string();
    
    let headers = vec![
        ("From".to_string(), from),
        ("Newsgroups".to_string(), group.clone()),
        ("Subject".to_string(), form.subject.trim().to_string()),
        ("Message-ID".to_string(), message_id),
        ("Date".to_string(), date),
        ("User-Agent".to_string(), format!("September/{}", env!("CARGO_PKG_VERSION"))),
    ];
    
    // Post the article
    state.nntp
        .post_article(&group, headers, form.body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to post: {}", e)))
        .with_request_id(&request_id)?;
    
    tracing::info!(group = %group, "New article posted successfully");
    
    // Redirect back to group
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
        return Err(AppError::Internal("Invalid form submission. Please try again.".into()))
            .with_request_id(&request_id);
    }
    
    // Validate input lengths
    validate_input_lengths(&form.subject, &form.body)
        .with_request_id(&request_id)?;
    
    // Validate form content
    if form.body.trim().is_empty() {
        return Err(AppError::Internal("Message body is required".into()))
            .with_request_id(&request_id);
    }
    
    // Build References header: parent's References + parent's Message-ID
    let references = if form.references.trim().is_empty() {
        message_id.clone()
    } else {
        format!("{} {}", form.references.trim(), message_id)
    };
    
    // Build headers
    let from = format_from_header(user.name.as_deref(), &email);
    let new_message_id = generate_message_id(&get_domain(&state));
    let date = Utc::now().format("%a, %d %b %Y %H:%M:%S %z").to_string();
    
    let headers = vec![
        ("From".to_string(), from),
        ("Newsgroups".to_string(), form.group.clone()),
        ("Subject".to_string(), form.subject.trim().to_string()),
        ("Message-ID".to_string(), new_message_id),
        ("Date".to_string(), date),
        ("References".to_string(), references),
        ("User-Agent".to_string(), format!("September/{}", env!("CARGO_PKG_VERSION"))),
    ];
    
    // Post the reply
    state.nntp
        .post_article(&form.group, headers, form.body)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to post reply: {}", e)))
        .with_request_id(&request_id)?;
    
    tracing::info!(parent = %message_id, group = %form.group, "Reply posted successfully");
    
    // Redirect back to the thread
    // URL-encode the parent message_id for the redirect
    let encoded_parent = urlencoding::encode(&message_id);
    Ok(Redirect::to(&format!("/g/{}/thread/{}", form.group, encoded_parent)))
}
