//! Shared application state for request handlers.

use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use std::sync::Arc;
use tera::Tera;

use crate::config::AppConfig;
use crate::nntp::NntpFederatedService;
use crate::oidc::OidcManager;

/// Shared application state, cloneable across handlers via Arc-wrapped fields.
///
/// Contains the application configuration, Tera template engine, and the
/// federated NNTP service for accessing newsgroup servers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub tera: Arc<Tera>,
    pub nntp: NntpFederatedService,
    pub oidc: Option<OidcManager>,
    /// Cookie signing key for session cookies.
    /// Generated randomly if OIDC is not configured.
    cookie_key: Key,
}

impl AppState {
    /// Creates a new application state from the given configuration, templates, and NNTP service.
    pub fn new(
        config: AppConfig,
        tera: Tera,
        nntp: NntpFederatedService,
        oidc: Option<OidcManager>,
    ) -> Self {
        // Get cookie key from OidcManager if available, otherwise generate random
        let cookie_key = oidc
            .as_ref()
            .map(|o| o.cookie_key().clone())
            .unwrap_or_else(Key::generate);

        Self {
            config: Arc::new(config),
            tera: Arc::new(tera),
            nntp,
            oidc,
            cookie_key,
        }
    }
}

/// Implement FromRef to allow axum-extra's PrivateCookieJar to extract the Key from AppState
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}
