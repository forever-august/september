//! Authentication routes for OIDC/OAuth2 login flow.
//!
//! Routes:
//! - GET /auth/login - Show provider selection page (or redirect if single provider)
//! - GET /auth/login/:provider - Initiate OIDC flow with specific provider
//! - GET /auth/callback/:provider - Handle IdP callback
//! - POST /auth/logout - Clear session and redirect to home

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::{
    cookie::{Cookie, PrivateCookieJar, SameSite},
    Host,
};
use http::{HeaderMap, StatusCode};
use openidconnect::{CsrfToken, PkceCodeChallenge};
use serde::Deserialize;
use time::Duration as TimeDuration;
use tracing::instrument;

use crate::oidc::session::{cookie_names, AuthFlowState, User};
use crate::state::AppState;

/// Query parameters for login initiation
#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    /// URL to redirect to after successful login
    pub return_to: Option<String>,
}

/// Query parameters from IdP callback
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Form data for logout
#[derive(Debug, Deserialize)]
pub struct LogoutForm {
    /// Optional URL to redirect to after logout
    pub return_to: Option<String>,
}

/// Validate a return_to URL to prevent open redirects.
/// Only allows relative paths starting with "/" and not containing "//".
fn validate_return_to(return_to: Option<&str>) -> Option<String> {
    let url = return_to?;
    let trimmed = url.trim();

    // Must start with "/" (relative path)
    if !trimmed.starts_with('/') {
        return None;
    }

    // Must not contain "//" which could be a protocol-relative URL
    if trimmed.contains("//") {
        return None;
    }

    // Must not contain control characters or start with "/\"
    if trimmed.starts_with("/\\") || trimmed.chars().any(|c| c.is_control()) {
        return None;
    }

    Some(trimmed.to_string())
}

/// Detect if the request is using HTTPS based on headers and scheme.
/// Checks X-Forwarded-Proto header first (for reverse proxies), then request scheme.
fn detect_https(headers: &HeaderMap) -> bool {
    // Check X-Forwarded-Proto header (set by reverse proxies)
    if let Some(proto) = headers.get("x-forwarded-proto") {
        if let Ok(proto_str) = proto.to_str() {
            return proto_str.eq_ignore_ascii_case("https");
        }
    }

    // Check X-Forwarded-Ssl header
    if let Some(ssl) = headers.get("x-forwarded-ssl") {
        if let Ok(ssl_str) = ssl.to_str() {
            return ssl_str.eq_ignore_ascii_case("on");
        }
    }

    false
}

/// Show provider selection page or redirect to single provider
#[instrument(name = "auth::login", skip(state, _jar))]
pub async fn login(
    State(state): State<AppState>,
    _jar: PrivateCookieJar,
    Query(query): Query<LoginQuery>,
) -> Result<Response, AuthError> {
    let oidc = state.oidc.as_ref().ok_or(AuthError::NotConfigured)?;

    let providers: Vec<_> = oidc.providers().collect();

    if providers.is_empty() {
        return Err(AuthError::NotConfigured);
    }

    // If only one provider, redirect directly to it
    if providers.len() == 1 {
        let provider = &providers[0];
        let redirect_url = if let Some(return_to) = &query.return_to {
            format!(
                "/auth/login/{}?return_to={}",
                provider.name,
                urlencoding::encode(return_to)
            )
        } else {
            format!("/auth/login/{}", provider.name)
        };
        return Ok(Redirect::to(&redirect_url).into_response());
    }

    // Multiple providers - show selection page
    let provider_list: Vec<_> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "display_name": p.display_name,
            })
        })
        .collect();

    let mut context = tera::Context::new();
    context.insert("config", &state.config.ui);
    context.insert("providers", &provider_list);
    context.insert("return_to", &query.return_to);

    let html = state
        .tera
        .render("auth/login.html", &context)
        .map_err(|e| AuthError::Internal(format!("Template error: {}", e)))?;

    Ok(Html(html).into_response())
}

/// Initiate OIDC flow with specific provider
#[instrument(name = "auth::login_provider", skip(state, jar, headers), fields(provider = %provider))]
pub async fn login_provider(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Host(host): Host,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<LoginQuery>,
) -> Result<(PrivateCookieJar, Redirect), AuthError> {
    let oidc = state.oidc.as_ref().ok_or(AuthError::NotConfigured)?;

    let provider_config = oidc
        .get_provider(&provider)
        .ok_or_else(|| AuthError::ProviderNotFound(provider.clone()))?;

    // Generate PKCE challenge
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate CSRF token
    let csrf_token = CsrfToken::new_random();

    // Detect HTTPS from headers
    let use_https = detect_https(&headers);

    // Build redirect URI from Host header
    let redirect_uri = oidc
        .build_redirect_uri(&host, &provider, use_https)
        .map_err(|e| AuthError::Internal(e.to_string()))?;

    // Build authorization URL
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        provider_config.endpoints.auth_url.as_str(),
        urlencoding::encode(provider_config.client_id.as_str()),
        urlencoding::encode(redirect_uri.as_str()),
        urlencoding::encode("openid email profile"),
        urlencoding::encode(csrf_token.secret()),
        urlencoding::encode(pkce_challenge.as_str()),
    );

    // Validate return_to to prevent open redirects
    let safe_return_to = validate_return_to(query.return_to.as_deref());

    // Store flow state in cookie
    let flow_state = AuthFlowState::new(
        csrf_token.secret().to_string(),
        pkce_verifier.secret().to_string(),
        safe_return_to,
    );

    let flow_state_json = serde_json::to_string(&flow_state)
        .map_err(|e| AuthError::Internal(format!("Failed to serialize flow state: {}", e)))?;

    let cookie = Cookie::build((cookie_names::AUTH_FLOW, flow_state_json))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(TimeDuration::minutes(10))
        .build();

    let jar = jar.add(cookie);

    Ok((jar, Redirect::to(&auth_url)))
}

/// Handle IdP callback
#[instrument(name = "auth::callback", skip(state, jar, headers), fields(provider = %provider))]
pub async fn callback(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Host(host): Host,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Result<(PrivateCookieJar, Response), AuthError> {
    let oidc = state.oidc.as_ref().ok_or(AuthError::NotConfigured)?;

    // Check for error from IdP
    if let Some(error) = &query.error {
        let description = query
            .error_description
            .as_deref()
            .unwrap_or("Unknown error");
        tracing::warn!(error = %error, description = %description, "IdP returned error");
        return Err(AuthError::IdpError {
            error: error.clone(),
            description: description.to_string(),
        });
    }

    // Get authorization code
    let code = query.code.as_ref().ok_or(AuthError::MissingCode)?;

    // Get and validate state
    let state_param = query.state.as_ref().ok_or(AuthError::InvalidState)?;

    // Get flow state from cookie
    let flow_state_cookie = jar
        .get(cookie_names::AUTH_FLOW)
        .ok_or(AuthError::InvalidState)?;

    let flow_state: AuthFlowState =
        serde_json::from_str(flow_state_cookie.value()).map_err(|_| AuthError::InvalidState)?;

    // Validate CSRF token
    if !flow_state.validate_state(state_param) {
        return Err(AuthError::InvalidState);
    }

    // Check expiry
    if flow_state.is_expired() {
        return Err(AuthError::FlowExpired);
    }

    // Get provider config
    let provider_config = oidc
        .get_provider(&provider)
        .ok_or_else(|| AuthError::ProviderNotFound(provider.clone()))?;

    // Detect HTTPS from headers
    let use_https = detect_https(&headers);

    // Exchange code for tokens - use the same redirect URI as in login
    let redirect_uri = oidc
        .build_redirect_uri(&host, &provider, use_https)
        .map_err(|e| AuthError::Internal(e.to_string()))?;

    let token_response = exchange_code_for_tokens(
        oidc.http_client(),
        &provider_config,
        code,
        &redirect_uri,
        &flow_state.pkce_verifier,
    )
    .await?;

    // Fetch user info
    let user_info = fetch_user_info(
        oidc.http_client(),
        &provider_config,
        &token_response.access_token,
    )
    .await?;

    // Extract user fields
    let sub = user_info
        .get(&provider_config.userinfo_sub_field)
        .and_then(|v| v.as_str())
        .or_else(|| {
            user_info
                .get(&provider_config.userinfo_sub_field)
                .and_then(|v| v.as_i64().map(|_| ""))
        })
        .map(|s| s.to_string())
        .or_else(|| {
            user_info
                .get(&provider_config.userinfo_sub_field)
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
        })
        .ok_or_else(|| AuthError::MissingClaim(provider_config.userinfo_sub_field.clone()))?;

    let name = user_info
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);

    let email = user_info
        .get("email")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Create user session
    let user = User::new(sub, name, email, provider.clone(), oidc.session_lifetime());

    let user_json = serde_json::to_string(&user)
        .map_err(|e| AuthError::Internal(format!("Failed to serialize user: {}", e)))?;

    // Set session cookie
    let session_cookie = Cookie::build((cookie_names::SESSION, user_json))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(TimeDuration::days(
            oidc.session_lifetime().as_secs() as i64 / 86400,
        ))
        .build();

    // Remove auth flow cookie
    let remove_flow_cookie = Cookie::build((cookie_names::AUTH_FLOW, ""))
        .path("/")
        .max_age(TimeDuration::ZERO)
        .build();

    let jar = jar.add(session_cookie).remove(remove_flow_cookie);

    // Redirect to return_to (already validated during login) or home
    let redirect_url = flow_state.return_to.as_deref().unwrap_or("/");

    Ok((jar, Redirect::to(redirect_url).into_response()))
}

/// Logout handler
#[instrument(name = "auth::logout", skip(_state, jar))]
pub async fn logout(
    State(_state): State<AppState>,
    jar: PrivateCookieJar,
    Form(form): Form<LogoutForm>,
) -> (PrivateCookieJar, Redirect) {
    // Remove session cookie
    let remove_cookie = Cookie::build((cookie_names::SESSION, ""))
        .path("/")
        .max_age(TimeDuration::ZERO)
        .build();

    let jar = jar.remove(remove_cookie);

    // Validate return_to to prevent open redirects
    let redirect_url =
        validate_return_to(form.return_to.as_deref()).unwrap_or_else(|| "/".to_string());
    (jar, Redirect::to(&redirect_url))
}

/// Token response from token endpoint
#[derive(Debug, Deserialize)]
struct TokenResponseData {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[serde(default)]
    #[allow(dead_code)]
    expires_in: Option<u64>,
    // Note: id_token and refresh_token are intentionally not captured.
    // We rely on the userinfo endpoint for user claims, which is more
    // compatible across OAuth2/OIDC providers.
}

/// Exchange authorization code for tokens
async fn exchange_code_for_tokens(
    http_client: &reqwest::Client,
    provider: &crate::oidc::OidcProvider,
    code: &str,
    redirect_uri: &openidconnect::RedirectUrl,
    pkce_verifier: &str,
) -> Result<TokenResponseData, AuthError> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", provider.client_id.as_str()),
        ("client_secret", provider.client_secret.secret()),
        ("code_verifier", pkce_verifier),
    ];

    let response = http_client
        .post(provider.endpoints.token_url.as_str())
        .form(&params)
        .send()
        .await
        .map_err(|e| AuthError::TokenExchange(format!("Request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %body, "Token exchange failed");
        return Err(AuthError::TokenExchange(format!(
            "Token endpoint returned {}: {}",
            status, body
        )));
    }

    let token_response: TokenResponseData = response
        .json()
        .await
        .map_err(|e| AuthError::TokenExchange(format!("Failed to parse response: {}", e)))?;

    Ok(token_response)
}

/// Fetch user info from userinfo endpoint
async fn fetch_user_info(
    http_client: &reqwest::Client,
    provider: &crate::oidc::OidcProvider,
    access_token: &str,
) -> Result<serde_json::Value, AuthError> {
    let userinfo_url = provider
        .endpoints
        .userinfo_url
        .as_ref()
        .ok_or_else(|| AuthError::Internal("No userinfo endpoint configured".to_string()))?;

    let response = http_client
        .get(userinfo_url.as_str())
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AuthError::UserInfo(format!("Request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AuthError::UserInfo(format!(
            "Userinfo endpoint returned {}: {}",
            status, body
        )));
    }

    let user_info: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AuthError::UserInfo(format!("Failed to parse response: {}", e)))?;

    Ok(user_info)
}

/// Auth-specific error type
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("OIDC authentication is not configured")]
    NotConfigured,

    #[error("Provider '{0}' not found")]
    ProviderNotFound(String),

    #[error("Identity provider returned error: {error} - {description}")]
    IdpError { error: String, description: String },

    #[error("Missing authorization code")]
    MissingCode,

    #[error("Invalid or missing state parameter")]
    InvalidState,

    #[error("Authentication flow expired")]
    FlowExpired,

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Failed to fetch user info: {0}")]
    UserInfo(String),

    #[error("Missing required claim: {0}")]
    MissingClaim(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::NotConfigured => (
                StatusCode::NOT_FOUND,
                "Authentication is not configured on this server".to_string(),
            ),
            AuthError::ProviderNotFound(name) => (
                StatusCode::NOT_FOUND,
                format!("Authentication provider '{}' not found", name),
            ),
            AuthError::IdpError {
                error: _,
                description,
            } => (
                StatusCode::BAD_REQUEST,
                format!("Authentication failed: {}", description),
            ),
            AuthError::MissingCode | AuthError::InvalidState | AuthError::FlowExpired => (
                StatusCode::BAD_REQUEST,
                "Authentication flow invalid or expired. Please try again.".to_string(),
            ),
            AuthError::TokenExchange(msg) | AuthError::UserInfo(msg) => {
                tracing::error!(error = %msg, "Auth error");
                (
                    StatusCode::BAD_GATEWAY,
                    "Failed to complete authentication with provider".to_string(),
                )
            }
            AuthError::MissingClaim(claim) => {
                tracing::error!(claim = %claim, "Missing claim");
                (
                    StatusCode::BAD_GATEWAY,
                    "Provider did not return required user information".to_string(),
                )
            }
            AuthError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal auth error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        // Return error page using template structure
        // Note: We use inline HTML here since we don't have access to Tera in the error handler.
        // This matches the structure of templates/auth/error.html
        let body = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication Error</title>
    <link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
    <header class="site-header">
        <nav class="main-nav">
            <a href="/" class="nav-home">Home</a>
        </nav>
    </header>
    <main class="container">
        <div class="error-page">
            <h1>Authentication Error</h1>
            <p>{}</p>
            <a href="/">Return to homepage</a>
        </div>
    </main>
</body>
</html>"#,
            message
        );

        (status, Html(body)).into_response()
    }
}
