//! task-api: the finished project from the tutorial in `docs/tutorial.md`,
//! a feature-oriented Kerniq application.
//!
//! Layout:
//!
//! - `config`   — configuration loaded from the environment
//! - `state`    — the shared application state
//! - `error`    — domain errors and their HTTP mapping
//! - `models`   — data types
//! - `services` — business logic, independent of HTTP
//! - `routes`   — HTTP handlers and routers, one module per feature

mod config;
mod error;
mod models;
mod routes;
mod services;
mod state;

use kerniq_server::prelude::*;

use crate::config::AppConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let address = config.server.address();
    println!("{} listening on http://{address}", config.service_name);

    app(config).run(address).await
}

pub fn app(config: AppConfig) -> App<AppState> {
    let state = AppState::new(&config);
    App::new()
        .config(config.server)
        .state(state)
        .merge(routes::router())
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use kerniq_server::axum::body::Body;
    use kerniq_server::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_and_users() {
        let router = app(AppConfig::default()).into_axum_router();

        let response = router
            .clone()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let request = Request::post("/api/v1/users")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Ada","email":"ada@example.com"}"#))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 201);

        let request = Request::post("/api/v1/users")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"Ada again","email":"ada@example.com"}"#,
            ))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), 409);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "EMAIL_TAKEN");
    }
}
