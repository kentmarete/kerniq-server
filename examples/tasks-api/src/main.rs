//! tasks-api: a small Kerniq service that stores tasks in an in-memory map
//! and lists them. Task routes require an `x-api-key` header.

mod auth;
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
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(_) => {
            eprintln!("configuration error: the API_KEY environment variable must be set");
            std::process::exit(1);
        }
    };
    let address = config.server.address();
    println!("{} listening on http://{address}", config.service_name);

    app(config).run(address).await
}

pub fn app(config: AppConfig) -> App<AppState> {
    let state = AppState::new(&config);
    App::new()
        .config(config.server)
        .state(state)
        .merge(routes::router(&config.api_key))
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use kerniq_server::axum::Router;
    use kerniq_server::axum::body::Body;
    use kerniq_server::http::Request;
    use tower::ServiceExt;

    use super::*;

    const KEY: &str = "test-key";

    fn router() -> Router {
        app(AppConfig {
            server: ServerConfig::default(),
            service_name: "tasks-api".into(),
            api_key: KEY.into(),
        })
        .into_axum_router()
    }

    async fn json(response: kerniq_server::axum::response::Response) -> serde_json::Value {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    fn create(key: Option<&str>, body: &'static str) -> Request<Body> {
        let mut req = Request::post("/api/v1/tasks").header("content-type", "application/json");
        if let Some(key) = key {
            req = req.header("x-api-key", key);
        }
        req.body(Body::from(body)).unwrap()
    }

    fn list(key: Option<&str>) -> Request<Body> {
        let mut req = Request::get("/api/v1/tasks");
        if let Some(key) = key {
            req = req.header("x-api-key", key);
        }
        req.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn health_is_public() {
        let response = router()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn creates_and_lists_tasks() {
        let router = router();

        let response = router
            .clone()
            .oneshot(create(Some(KEY), r#"{"title":"Write docs"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
        let task = json(response).await;
        assert_eq!(task["id"], 1);
        assert_eq!(task["title"], "Write docs");
        assert_eq!(task["done"], false);

        router
            .clone()
            .oneshot(create(Some(KEY), r#"{"title":"Ship it","done":true}"#))
            .await
            .unwrap();

        let response = router.oneshot(list(Some(KEY))).await.unwrap();
        assert_eq!(response.status(), 200);
        let tasks = json(response).await;
        assert_eq!(tasks.as_array().unwrap().len(), 2);
        assert_eq!(tasks[1]["title"], "Ship it");
    }

    fn get(id: &str, key: Option<&str>) -> Request<Body> {
        let mut req = Request::get(format!("/api/v1/tasks/{id}"));
        if let Some(key) = key {
            req = req.header("x-api-key", key);
        }
        req.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn gets_task_by_id() {
        let router = router();
        router
            .clone()
            .oneshot(create(Some(KEY), r#"{"title":"Write docs"}"#))
            .await
            .unwrap();

        let response = router.oneshot(get("1", Some(KEY))).await.unwrap();
        assert_eq!(response.status(), 200);
        let task = json(response).await;
        assert_eq!(task["id"], 1);
        assert_eq!(task["title"], "Write docs");
    }

    #[tokio::test]
    async fn unknown_task_is_not_found() {
        let response = router().oneshot(get("42", Some(KEY))).await.unwrap();
        assert_eq!(response.status(), 404);
        assert_eq!(json(response).await["error"]["code"], "TASK_NOT_FOUND");
    }

    #[tokio::test]
    async fn non_numeric_id_is_bad_request() {
        let response = router().oneshot(get("abc", Some(KEY))).await.unwrap();
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn get_by_id_requires_api_key() {
        let response = router().oneshot(get("1", None)).await.unwrap();
        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn rejects_empty_title() {
        let response = router()
            .oneshot(create(Some(KEY), r#"{"title":"  "}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
        assert_eq!(json(response).await["error"]["code"], "EMPTY_TITLE");
    }

    #[tokio::test]
    async fn missing_api_key_is_unauthorized() {
        let response = router().oneshot(list(None)).await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(json(response).await["error"]["code"], "UNAUTHORIZED");
    }

    #[tokio::test]
    async fn wrong_api_key_is_unauthorized() {
        let response = router()
            .oneshot(create(Some("nope"), r#"{"title":"x"}"#))
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
}
