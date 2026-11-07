mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use common::{get, send};
use http::{HeaderValue, Request};
use kerniq_server::extract::XRequestId;
use kerniq_server::middleware::auth::{Auth, Authenticator, Identity};
use kerniq_server::prelude::*;
use tower::{Layer, Service};

// ---------------------------------------------------------------------------
// A hand-written Tower middleware: records when requests enter and leave.
// ---------------------------------------------------------------------------

type Log = Arc<Mutex<Vec<String>>>;

#[derive(Clone)]
struct Tag {
    name: &'static str,
    log: Log,
}

impl<S> Layer<S> for Tag {
    type Service = TagService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        TagService {
            tag: self.clone(),
            inner,
        }
    }
}

#[derive(Clone)]
struct TagService<S> {
    tag: Tag,
    inner: S,
}

impl<S, B> Service<Request<B>> for TagService<S>
where
    S: Service<Request<B>> + Send,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<S::Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let Tag { name, log } = self.tag.clone();
        log.lock().unwrap().push(format!("{name}:in"));
        let future = self.inner.call(request);
        Box::pin(async move {
            let response = future.await;
            log.lock().unwrap().push(format!("{name}:out"));
            response
        })
    }
}

fn tag(name: &'static str, log: &Log) -> Tag {
    Tag {
        name,
        log: log.clone(),
    }
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

#[tokio::test]
async fn first_declared_middleware_is_outermost() {
    let log = Log::default();
    let router = App::new()
        .layer(tag("A", &log))
        .layer(tag("B", &log))
        .get("/", || async { "ok" })
        .into_axum_router();
    assert_eq!(get(&router, "/").await.body, "ok");
    assert_eq!(take(&log), ["A:in", "B:in", "B:out", "A:out"]);
}

#[tokio::test]
async fn middleware_applies_to_routes_registered_later_and_to_fallback() {
    let log = Log::default();
    let router = App::new()
        .layer(tag("A", &log))
        .get("/late", || async { "late" })
        .into_axum_router();
    get(&router, "/late").await;
    assert_eq!(take(&log), ["A:in", "A:out"]);
    assert_eq!(get(&router, "/missing").await.status, StatusCode::NOT_FOUND);
    assert_eq!(take(&log), ["A:in", "A:out"]);
}

#[tokio::test]
async fn router_scoped_middleware_only_wraps_its_routes() {
    let log = Log::default();
    let admin = Router::new()
        .layer(tag("admin", &log))
        .get("/stats", || async { "stats" });
    let router = App::new()
        .layer(tag("app", &log))
        .get("/public", || async { "public" })
        .mount("/admin", admin)
        .into_axum_router();

    get(&router, "/public").await;
    assert_eq!(take(&log), ["app:in", "app:out"]);
    get(&router, "/admin/stats").await;
    assert_eq!(take(&log), ["app:in", "admin:in", "admin:out", "app:out"]);
}

#[tokio::test]
async fn axum_and_tower_http_layers_compose() {
    async fn add_header(
        request: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let mut response = next.run(request).await;
        response
            .headers_mut()
            .insert("x-from-fn", HeaderValue::from_static("1"));
        response
    }
    let router = App::new()
        .layer(axum::middleware::from_fn(add_header))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            http::header::SERVER,
            HeaderValue::from_static("kerniq"),
        ))
        .get("/", || async { "ok" })
        .into_axum_router();
    let res = get(&router, "/").await;
    assert_eq!(res.header("x-from-fn"), Some("1"));
    assert_eq!(res.header("server"), Some("kerniq"));
}

#[tokio::test]
async fn timeout_middleware_returns_json_408() {
    let router = App::new()
        .middleware(Timeout::millis(50))
        .get("/slow", || async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "never"
        })
        .get("/fast", || async { "fast" })
        .into_axum_router();
    let res = get(&router, "/slow").await;
    assert_eq!(res.status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(res.error_code(), "REQUEST_TIMEOUT");
    assert_eq!(get(&router, "/fast").await.body, "fast");
}

#[tokio::test]
async fn request_ids_are_generated_propagated_and_extractable() {
    let router = App::new()
        .middleware(RequestId::new())
        .get(
            "/",
            |Header(XRequestId(id)): Header<XRequestId>| async move { id },
        )
        .into_axum_router();

    let res = get(&router, "/").await;
    let id = res.header("x-request-id").expect("generated id").to_owned();
    assert_eq!(id.len(), 36, "uuid: {id}");
    assert_eq!(res.body, id, "handler sees the same id");

    let req = Request::get("/")
        .header("x-request-id", "upstream-123")
        .body(Body::empty())
        .unwrap();
    let res = send(&router, req).await;
    assert_eq!(res.header("x-request-id"), Some("upstream-123"));
    assert_eq!(res.body, "upstream-123");
}

#[cfg(feature = "tracing")]
#[tokio::test]
async fn logger_does_not_alter_responses() {
    let router = App::new()
        .middleware(RequestId::new())
        .middleware(Logger::new())
        .get("/users/{id}", |Path(id): Path<u32>| async move {
            id.to_string()
        })
        .into_axum_router();
    let res = get(&router, "/users/5").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, "5");
}

#[cfg(feature = "cors")]
#[tokio::test]
async fn cors_preflight_and_simple_requests() {
    let router = App::new()
        .middleware(Cors::default())
        .post("/users", || async { "created" })
        .into_axum_router();
    let preflight = Request::builder()
        .method("OPTIONS")
        .uri("/users")
        .header("origin", "https://app.example.com")
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .unwrap();
    let res = send(&router, preflight).await;
    assert!(res.status.is_success(), "{}", res.status);
    assert_eq!(res.header("access-control-allow-origin"), Some("*"));
    assert!(
        res.header("access-control-allow-methods")
            .unwrap()
            .contains("POST")
    );

    let simple = Request::post("/users")
        .header("origin", "https://app.example.com")
        .body(Body::empty())
        .unwrap();
    let res = send(&router, simple).await;
    assert_eq!(res.body, "created");
    assert_eq!(res.header("access-control-allow-origin"), Some("*"));

    let strict = App::new()
        .middleware(Cors::new().allow_origins(["https://ok.example"]))
        .get("/", || async { "x" })
        .into_axum_router();
    let req = Request::get("/")
        .header("origin", "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&strict, req)
            .await
            .header("access-control-allow-origin"),
        None
    );
}

#[cfg(feature = "compression")]
#[tokio::test]
async fn compression_honours_accept_encoding() {
    let big = "kerniq ".repeat(2000);
    let router = App::new()
        .middleware(Compression::new())
        .get("/big", move || {
            let big = big.clone();
            async move { big }
        })
        .into_axum_router();
    let req = Request::get("/big")
        .header("accept-encoding", "gzip")
        .body(Body::empty())
        .unwrap();
    let res = send(&router, req).await;
    assert_eq!(res.header("content-encoding"), Some("gzip"));
    let res = get(&router, "/big").await;
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.body.len(), 14000);
}

#[tokio::test]
async fn security_headers_do_not_override_handler_headers() {
    let router = App::new()
        .middleware(SecurityHeaders::new().hsts(Duration::from_secs(31_536_000)))
        .get("/", || async { "ok" })
        .get("/framed", || async {
            ([("x-frame-options", "SAMEORIGIN")], "ok")
        })
        .into_axum_router();
    let res = get(&router, "/").await;
    assert_eq!(res.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(res.header("x-frame-options"), Some("DENY"));
    assert_eq!(
        res.header("strict-transport-security"),
        Some("max-age=31536000; includeSubDomains")
    );
    assert_eq!(
        get(&router, "/framed").await.header("x-frame-options"),
        Some("SAMEORIGIN")
    );
}

#[tokio::test]
async fn rate_limit_rejects_with_retry_after() {
    let router = App::new()
        .middleware(RateLimit::per_minute(2).by_header("x-api-key"))
        .get("/", || async { "ok" })
        .get("/other", || async { "other" })
        .into_axum_router();
    let with_key = |key: &str, path: &str| {
        Request::get(path)
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, with_key("a", "/")).await.status,
        StatusCode::OK
    );
    // The bucket is shared across routes.
    assert_eq!(
        send(&router, with_key("a", "/other")).await.status,
        StatusCode::OK
    );
    let res = send(&router, with_key("a", "/")).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.error_code(), "TOO_MANY_REQUESTS");
    let retry: u64 = res.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=30).contains(&retry), "{retry}");
    // Another key has its own bucket.
    assert_eq!(
        send(&router, with_key("b", "/")).await.status,
        StatusCode::OK
    );
}

#[derive(Clone, Debug, PartialEq)]
struct User {
    name: String,
}

#[tokio::test]
async fn bearer_auth_and_identity_extraction() {
    let protected = Router::new()
        .middleware(Auth::bearer(|token: String| async move {
            match token.as_str() {
                "good" => Ok(User { name: "ada".into() }),
                _ => Err(Error::unauthorized("Invalid token")),
            }
        }))
        .get(
            "/me",
            |Identity(user): Identity<User>| async move { user.name },
        );
    let router = App::new()
        .get("/public", |user: Option<Identity<User>>| async move {
            format!("anonymous={}", user.is_none())
        })
        .get(
            "/misconfigured",
            |Identity(user): Identity<User>| async move { user.name },
        )
        .mount("/api", protected)
        .into_axum_router();

    let res = get(&router, "/api/me").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.header("www-authenticate"), Some("Bearer"));

    let bad = Request::get("/api/me")
        .header("authorization", "Bearer nope")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, bad).await.status, StatusCode::UNAUTHORIZED);

    let good = Request::get("/api/me")
        .header("authorization", "Bearer good")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, good).await.body, "ada");

    assert_eq!(get(&router, "/public").await.body, "anonymous=true");
    // Identity on an unprotected route fails closed.
    assert_eq!(
        get(&router, "/misconfigured").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn custom_authenticator_trait() {
    #[derive(Clone)]
    struct ApiKeys;
    impl Authenticator for ApiKeys {
        type Identity = String;
        async fn authenticate(&self, parts: &http::request::Parts) -> Result<String> {
            match parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
                Some("k1") => Ok("tenant-1".to_owned()),
                Some(_) => Err(Error::forbidden("Unknown API key")),
                None => Err(Error::unauthorized("API key required")),
            }
        }
    }
    let router = App::new()
        .middleware(Auth::new(ApiKeys))
        .get(
            "/",
            |Identity(tenant): Identity<String>| async move { tenant },
        )
        .into_axum_router();
    assert_eq!(get(&router, "/").await.status, StatusCode::UNAUTHORIZED);
    let req = |key: &str| {
        Request::get("/")
            .header("x-api-key", key)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(send(&router, req("zz")).await.status, StatusCode::FORBIDDEN);
    assert_eq!(send(&router, req("k1")).await.body, "tenant-1");
}

#[tokio::test]
async fn metrics_use_route_templates() {
    let metrics = Metrics::new();
    let registry = metrics.registry();
    let router = App::new()
        .middleware(metrics)
        .get("/users/{id}", |Path(id): Path<u32>| async move {
            id.to_string()
        })
        .get("/metrics", move || {
            let registry = registry.clone();
            async move { registry.render() }
        })
        .into_axum_router();
    for id in [1, 2, 3] {
        get(&router, &format!("/users/{id}")).await;
    }
    get(&router, "/users/not-a-number").await;
    get(&router, "/nowhere").await;

    let text = get(&router, "/metrics").await.body;
    assert!(
        text.contains(r#"http_requests_total{method="GET",route="/users/{id}",status="200"} 3"#),
        "{text}"
    );
    assert!(
        text.contains(r#"http_requests_total{method="GET",route="/users/{id}",status="400"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"route="<unmatched>",status="404"} 1"#),
        "{text}"
    );
    assert!(
        text.contains(r#"http_request_duration_seconds_count{method="GET",route="/users/{id}"} 4"#),
        "{text}"
    );
    assert!(
        text.contains("http_requests_in_flight 1"),
        "the /metrics request itself is in flight: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_limit_is_global_across_routes() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let handler = {
        let (active, peak) = (active.clone(), peak.clone());
        move || {
            let (active, peak) = (active.clone(), peak.clone());
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                "ok"
            }
        }
    };
    let router = App::new()
        .config(ServerConfig::new().max_concurrent_requests(2))
        .get("/a", handler.clone())
        .get("/b", handler)
        .into_axum_router();
    let tasks: Vec<_> = (0..12)
        .map(|i| {
            let router = router.clone();
            tokio::spawn(async move {
                get(&router, if i % 2 == 0 { "/a" } else { "/b" })
                    .await
                    .status
            })
        })
        .collect();
    for t in tasks {
        assert_eq!(t.await.unwrap(), StatusCode::OK);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}
