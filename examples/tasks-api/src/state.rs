use std::sync::Arc;

use crate::config::AppConfig;
use crate::services::tasks::TaskService;

/// Shared state. Cloned per request, so every field is cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub service_name: Arc<str>,
    pub tasks: TaskService,
}

impl AppState {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            service_name: config.service_name.as_str().into(),
            tasks: TaskService::default(),
        }
    }
}
