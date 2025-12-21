use std::sync::Arc;
use tera::Tera;

use crate::config::AppConfig;
use crate::nntp::NntpFederatedService;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub tera: Arc<Tera>,
    pub nntp: NntpFederatedService,
}

impl AppState {
    pub fn new(config: AppConfig, tera: Tera, nntp: NntpFederatedService) -> Self {
        Self {
            config: Arc::new(config),
            tera: Arc::new(tera),
            nntp,
        }
    }
}
