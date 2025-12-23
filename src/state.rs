//! Shared application state for request handlers.

use std::sync::Arc;
use tera::Tera;

use crate::config::AppConfig;
use crate::nntp::NntpFederatedService;

/// Shared application state, cloneable across handlers via Arc-wrapped fields.
///
/// Contains the application configuration, Tera template engine, and the
/// federated NNTP service for accessing newsgroup servers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub tera: Arc<Tera>,
    pub nntp: NntpFederatedService,
}

impl AppState {
    /// Creates a new application state from the given configuration, templates, and NNTP service.
    pub fn new(config: AppConfig, tera: Tera, nntp: NntpFederatedService) -> Self {
        Self {
            config: Arc::new(config),
            tera: Arc::new(tera),
            nntp,
        }
    }
}
