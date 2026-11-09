mod health;
mod tasks;

use kerniq_server::prelude::*;

use crate::auth::ApiKeyAuth;
use crate::state::AppState;

/// All routes of the application. `/health` is public; everything under
/// `/api/v1` requires a valid `x-api-key` header.
pub fn router(api_key: &str) -> Router<AppState> {
    let protected = Router::new()
        .mount("/tasks", tasks::router())
        .middleware(ApiKeyAuth::layer(api_key));

    Router::new()
        .get("/health", health::health)
        .mount("/api/v1", protected)
}
