//! Session management for OIDC authentication.
//!
//! Provides:
//! - `User`: Authenticated user information stored in session cookie
//! - `AuthFlowState`: Temporary state during OAuth2 authorization flow
//! - `CsrfToken`: Token for CSRF protection on forms

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Authenticated user information.
/// 
/// This is stored in a signed cookie and represents the current session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Subject identifier (unique ID from the identity provider)
    pub sub: String,
    /// User's display name (from "name" claim or constructed from given/family name)
    pub name: Option<String>,
    /// User's email address
    pub email: Option<String>,
    /// Which provider authenticated this user
    pub provider: String,
    /// When this session expires (Unix timestamp)
    pub expires_at: u64,
    /// CSRF token for form protection
    #[serde(default = "generate_csrf_token")]
    pub csrf_token: String,
}

/// Generate a random CSRF token
fn generate_csrf_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    
    // Use RandomState to generate a unique token
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
    );
    format!("{:016x}", hasher.finish())
}

impl User {
    /// Create a new user session
    pub fn new(
        sub: String,
        name: Option<String>,
        email: Option<String>,
        provider: String,
        lifetime: Duration,
    ) -> Self {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + lifetime.as_secs();
        
        Self {
            sub,
            name,
            email,
            provider,
            expires_at,
            csrf_token: generate_csrf_token(),
        }
    }
    
    /// Check if this session has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now >= self.expires_at
    }
    
    /// Check if this session should be refreshed (within last 20% of lifetime)
    pub fn should_refresh(&self, lifetime: Duration) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        // Refresh when less than 20% of lifetime remains
        let refresh_threshold = lifetime.as_secs() / 5;
        let remaining = self.expires_at.saturating_sub(now);
        
        remaining < refresh_threshold
    }
    
    /// Extend the session expiry (for sliding window)
    pub fn refresh(&mut self, lifetime: Duration) {
        self.expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + lifetime.as_secs();
    }
    
    /// Get the display name, falling back to email or subject ID
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .or(self.email.as_deref())
            .unwrap_or(&self.sub)
    }
    
    /// Validate a CSRF token against the session's token
    pub fn validate_csrf(&self, token: &str) -> bool {
        // Use constant-time comparison to prevent timing attacks
        if self.csrf_token.len() != token.len() {
            return false;
        }
        self.csrf_token
            .bytes()
            .zip(token.bytes())
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

/// Temporary state stored during OAuth2 authorization flow.
/// 
/// This is stored in a short-lived cookie and contains:
/// - CSRF token (state parameter)
/// - PKCE code verifier
/// - Return URL after login
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFlowState {
    /// CSRF protection token (sent as "state" parameter)
    pub csrf_token: String,
    /// PKCE code verifier
    pub pkce_verifier: String,
    /// URL to redirect to after successful login
    pub return_to: Option<String>,
    /// When this flow state expires (Unix timestamp)
    pub expires_at: u64,
}

impl AuthFlowState {
    /// Create new auth flow state with 10-minute expiry
    pub fn new(
        csrf_token: String,
        pkce_verifier: String,
        return_to: Option<String>,
    ) -> Self {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600; // 10 minutes
        
        Self {
            csrf_token,
            pkce_verifier,
            return_to,
            expires_at,
        }
    }
    
    /// Check if this flow state has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now >= self.expires_at
    }
    
    /// Validate that the returned state matches our CSRF token
    pub fn validate_state(&self, state: &str) -> bool {
        self.csrf_token == state
    }
}

/// Cookie names used for authentication
pub mod cookie_names {
    /// Session cookie containing serialized User
    pub const SESSION: &str = "september_session";
    /// Temporary cookie for OAuth2 flow state
    pub const AUTH_FLOW: &str = "september_auth_flow";
}
