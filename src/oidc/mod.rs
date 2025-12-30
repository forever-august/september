//! OpenID Connect / OAuth2 authentication module.
//!
//! Provides OIDC client management with support for both:
//! - Discovery mode: endpoints auto-discovered via .well-known/openid-configuration
//! - Manual mode: endpoints explicitly configured (for OAuth2-only providers like GitHub)

pub mod session;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum_extra::extract::cookie::Key;
use hkdf::Hkdf;
use openidconnect::core::CoreProviderMetadata;
use openidconnect::{
    AuthUrl, ClientId, ClientSecret, IssuerUrl, RedirectUrl, TokenUrl, UserInfoUrl,
};
use sha2::Sha256;

use crate::config::{OidcConfig, OidcProviderConfig};

/// Error type for OIDC operations
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("Provider '{0}' not found")]
    ProviderNotFound(String),

    #[error("Discovery failed for provider '{provider}': {message}")]
    Discovery { provider: String, message: String },

    #[error("Invalid URL for provider '{provider}': {message}")]
    InvalidUrl { provider: String, message: String },

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Failed to fetch user info: {0}")]
    UserInfo(String),

    #[error("Invalid state parameter")]
    InvalidState,

    #[error("Missing required claim: {0}")]
    MissingClaim(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// Endpoints for an OIDC/OAuth2 provider
#[derive(Clone, Debug)]
pub struct ProviderEndpoints {
    /// Authorization endpoint
    pub auth_url: AuthUrl,
    /// Token endpoint
    pub token_url: TokenUrl,
    /// UserInfo endpoint (optional for OIDC, required for manual mode)
    pub userinfo_url: Option<UserInfoUrl>,
    /// Issuer URL (for ID token validation in discovery mode)
    pub issuer_url: Option<IssuerUrl>,
}

/// A configured OIDC/OAuth2 provider
#[derive(Clone)]
pub struct OidcProvider {
    /// URL-safe identifier
    pub name: String,
    /// Human-readable display name
    pub display_name: String,
    /// OAuth2 client ID
    pub client_id: ClientId,
    /// OAuth2 client secret
    pub client_secret: ClientSecret,
    /// Provider endpoints
    pub endpoints: ProviderEndpoints,
    /// Field name for subject ID in userinfo response (default: "sub")
    pub userinfo_sub_field: String,
    /// Whether this provider uses manual endpoint configuration (no ID token validation)
    pub is_manual_mode: bool,
}

/// Manages all configured OIDC providers
#[derive(Clone)]
pub struct OidcManager {
    /// Map of provider name -> provider
    providers: HashMap<String, Arc<OidcProvider>>,
    /// Key for signing/encrypting cookies
    cookie_key: Key,
    /// Session lifetime
    session_lifetime: Duration,
    /// Optional base URL for redirect URIs (if not auto-detected)
    redirect_uri_base: Option<String>,
    /// HTTP client for OIDC operations
    http_client: reqwest::Client,
}

impl OidcManager {
    /// Initialize the OIDC manager by discovering/configuring all providers.
    ///
    /// This performs async discovery for providers using issuer_url.
    pub async fn new(config: &OidcConfig) -> Result<Self, OidcError> {
        // Resolve and derive cookie key
        let secret = config
            .resolve_cookie_secret()
            .map_err(|e| OidcError::Config(format!("Failed to resolve cookie secret: {}", e)))?;

        // Derive a 64-byte key from the secret using HKDF
        let cookie_key = derive_cookie_key(&secret);

        let session_lifetime = Duration::from_secs(config.session_lifetime_days * 24 * 60 * 60);

        // Create HTTP client for OIDC operations
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| OidcError::Config(format!("Failed to create HTTP client: {}", e)))?;

        let mut providers = HashMap::new();

        for provider_config in &config.providers {
            let provider = init_provider(provider_config, &http_client).await?;
            providers.insert(provider.name.clone(), Arc::new(provider));
        }

        Ok(Self {
            providers,
            cookie_key,
            session_lifetime,
            redirect_uri_base: config.redirect_uri_base.clone(),
            http_client,
        })
    }

    /// Get a provider by name
    pub fn get_provider(&self, name: &str) -> Option<Arc<OidcProvider>> {
        self.providers.get(name).cloned()
    }

    /// Get all providers (for login page)
    pub fn providers(&self) -> impl Iterator<Item = &Arc<OidcProvider>> {
        self.providers.values()
    }

    /// Get the number of configured providers
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Get the cookie signing key
    pub fn cookie_key(&self) -> &Key {
        &self.cookie_key
    }

    /// Get the session lifetime
    pub fn session_lifetime(&self) -> Duration {
        self.session_lifetime
    }

    /// Get the configured redirect URI base, if any
    pub fn redirect_uri_base(&self) -> Option<&str> {
        self.redirect_uri_base.as_deref()
    }

    /// Get the HTTP client for OIDC operations
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    /// Build the redirect URI for a provider callback
    pub fn build_redirect_uri(
        &self,
        host: &str,
        provider_name: &str,
        use_https: bool,
    ) -> Result<RedirectUrl, OidcError> {
        let uri = if let Some(base) = &self.redirect_uri_base {
            format!(
                "{}/auth/callback/{}",
                base.trim_end_matches('/'),
                provider_name
            )
        } else {
            let scheme = if use_https { "https" } else { "http" };
            format!("{}://{}/auth/callback/{}", scheme, host, provider_name)
        };

        RedirectUrl::new(uri.clone()).map_err(|e| OidcError::InvalidUrl {
            provider: provider_name.to_string(),
            message: format!("Invalid redirect URI '{}': {}", uri, e),
        })
    }
}

/// Initialize a single provider from config
async fn init_provider(
    config: &OidcProviderConfig,
    http_client: &reqwest::Client,
) -> Result<OidcProvider, OidcError> {
    let client_id = ClientId::new(config.client_id.clone());
    let client_secret_str = config.resolve_client_secret().map_err(|e| {
        OidcError::Config(format!(
            "Failed to resolve client secret for provider '{}': {}",
            config.name, e
        ))
    })?;
    let client_secret = ClientSecret::new(client_secret_str);

    if config.uses_discovery() {
        init_provider_discovery(config, client_id, client_secret, http_client).await
    } else {
        init_provider_manual(config, client_id, client_secret)
    }
}

/// Initialize provider using OIDC discovery
async fn init_provider_discovery(
    config: &OidcProviderConfig,
    client_id: ClientId,
    client_secret: ClientSecret,
    http_client: &reqwest::Client,
) -> Result<OidcProvider, OidcError> {
    let issuer_url_str = config.issuer_url.as_ref().unwrap();

    let issuer_url = IssuerUrl::new(issuer_url_str.clone()).map_err(|e| OidcError::InvalidUrl {
        provider: config.name.clone(),
        message: format!("Invalid issuer URL '{}': {}", issuer_url_str, e),
    })?;

    // Perform discovery
    let metadata: CoreProviderMetadata =
        CoreProviderMetadata::discover_async(issuer_url.clone(), http_client)
            .await
            .map_err(|e| OidcError::Discovery {
                provider: config.name.clone(),
                message: e.to_string(),
            })?;

    // Extract endpoints from metadata
    let auth_url = metadata.authorization_endpoint().clone();
    let token_url = metadata
        .token_endpoint()
        .cloned()
        .ok_or_else(|| OidcError::Discovery {
            provider: config.name.clone(),
            message: "No token endpoint in discovery metadata".to_string(),
        })?;
    let userinfo_url = metadata.userinfo_endpoint().cloned();

    Ok(OidcProvider {
        name: config.name.clone(),
        display_name: config.display_name.clone(),
        client_id,
        client_secret,
        endpoints: ProviderEndpoints {
            auth_url,
            token_url,
            userinfo_url,
            issuer_url: Some(issuer_url),
        },
        userinfo_sub_field: config.userinfo_sub_field.clone(),
        is_manual_mode: false,
    })
}

/// Initialize provider with manual endpoint configuration
fn init_provider_manual(
    config: &OidcProviderConfig,
    client_id: ClientId,
    client_secret: ClientSecret,
) -> Result<OidcProvider, OidcError> {
    let auth_url =
        AuthUrl::new(config.auth_url.clone().unwrap()).map_err(|e| OidcError::InvalidUrl {
            provider: config.name.clone(),
            message: format!("Invalid auth URL: {}", e),
        })?;

    let token_url =
        TokenUrl::new(config.token_url.clone().unwrap()).map_err(|e| OidcError::InvalidUrl {
            provider: config.name.clone(),
            message: format!("Invalid token URL: {}", e),
        })?;

    let userinfo_url = UserInfoUrl::new(config.userinfo_url.clone().unwrap()).map_err(|e| {
        OidcError::InvalidUrl {
            provider: config.name.clone(),
            message: format!("Invalid userinfo URL: {}", e),
        }
    })?;

    Ok(OidcProvider {
        name: config.name.clone(),
        display_name: config.display_name.clone(),
        client_id,
        client_secret,
        endpoints: ProviderEndpoints {
            auth_url,
            token_url,
            userinfo_url: Some(userinfo_url),
            issuer_url: None,
        },
        userinfo_sub_field: config.userinfo_sub_field.clone(),
        is_manual_mode: true,
    })
}

/// Derive a 64-byte cookie key from an arbitrary-length secret using HKDF
fn derive_cookie_key(secret: &str) -> Key {
    let hkdf = Hkdf::<Sha256>::new(None, secret.as_bytes());
    let mut key_bytes = [0u8; 64];
    hkdf.expand(b"september-session-cookie", &mut key_bytes)
        .expect("64 bytes is a valid length for HKDF-SHA256");

    Key::from(&key_bytes)
}
