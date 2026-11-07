//! Equivalent Kerniq and Axum applications used by every benchmark.
//!
//! Each pair answers the same routes with the same handlers and, where
//! middleware is involved, the same Tower layers (Kerniq's middleware is
//! built on these layers, so the comparison measures Kerniq's composition
//! overhead, not different middleware implementations).

use std::time::Duration;

use axum::routing::get;
use http::{HeaderName, HeaderValue};
use kerniq_server::prelude::*;
use serde::Serialize;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

/// The JSON payload returned by the JSON endpoints.
#[derive(Serialize, Clone)]
pub struct Account {
    pub id: u64,
    pub owner: &'static str,
    pub currency: &'static str,
    pub balance_minor: i64,
    pub tags: [&'static str; 3],
}

pub const ACCOUNT: Account = Account {
    id: 42,
    owner: "Ada Lovelace",
    currency: "USD",
    balance_minor: 1_234_567,
    tags: ["retail", "verified", "priority"],
};

async fn hello() -> &'static str {
    "Hello, World!"
}

async fn kerniq_json() -> Json<Account> {
    Json(ACCOUNT)
}

async fn axum_json() -> axum::Json<Account> {
    axum::Json(ACCOUNT)
}

/// Minimal GET endpoint, Kerniq.
pub fn kerniq_minimal() -> axum::Router {
    App::new().get("/", hello).into_axum_router()
}

/// Minimal GET endpoint, Axum.
pub fn axum_minimal() -> axum::Router {
    axum::Router::new().route("/", get(hello))
}

/// JSON endpoint with a path parameter, Kerniq.
pub fn kerniq_json_app() -> axum::Router {
    App::new()
        .get("/accounts/{id}", |Path(_id): Path<u64>| kerniq_json())
        .into_axum_router()
}

/// JSON endpoint with a path parameter, Axum.
pub fn axum_json_app() -> axum::Router {
    axum::Router::new().route(
        "/accounts/{id}",
        get(|axum::extract::Path(_id): axum::extract::Path<u64>| axum_json()),
    )
}

/// A 50-route table plus the measured route, Kerniq (routing cost).
pub fn kerniq_many_routes() -> axum::Router {
    let mut app = App::new();
    for i in 0..50 {
        app = app.get(&format!("/resource{i}/{{id}}"), hello);
    }
    app.get("/accounts/{id}", |Path(_id): Path<u64>| kerniq_json())
        .into_axum_router()
}

/// A 50-route table plus the measured route, Axum.
pub fn axum_many_routes() -> axum::Router {
    let mut router = axum::Router::new();
    for i in 0..50 {
        router = router.route(&format!("/resource{i}/{{id}}"), get(hello));
    }
    router.route(
        "/accounts/{id}",
        get(|axum::extract::Path(_id): axum::extract::Path<u64>| axum_json()),
    )
}

const SECURITY_HEADERS: [(&str, &str); 4] = [
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "no-referrer"),
    ("cross-origin-opener-policy", "same-origin"),
];

/// Request IDs + security headers + 30 s timeout, Kerniq middleware.
pub fn kerniq_middleware() -> axum::Router {
    App::new()
        .middleware(RequestId::new())
        .middleware(SecurityHeaders::new())
        .middleware(Timeout::seconds(30))
        .get("/", hello)
        .into_axum_router()
}

/// The same Kerniq middleware fused into one layer with `ServiceBuilder`.
pub fn kerniq_middleware_fused() -> axum::Router {
    App::new()
        .layer(
            ServiceBuilder::new()
                .layer(RequestId::new().into_layer())
                .layer(SecurityHeaders::new().into_layer())
                .layer(Timeout::seconds(30).into_layer()),
        )
        .get("/", hello)
        .into_axum_router()
}

/// The same stack built from tower-http layers on a plain Axum router.
pub fn axum_middleware() -> axum::Router {
    let x_request_id = HeaderName::from_static("x-request-id");
    let [(n1, v1), (n2, v2), (n3, v3), (n4, v4)] = SECURITY_HEADERS;
    axum::Router::new().route("/", get(hello)).layer(
        ServiceBuilder::new()
            .layer(SetRequestIdLayer::new(
                x_request_id.clone(),
                MakeRequestUuid,
            ))
            .layer(PropagateRequestIdLayer::new(x_request_id))
            .layer(SetResponseHeaderLayer::if_not_present(
                HeaderName::from_static(n1),
                HeaderValue::from_static(v1),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                HeaderName::from_static(n2),
                HeaderValue::from_static(v2),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                HeaderName::from_static(n3),
                HeaderValue::from_static(v3),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                HeaderName::from_static(n4),
                HeaderValue::from_static(v4),
            ))
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                Duration::from_secs(30),
            )),
    )
}

/// All scenarios by name: (scenario, kerniq, axum, request path).
pub type Builder = fn() -> axum::Router;
pub const SCENARIOS: &[(&str, Builder, Builder, &str)] = &[
    ("minimal", kerniq_minimal, axum_minimal, "/"),
    ("json", kerniq_json_app, axum_json_app, "/accounts/42"),
    (
        "routes-50",
        kerniq_many_routes,
        axum_many_routes,
        "/accounts/42",
    ),
    ("middleware", kerniq_middleware, axum_middleware, "/"),
    (
        "middleware-fused",
        kerniq_middleware_fused,
        axum_middleware,
        "/",
    ),
];

/// Sends one request through `router` in-process and consumes the body.
pub async fn dispatch(router: &axum::Router, path: &'static str) -> usize {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let request = http::Request::get(path)
        .body(axum::body::Body::empty())
        .expect("valid request");
    let response = router.clone().oneshot(request).await.expect("infallible");
    assert!(response.status().is_success());
    response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .len()
}
