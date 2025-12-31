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
    pub fn new(csrf_token: String, pkce_verifier: String, return_to: Option<String>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_user_new_sets_expiry() {
        let lifetime = Duration::from_secs(3600); // 1 hour
        let user = User::new(
            "sub123".to_string(),
            Some("Test User".to_string()),
            Some("test@example.com".to_string()),
            "google".to_string(),
            lifetime,
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Expiry should be approximately now + 1 hour (within 2 seconds tolerance)
        assert!(user.expires_at >= now + 3598);
        assert!(user.expires_at <= now + 3602);
    }

    #[test]
    fn test_user_is_expired_false_when_fresh() {
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        assert!(!user.is_expired());
    }

    #[test]
    fn test_user_is_expired_true_when_past() {
        let mut user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        // Set expiry to the past
        user.expires_at = 0;
        assert!(user.is_expired());
    }

    #[test]
    fn test_user_should_refresh_false_when_fresh() {
        let lifetime = Duration::from_secs(3600);
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            lifetime,
        );
        // Fresh session should not need refresh (has > 20% lifetime remaining)
        assert!(!user.should_refresh(lifetime));
    }

    #[test]
    fn test_user_should_refresh_true_near_expiry() {
        let lifetime = Duration::from_secs(3600);
        let mut user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            lifetime,
        );
        // Set expiry to 5 minutes from now (less than 20% of 1 hour = 12 minutes)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        user.expires_at = now + 300; // 5 minutes

        assert!(user.should_refresh(lifetime));
    }

    #[test]
    fn test_user_refresh_extends_expiry() {
        let lifetime = Duration::from_secs(3600);
        let mut user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(60), // Short initial lifetime
        );

        let old_expiry = user.expires_at;
        user.refresh(lifetime);

        // New expiry should be greater than old expiry
        assert!(user.expires_at > old_expiry);

        // New expiry should be approximately now + 1 hour
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(user.expires_at >= now + 3598);
        assert!(user.expires_at <= now + 3602);
    }

    #[test]
    fn test_user_display_name_prefers_name() {
        let user = User::new(
            "sub123".to_string(),
            Some("John Doe".to_string()),
            Some("john@example.com".to_string()),
            "google".to_string(),
            Duration::from_secs(3600),
        );
        assert_eq!(user.display_name(), "John Doe");
    }

    #[test]
    fn test_user_display_name_falls_back_to_email() {
        let user = User::new(
            "sub123".to_string(),
            None,
            Some("john@example.com".to_string()),
            "google".to_string(),
            Duration::from_secs(3600),
        );
        assert_eq!(user.display_name(), "john@example.com");
    }

    #[test]
    fn test_user_display_name_falls_back_to_sub() {
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        assert_eq!(user.display_name(), "sub123");
    }

    #[test]
    fn test_user_validate_csrf_valid() {
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        let token = user.csrf_token.clone();
        assert!(user.validate_csrf(&token));
    }

    #[test]
    fn test_user_validate_csrf_invalid() {
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        assert!(!user.validate_csrf("invalid_token_12345"));
    }

    #[test]
    fn test_user_validate_csrf_different_length() {
        let user = User::new(
            "sub123".to_string(),
            None,
            None,
            "google".to_string(),
            Duration::from_secs(3600),
        );
        // Different length should fail fast
        assert!(!user.validate_csrf("short"));
        assert!(!user.validate_csrf("this_is_a_very_long_token_that_is_longer_than_expected"));
    }

    #[test]
    fn test_auth_flow_state_new_sets_expiry() {
        let state = AuthFlowState::new(
            "csrf123".to_string(),
            "pkce456".to_string(),
            Some("/return".to_string()),
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Expiry should be approximately now + 10 minutes (600 seconds)
        assert!(state.expires_at >= now + 598);
        assert!(state.expires_at <= now + 602);
    }

    #[test]
    fn test_auth_flow_state_is_expired_false_when_fresh() {
        let state = AuthFlowState::new("csrf123".to_string(), "pkce456".to_string(), None);
        assert!(!state.is_expired());
    }

    #[test]
    fn test_auth_flow_state_is_expired_true_when_past() {
        let mut state = AuthFlowState::new("csrf123".to_string(), "pkce456".to_string(), None);
        state.expires_at = 0;
        assert!(state.is_expired());
    }

    #[test]
    fn test_auth_flow_state_validate_state_valid() {
        let state = AuthFlowState::new("csrf123".to_string(), "pkce456".to_string(), None);
        assert!(state.validate_state("csrf123"));
    }

    #[test]
    fn test_auth_flow_state_validate_state_invalid() {
        let state = AuthFlowState::new("csrf123".to_string(), "pkce456".to_string(), None);
        assert!(!state.validate_state("wrong_csrf"));
    }
}
