//! Proof that Kerniq stays open to the Axum and Tower ecosystems.

mod common;

use std::time::Duration;

use axum::extract::{Extension, FromRef};
use common::{get, json_request, send};
use kerniq_server::prelude::*;

#[tokio::test]
async fn kerniq_app_converts_to_axum_router_and_runs_under_axum_serve() {
    let router: axum::Router = App::new()
        .get("/", || async { "from kerniq" })
        .into_axum_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("from kerniq"), "{response}");
    server.abort();
}

#[tokio::test]
async fn existing_axum_router_with_its_own_state_and_layers_can_be_mounted() {
    #[derive(Clone)]
    struct AdminState {
        name: &'static str,
    }
    let admin = axum::Router::new()
        .route(
            "/whoami",
            axum::routing::get(
                |axum::extract::State(s): axum::extract::State<AdminState>| async move { s.name },
            ),
        )
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-store"),
        ))
        .with_state(AdminState { name: "axum-admin" });

    let router = App::new()
        .get("/", || async { "kerniq" })
        .mount_axum("/admin", admin)
        .into_axum_router();
    let res = get(&router, "/admin/whoami").await;
    assert_eq!(res.body, "axum-admin");
    assert_eq!(res.header("cache-control"), Some("no-store"));
    assert_eq!(get(&router, "/").await.body, "kerniq");
}

#[tokio::test]
async fn native_axum_handlers_extractors_and_responses() {
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Item {
        id: u32,
    }
    let router = App::new()
        // Axum's own Json and Path extractors work unchanged.
        .post(
            "/axum-json",
            |axum::Json(item): axum::Json<Item>| async move { axum::Json(item) },
        )
        .get(
            "/axum-path/{id}",
            |axum::extract::Path(id): axum::extract::Path<u32>| async move { id.to_string() },
        )
        // Axum response types.
        .get("/redirect", || async {
            axum::response::Redirect::temporary("/elsewhere")
        })
        // Extension values inserted by a layer.
        .get(
            "/ext",
            |Extension(v): Extension<&'static str>| async move { v },
        )
        .layer(axum::Extension("from-extension"))
        .into_axum_router();

    assert_eq!(
        send(&router, json_request("POST", "/axum-json", r#"{"id":5}"#))
            .await
            .body,
        r#"{"id":5}"#
    );
    assert_eq!(get(&router, "/axum-path/9").await.body, "9");
    assert_eq!(
        get(&router, "/redirect").await.status,
        StatusCode::TEMPORARY_REDIRECT
    );
    assert_eq!(get(&router, "/ext").await.body, "from-extension");
}

#[tokio::test]
async fn kerniq_router_nests_inside_a_larger_axum_app() {
    let users: Router = Router::new()
        .prefix("/users")
        .get("/", || async { "users" });
    let axum_app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "healthy" }))
        .nest("/api", users.into_axum());
    assert_eq!(get(&axum_app, "/api/users").await.body, "users");
    assert_eq!(get(&axum_app, "/health").await.body, "healthy");
}

#[tokio::test]
async fn substate_extraction_with_from_ref() {
    #[derive(Clone)]
    struct AppState {
        db: Db,
    }
    #[derive(Clone)]
    struct Db(&'static str);
    impl FromRef<AppState> for Db {
        fn from_ref(state: &AppState) -> Self {
            state.db.clone()
        }
    }
    let router = App::new()
        .state(AppState { db: Db("postgres") })
        .get("/db", |State(Db(name)): State<Db>| async move { name })
        .into_axum_router();
    assert_eq!(get(&router, "/db").await.body, "postgres");
}

#[tokio::test]
async fn axum_method_router_with_per_route_tower_layer() {
    use kerniq_server::routing::methods;
    use tower::ServiceBuilder;

    let router = App::new()
        .route(
            "/limited",
            methods::get(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "slow"
            })
            .layer(
                ServiceBuilder::new()
                    .layer(axum::error_handling::HandleErrorLayer::new(
                        |_: tower::BoxError| async { StatusCode::GATEWAY_TIMEOUT },
                    ))
                    .timeout(Duration::from_millis(50)),
            ),
        )
        .get("/unlimited", || async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            "fine"
        })
        .into_axum_router();
    assert_eq!(
        get(&router, "/limited").await.status,
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(get(&router, "/unlimited").await.body, "fine");
}
