# Middleware

Kerniq's middleware is Tower middleware. The built-in middleware is mostly
configuration over `tower` and `tower-http`, and anything implementing Tower's
`Layer` works.

```rust
use kerniq_server::prelude::*;

let app = App::new()
    .middleware(RequestId::new())
    .middleware(Logger::new())
    .middleware(Cors::default())
    .middleware(Timeout::seconds(30))
    .get("/", || async { "ok" });
```

## Execution order

**The first middleware declared is the outermost.** It sees the request
first and the response last:

```text
App::new().middleware(A).middleware(B).get("/", handler)

request  ──▶ A ──▶ B ──▶ handler
response ◀── A ◀── B ◀──┘
```

Middleware applies to **every route**, whether the route was registered
before or after the middleware. (Plain Axum's `Router::layer` only wraps
routes that already exist; Kerniq defers application to avoid that trap.)

Middleware implied by `ServerConfig` is added *inside* your middleware, in
this order: `request_timeout` → `max_concurrent_requests` →
`max_body_size`. Your logging therefore sees timeouts.

Recommended order, outermost first:

1. `RequestId` (so everything after it can log the id)
2. `Logger`
3. `Metrics`
4. `SecurityHeaders`, `Cors`
5. `Compression`
6. `RateLimit`, `Auth`
7. `Timeout` (or `ServerConfig::request_timeout`)

## Scope

* `App::middleware` / `App::layer` wrap every route **and** the 404 fallback.
* `Router::middleware` / `Router::layer` wrap only that router's routes:

```rust
use kerniq_server::prelude::*;

let admin = Router::new()
    .middleware(RateLimit::per_minute(30).by_ip())
    .get("/stats", || async { "stats" });

let app = App::new()
    .middleware(Logger::new())   // everything
    .get("/", || async { "public" })
    .mount("/admin", admin);     // only /admin/* is rate limited
```

Middleware runs after routing, so `MatchedPath` and path parameters are
available to it.

## Built-in middleware

| Middleware | Purpose | Feature |
|---|---|---|
| `Logger` | `tracing` span per request (`method`, `path`, `route`, `request_id`, `trace_id`) and a response event (`status`, `latency_ms`) | `tracing` (default) |
| `RequestId` | Keep or generate `x-request-id` (UUID v4), set it on the request and the response | — |
| `Timeout` | `408 REQUEST_TIMEOUT` after a deadline; cancels the handler | — |
| `BodyLimit` | Maximum body size for extractors (`413`) | — |
| `ConcurrencyLimit` | Global cap on in-flight requests (excess requests wait) | — |
| `Cors` | CORS headers and preflight | `cors` |
| `Compression` | gzip / deflate / brotli / zstd by `Accept-Encoding` | `compression` |
| `SecurityHeaders` | `nosniff`, `X-Frame-Options`, `Referrer-Policy`, COOP; optional HSTS and CSP | — |
| `RateLimit` | Token bucket, global / per IP / per header / custom key; `429` + `Retry-After` | — |
| `Auth` | Authentication extension point (see below) | — |
| `Metrics` | Prometheus counters, latency histogram, in-flight gauge | — |

### CORS

```rust
use kerniq_server::prelude::*;

let public = Cors::default(); // any origin, standard methods, any header, no credentials

let dashboard = Cors::new()
    .allow_origins(["https://dashboard.example.com"])
    .allow_methods([Method::GET, Method::POST])
    .allow_headers(["content-type", "authorization"])
    .allow_credentials(true);
```

### Rate limiting

```rust
use kerniq_server::prelude::*;

let per_ip = RateLimit::per_second(100).burst(200).by_ip();
let per_key = RateLimit::per_minute(1_000).by_header("x-api-key");
let behind_proxy = RateLimit::per_second(50).by_key(|req| {
    req.headers().get("x-real-ip").and_then(|v| v.to_str().ok()).map(str::to_owned)
});
```

Limits are kept in process memory, in 16 independently locked shards. For
limits shared across instances, enforce them at a gateway.

### Authentication

Kerniq provides the plumbing, not a provider: no JWT library or session store
is bundled.

```rust
use kerniq_server::prelude::*;
use kerniq_server::middleware::auth::{Auth, Identity};

#[derive(Clone)]
struct User { id: u64 }

let auth = Auth::bearer(|token: String| async move {
    // Verify the token with your JWT library, database or identity provider.
    if token == "valid" { Ok(User { id: 1 }) } else { Err(Error::unauthorized("Invalid token")) }
});

async fn me(Identity(user): Identity<User>) -> String {
    format!("user {}", user.id)
}

let protected = Router::new().middleware(auth).get("/me", me);
let app = App::new().mount("/api", protected);
```

For other schemes, implement `Authenticator`:

```rust
use kerniq_server::prelude::*;
use kerniq_server::http::request::Parts;
use kerniq_server::middleware::auth::{Auth, Authenticator};

#[derive(Clone)]
struct ApiKeys;

impl Authenticator for ApiKeys {
    type Identity = String; // the tenant

    async fn authenticate(&self, parts: &Parts) -> Result<String> {
        match parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
            Some("k-123") => Ok("tenant-a".into()),
            Some(_) => Err(Error::forbidden("Unknown API key")),
            None => Err(Error::unauthorized("API key required")),
        }
    }
}

let app = App::new().middleware(Auth::new(ApiKeys));
```

`Identity<T>` rejects with `401` when no identity is present, so a route that
is accidentally not covered by `Auth` fails closed. Use
`Option<Identity<T>>` for optional authentication.

To restrict routes by role or permission, see
[Authorization](authorization.md).

### Metrics

```rust
use kerniq_server::prelude::*;

let metrics = Metrics::new();
let registry = metrics.registry();

let app = App::new()
    .middleware(metrics)
    .get("/metrics", move || {
        let registry = registry.clone();
        async move { registry.render() }
    });
```

`route` labels use route templates (`/users/{id}`), so cardinality is bounded.

## Custom middleware

### Tower `Layer` + `Service`

Any Tower layer whose service handles `Request<Body>` and returns something
convertible to a response can be added with `.layer(..)`:

```rust
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use kerniq_server::http::{HeaderValue, Request, Response};
use kerniq_server::prelude::*;
use kerniq_server::tower::{Layer, Service};

#[derive(Clone)]
struct ResponseTimeLayer;

impl<S> Layer<S> for ResponseTimeLayer {
    type Service = ResponseTime<S>;
    fn layer(&self, inner: S) -> Self::Service {
        ResponseTime { inner }
    }
}

#[derive(Clone)]
struct ResponseTime<S> {
    inner: S,
}

impl<S, B, R> Service<Request<B>> for ResponseTime<S>
where
    S: Service<Request<B>, Response = Response<R>>,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let start = Instant::now();
        let future = self.inner.call(request);
        Box::pin(async move {
            let mut response = future.await?;
            let elapsed = format!("{}us", start.elapsed().as_micros());
            response.headers_mut().insert("x-response-time", HeaderValue::from_str(&elapsed).unwrap());
            Ok(response)
        })
    }
}

let app = App::new().layer(ResponseTimeLayer).get("/", || async { "ok" });
```

Because Axum applies layers per route, keep shared mutable state behind an
`Arc` inside the layer.

### Async functions with `axum::middleware::from_fn`

```rust
use kerniq_server::axum::{extract::Request, middleware::{from_fn, Next}, response::Response};
use kerniq_server::prelude::*;

async fn require_admin(request: Request, next: Next) -> std::result::Result<Response, Error> {
    if request.headers().get("x-admin").is_some_and(|v| v == "yes") {
        Ok(next.run(request).await)
    } else {
        Err(Error::forbidden("Administrators only"))
    }
}

let admin = Router::new().layer(from_fn(require_admin)).get("/", || async { "admin" });
let app = App::new().mount("/admin", admin);
```

### Bundling layers with the `Middleware` trait

Implement `Middleware` when one configurable unit should apply several
layers:

```rust
use kerniq_server::prelude::*;

struct InternalEndpoint {
    requests_per_minute: u32,
}

impl Middleware for InternalEndpoint {
    fn apply<S>(self, router: kerniq_server::axum::Router<S>) -> kerniq_server::axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let router = SecurityHeaders::new().apply(router);
        RateLimit::per_minute(self.requests_per_minute).by_ip().apply(router)
    }
}

let app = App::new().middleware(InternalEndpoint { requests_per_minute: 30 });
```

## Fusing middleware on hot paths

Each `.middleware(..)` call is one Axum `Router::layer`, and each adds one
service boxing per request (about two small allocations). On hot paths,
compose several into one layer with `ServiceBuilder`; every built-in
middleware has `into_layer()`:

```rust
use kerniq_server::prelude::*;
use kerniq_server::tower::ServiceBuilder;

let app = App::new()
    .layer(
        ServiceBuilder::new()
            .layer(RequestId::new().into_layer())       // outermost
            .layer(SecurityHeaders::new().into_layer())
            .layer(Timeout::seconds(30).into_layer()),  // innermost
    )
    .get("/", || async { "ok" });
```

See [performance](performance.md) for the measured difference.
