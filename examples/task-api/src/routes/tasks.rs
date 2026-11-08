use kerniq_server::prelude::*;

use crate::models::task::{NewTask, Task};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .get("/", list)
        .post("/", create)
        .get("/{id}", show)
        .post("/{id}/complete", complete)
        .delete("/{id}", remove)
}

async fn list(State(state): State<AppState>) -> Json<Vec<Task>> {
    Json(state.tasks.list().await)
}

async fn show(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<Task>> {
    Ok(Json(state.tasks.get(id).await?))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<NewTask>,
) -> Result<Created<Task>> {
    Ok(Created(state.tasks.create(input).await?))
}

async fn complete(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<Task>> {
    Ok(Json(state.tasks.complete(id).await?))
}

async fn remove(State(state): State<AppState>, Path(id): Path<u64>) -> Result<NoContent> {
    state.tasks.delete(id).await?;
    Ok(NoContent)
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use kerniq_server::axum::Router as AxumRouter;
    use kerniq_server::axum::body::Body;
    use kerniq_server::http::Request;
    use tower::ServiceExt;

    use crate::app;
    use crate::config::AppConfig;

    async fn send(router: &AxumRouter, request: Request<Body>) -> (u16, serde_json::Value) {
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    fn post_json(uri: &str, body: &'static str) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn task_lifecycle() {
        let router = app(AppConfig::default()).into_axum_router();

        let (status, task) = send(
            &router,
            post_json("/api/v1/tasks", r#"{"title":"Write docs"}"#),
        )
        .await;
        assert_eq!(status, 201);
        assert_eq!(task["done"], false);

        let (status, task) = send(&router, post_json("/api/v1/tasks/1/complete", "")).await;
        assert_eq!(status, 200);
        assert_eq!(task["done"], true);

        let delete = Request::delete("/api/v1/tasks/1")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&router, delete).await.0, 204);
    }

    #[tokio::test]
    async fn rejects_empty_title() {
        let router = app(AppConfig::default()).into_axum_router();
        let (status, json) = send(&router, post_json("/api/v1/tasks", r#"{"title":"  "}"#)).await;
        assert_eq!(status, 422);
        assert_eq!(json["error"]["code"], "EMPTY_TITLE");
    }

    #[tokio::test]
    async fn unknown_task_is_404() {
        let router = app(AppConfig::default()).into_axum_router();
        let get = Request::get("/api/v1/tasks/42")
            .body(Body::empty())
            .unwrap();
        let (status, json) = send(&router, get).await;
        assert_eq!(status, 404);
        assert_eq!(json["error"]["code"], "TASK_NOT_FOUND");
    }
}
