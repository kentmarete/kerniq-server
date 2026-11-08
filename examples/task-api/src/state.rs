use std::sync::Arc;

use crate::config::AppConfig;
use crate::services::tasks::TaskService;
use crate::services::users::UserService;

/// Shared state. Cloned per request, so every field is cheap to clone.
#[derive(Debug, Clone)]
pub struct AppState {
    pub service_name: Arc<str>,
    pub users: UserService,
    pub tasks: TaskService,
}

impl AppState {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            service_name: config.service_name.as_str().into(),
            users: UserService::default(),
            tasks: TaskService::default(),
        }
    }
}
