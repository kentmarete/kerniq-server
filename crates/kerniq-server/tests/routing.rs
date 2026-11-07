mod common;

use common::{get, request, send};
use kerniq_server::prelude::*;
use kerniq_server::routing::methods;

async fn ok() -> &'static str {
    "ok"
}

fn echo_method(method: Method) -> String {
    method.to_string()
}

#[tokio::test]
async fn minimal_app_responds() {
    let router = App::new()
        .get("/", || async { "Hello from Kerniq" })
        .into_axum_router();
    let res = get(&router, "/").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, "Hello from Kerniq");
}

#[tokio::test]
async fn every_standard_method_can_be_registered() {
    let router = App::new()
        .get("/r", |m: Method| async move { echo_method(m) })
        .post("/r", |m: Method| async move { echo_method(m) })
        .put("/r", |m: Method| async move { echo_method(m) })
        .patch("/r", |m: Method| async move { echo_method(m) })
        .delete("/r", |m: Method| async move { echo_method(m) })
        .options("/r", |m: Method| async move { echo_method(m) })
        .trace("/r", |m: Method| async move { echo_method(m) })
        .on(
            Method::CONNECT,
            "/r",
            |m: Method| async move { echo_method(m) },
        )
        .into_axum_router();
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE"] {
        let res = send(&router, request(method, "/r")).await;
        assert_eq!(res.status, StatusCode::OK, "{method}");
        assert_eq!(res.body, method);
    }
    // A 2xx response to CONNECT establishes a tunnel and never has a body.
    assert_eq!(
        send(&router, request("CONNECT", "/r")).await.status,
        StatusCode::OK
    );
    // HEAD is answered by the GET handler, without a body.
    let res = send(&router, request("HEAD", "/r")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body.is_empty());
}

#[tokio::test]
async fn explicit_head_and_any() {
    let router = App::new()
        .head("/h", || async { StatusCode::NO_CONTENT })
        .any("/any", |m: Method| async move { echo_method(m) })
        .into_axum_router();
    assert_eq!(
        send(&router, request("HEAD", "/h")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&router, request("DELETE", "/any")).await.body,
        "DELETE"
    );
}

#[test]
#[should_panic(expected = "cannot route extension method")]
fn extension_methods_are_rejected_with_guidance() {
    let custom = Method::from_bytes(b"PURGE").unwrap();
    let _ = App::new().on(custom, "/cache", ok);
}

#[tokio::test]
async fn generic_route_with_method_router() {
    let router = App::new()
        .route(
            "/items",
            methods::get(|| async { "list" }).post(|| async { "create" }),
        )
        .into_axum_router();
    assert_eq!(get(&router, "/items").await.body, "list");
    assert_eq!(
        send(&router, request("POST", "/items")).await.body,
        "create"
    );
}

#[tokio::test]
async fn colon_and_brace_path_syntax_are_equivalent() {
    let router = App::new()
        .get(
            "/a/:id",
            |Path(id): Path<u32>| async move { format!("a{id}") },
        )
        .get(
            "/b/{id}",
            |Path(id): Path<u32>| async move { format!("b{id}") },
        )
        .get(
            "/files/*rest",
            |Path(rest): Path<String>| async move { rest },
        )
        .into_axum_router();
    assert_eq!(get(&router, "/a/7").await.body, "a7");
    assert_eq!(get(&router, "/b/8").await.body, "b8");
    assert_eq!(get(&router, "/files/x/y.txt").await.body, "x/y.txt");
}

#[tokio::test]
async fn unknown_route_is_json_404() {
    let router = App::new().get("/", ok).into_axum_router();
    let res = get(&router, "/missing").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.header("content-type"), Some("application/json"));
    assert_eq!(res.error_code(), "NOT_FOUND");
}

#[tokio::test]
async fn wrong_method_is_json_405_with_allow_header() {
    let router = App::new()
        .get("/users", ok)
        .post("/users", ok)
        .into_axum_router();
    let res = send(&router, request("DELETE", "/users")).await;
    assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(res.error_code(), "METHOD_NOT_ALLOWED");
    let allow = res.header("allow").expect("Allow header");
    assert!(allow.contains("GET") && allow.contains("POST"), "{allow}");
}

#[tokio::test]
async fn custom_fallback_survives_state_transition() {
    #[derive(Clone)]
    struct S;
    let router = App::new()
        .fallback(|| async { (StatusCode::IM_A_TEAPOT, "custom") })
        .state(S)
        .get("/", ok)
        .into_axum_router();
    let res = get(&router, "/nope").await;
    assert_eq!(res.status, StatusCode::IM_A_TEAPOT);
    assert_eq!(res.body, "custom");
}

#[tokio::test]
async fn route_groups_with_prefix() {
    let api = Router::new().prefix("/api/v1").group(|router| {
        router.get("/users", || async { "users" });
        router.post("/users", || async { "created" });
        router.get("/accounts", || async { "accounts" });
    });
    let app = App::new().merge(api);
    let paths: Vec<_> = app
        .routes()
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect();
    assert_eq!(
        paths,
        [
            "GET /api/v1/users",
            "POST /api/v1/users",
            "GET /api/v1/accounts"
        ]
    );
    let router = app.into_axum_router();
    assert_eq!(get(&router, "/api/v1/users").await.body, "users");
    assert_eq!(
        send(&router, request("POST", "/api/v1/users")).await.body,
        "created"
    );
    assert_eq!(get(&router, "/api/v1/accounts").await.body, "accounts");
    assert_eq!(get(&router, "/users").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn group_methods_can_be_chained() {
    let router = App::new()
        .group(|g| {
            g.get("/a", || async { "a" }).get("/b", || async { "b" });
        })
        .into_axum_router();
    assert_eq!(get(&router, "/b").await.body, "b");
}

#[tokio::test]
async fn independent_routers_mounted_at_paths() {
    let users = Router::new()
        .get("/", || async { "all users" })
        .get("/{id}", |Path(id): Path<u64>| async move {
            format!("user {id}")
        })
        .post("/", || async { "created user" });
    let accounts = Router::new().get("/", || async { "all accounts" });

    let router = App::new()
        .mount("/users", users)
        .mount("/accounts", accounts)
        .into_axum_router();

    assert_eq!(get(&router, "/users").await.body, "all users");
    assert_eq!(get(&router, "/users/42").await.body, "user 42");
    assert_eq!(
        send(&router, request("POST", "/users")).await.body,
        "created user"
    );
    assert_eq!(get(&router, "/accounts").await.body, "all accounts");
}

#[tokio::test]
async fn nested_mounts_and_prefix_composition() {
    let v1 = Router::new()
        .prefix("/v1")
        .get("/ping", || async { "pong" });
    let api = Router::new().mount("/api", v1);
    let app = App::new().mount("/svc", api);
    assert_eq!(app.routes()[0].path, "/svc/api/v1/ping");
    let router = app.into_axum_router();
    assert_eq!(get(&router, "/svc/api/v1/ping").await.body, "pong");
}

#[tokio::test]
async fn prefix_applies_to_routes_registered_before_it() {
    let r = Router::new().get("/x", || async { "x" }).prefix("/p");
    assert_eq!(r.routes()[0].path, "/p/x");
    let router = App::new().merge(r).into_axum_router();
    assert_eq!(get(&router, "/p/x").await.body, "x");
}

#[tokio::test]
async fn router_fallback_scoped_to_prefix() {
    let api = Router::new()
        .get("/known", ok)
        .fallback(|| async { (StatusCode::NOT_FOUND, "api fallback") });
    let router = App::new().mount("/api", api).into_axum_router();
    assert_eq!(get(&router, "/api/unknown").await.body, "api fallback");
    // Outside the prefix the app-wide JSON 404 still applies.
    assert_eq!(get(&router, "/other").await.error_code(), "NOT_FOUND");
}
