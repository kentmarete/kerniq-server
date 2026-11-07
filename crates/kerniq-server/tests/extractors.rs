mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use common::{get, json_request, send};
use http::Request;
use kerniq_server::extract::{BearerToken, UserAgent};
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
}

#[derive(Deserialize)]
struct UserQuery {
    page: Option<u32>,
    limit: Option<u32>,
}

async fn create_user(Json(input): Json<CreateUser>) -> Created<User> {
    Created(User {
        id: 1,
        name: input.name,
    })
}

async fn get_user(Path(id): Path<u64>) -> Result<Json<User>> {
    if id == 404 {
        return Err(
            Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND")
        );
    }
    Ok(Json(User {
        id,
        name: format!("user-{id}"),
    }))
}

async fn list_users(Query(q): Query<UserQuery>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"page": q.page.unwrap_or(1), "limit": q.limit.unwrap_or(20)}))
}

fn app() -> axum::Router {
    App::new()
        .post("/users", create_user)
        .get("/users", list_users)
        .get("/users/{id}", get_user)
        .get(
            "/pair/{a}/{b}",
            |Path((a, b)): Path<(u32, String)>| async move { format!("{a}:{b}") },
        )
        .into_axum_router()
}

#[tokio::test]
async fn json_round_trip_and_created_status() {
    let res = send(&app(), json_request("POST", "/users", r#"{"name":"Ada"}"#)).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.header("content-type"), Some("application/json"));
    assert_eq!(res.json(), serde_json::json!({"id": 1, "name": "Ada"}));
}

#[tokio::test]
async fn typed_path_parameters() {
    let res = get(&app(), "/users/42").await;
    assert_eq!(res.json(), serde_json::json!({"id": 42, "name": "user-42"}));
    assert_eq!(get(&app(), "/pair/1/x").await.body, "1:x");
}

#[tokio::test]
async fn invalid_path_parameter_is_400() {
    let res = get(&app(), "/users/abc").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.error_code(), "INVALID_PATH_PARAMETER");
}

#[tokio::test]
async fn handler_errors_use_the_envelope() {
    let res = get(&app(), "/users/404").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(
        res.json(),
        serde_json::json!({"error": {"code": "USER_NOT_FOUND", "message": "The requested user does not exist"}})
    );
}

#[tokio::test]
async fn query_parameters() {
    let res = get(&app(), "/users?page=3&limit=50").await;
    assert_eq!(res.json(), serde_json::json!({"page": 3, "limit": 50}));
    let res = get(&app(), "/users").await;
    assert_eq!(res.json(), serde_json::json!({"page": 1, "limit": 20}));
    let res = get(&app(), "/users?page=minus-one").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.error_code(), "INVALID_QUERY");
}

#[tokio::test]
async fn malformed_json_inputs() {
    let res = send(&app(), json_request("POST", "/users", "{not json")).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.error_code(), "MALFORMED_JSON");

    let res = send(&app(), json_request("POST", "/users", r#"{"nom":"Ada"}"#)).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.error_code(), "INVALID_JSON");
    assert!(
        res.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("name")
    );

    let req = Request::post("/users")
        .body(Body::from(r#"{"name":"Ada"}"#))
        .unwrap();
    let res = send(&app(), req).await;
    assert_eq!(res.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(res.error_code(), "UNSUPPORTED_MEDIA_TYPE");
}

#[tokio::test]
async fn body_limit_from_config() {
    let router = App::new()
        .config(ServerConfig::new().max_body_size(16))
        .post("/users", create_user)
        .into_axum_router();
    let res = send(
        &router,
        json_request("POST", "/users", r#"{"name":"a very long name indeed"}"#),
    )
    .await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.error_code(), "PAYLOAD_TOO_LARGE");
    let res = send(&router, json_request("POST", "/users", r#"{"name":"A"}"#)).await;
    assert_eq!(res.status, StatusCode::CREATED);
}

#[tokio::test]
async fn shared_state_is_not_corrupted_under_concurrency() {
    #[derive(Clone)]
    struct AppState {
        hits: Arc<AtomicU64>,
        service_name: Arc<str>,
    }
    let state = AppState {
        hits: Arc::new(AtomicU64::new(0)),
        service_name: "customer-api".into(),
    };
    let router = App::new()
        .get("/stateless", || async { "registered before state" })
        .state(state.clone())
        .get("/hit", |State(s): State<AppState>| async move {
            s.hits.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            s.service_name.to_string()
        })
        .into_axum_router();

    let tasks: Vec<_> = (0..500)
        .map(|_| {
            let router = router.clone();
            tokio::spawn(async move { get(&router, "/hit").await.body })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap(), "customer-api");
    }
    assert_eq!(state.hits.load(Ordering::SeqCst), 500);
    assert_eq!(
        get(&router, "/stateless").await.body,
        "registered before state"
    );
}

#[tokio::test]
async fn typed_headers() {
    let router = App::new()
        .get(
            "/ua",
            |Header(UserAgent(ua)): Header<UserAgent>| async move { ua },
        )
        .get("/token", |token: Option<Header<BearerToken>>| async move {
            token
                .map(|Header(BearerToken(t))| t)
                .unwrap_or_else(|| "anonymous".into())
        })
        .into_axum_router();

    let req = Request::get("/ua")
        .header("user-agent", "kerniq-test")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, req).await.body, "kerniq-test");

    let res = get(&router, "/ua").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.error_code(), "MISSING_HEADER");

    assert_eq!(get(&router, "/token").await.body, "anonymous");
    let req = Request::get("/token")
        .header("authorization", "Bearer abc")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, req).await.body, "abc");
    let req = Request::get("/token")
        .header("authorization", "Basic abc")
        .body(Body::empty())
        .unwrap();
    let res = send(&router, req).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.error_code(), "INVALID_HEADER");
}

#[tokio::test]
async fn result_ext_hides_internal_details() {
    let router = App::new()
        .get("/db", || async {
            let failure: std::result::Result<(), std::io::Error> = Err(std::io::Error::other(
                "password=hunter2 rejected by db.internal",
            ));
            failure.or_internal()?;
            Ok::<_, Error>("unreachable")
        })
        .get("/opt", || async {
            None::<u8>.or_not_found("Nothing here").map(|_| "x")
        })
        .into_axum_router();
    let res = get(&router, "/db").await;
    assert_eq!(res.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(res.error_code(), "INTERNAL_ERROR");
    assert!(!res.body.contains("hunter2"));
    assert_eq!(get(&router, "/opt").await.status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "validation")]
mod validation {
    use super::*;
    use validator::Validate;

    #[derive(Deserialize, Validate)]
    struct Signup {
        #[validate(email(message = "must be a valid email address"))]
        email: String,
        #[validate(length(min = 8))]
        password: String,
    }

    #[tokio::test]
    async fn validated_json_reports_fields_without_echoing_values() {
        let router = App::new()
            .post(
                "/signup",
                |ValidatedJson(s): ValidatedJson<Signup>| async move { s.email },
            )
            .into_axum_router();
        let res = send(
            &router,
            json_request("POST", "/signup", r#"{"email":"nope","password":"short"}"#),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(res.error_code(), "VALIDATION_FAILED");
        let details = &res.json()["error"]["details"];
        assert_eq!(details["email"][0], "must be a valid email address");
        assert_eq!(details["password"][0], "failed `length` validation");
        assert!(
            !res.body.contains("short\""),
            "submitted values must not be echoed"
        );

        let ok = send(
            &router,
            json_request(
                "POST",
                "/signup",
                r#"{"email":"a@b.co","password":"long-enough"}"#,
            ),
        )
        .await;
        assert_eq!(ok.body, "a@b.co");
    }
}
