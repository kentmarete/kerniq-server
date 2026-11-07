# Axum and Tower interoperability

Kerniq is not a closed ecosystem. Every Kerniq app is an Axum router, every
Kerniq handler is an Axum handler, and every Kerniq middleware is Tower
middleware. Kerniq re-exports `axum`, `tower` and `http` at the versions it
uses (`kerniq_server::axum`, ...), so the types always match.

## What converts, mounts and composes

| You have | You want | Use |
|---|---|---|
| `App<S>` | `axum::Router` | `app.into_axum_router()` |
| Kerniq `Router<S>` | `axum::Router<S>` | `router.into_axum()` / `.into()` |
| `axum::Router<S>` | inside an `App<S>` / `Router<S>` | `.mount_axum(path, router)` (`"/"` merges) |
| `axum::Router<S>` | Kerniq `Router<S>` | `Router::from(axum_router)` |
| Axum handler | Kerniq route | pass it to `.get(..)` etc. |
| Axum extractor (`axum::Json`, `axum::extract::Path`, `axum-extra` types, ...) | in a Kerniq handler | use it as an argument |
| `axum::routing::MethodRouter` | Kerniq route | `.route(path, method_router)` |
| Tower `Layer` | app- or router-wide middleware | `.layer(layer)` |
| Axum `from_fn` middleware | middleware | `.layer(axum::middleware::from_fn(f))` |
| Kerniq middleware | in a plain Axum router or `ServiceBuilder` | `.into_layer()` on built-ins |

`into_axum_router()` includes routes, state, fallbacks and all middleware.
Lifecycle features (graceful shutdown, background tasks, connection limits)
belong to `App::run` and are not part of a router.

## Mounting an existing Axum router

```rust
use kerniq_server::prelude::*;
use kerniq_server::axum;

#[derive(Clone)]
struct AdminState { name: &'static str }

let existing_router = axum::Router::new()
    .route("/whoami", axum::routing::get(|axum::extract::State(s): axum::extract::State<AdminState>| async move { s.name }))
    .with_state(AdminState { name: "admin" });

let app = App::new()
    .get("/", || async { "kerniq" })
    .mount_axum("/admin", existing_router);
```

The mounted router keeps its own state, layers and handlers.

## Running Kerniq inside an Axum application

```rust,no_run
use kerniq_server::prelude::*;
use kerniq_server::axum;

#[tokio::main]
async fn main() -> Result<()> {
    let kerniq = App::new().get("/hello", || async { "from kerniq" });

    let app = axum::Router::new()
        .route("/status", axum::routing::get(|| async { "ok" }))
        .nest("/kerniq", kerniq.into_axum_router());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

## Native Axum handlers and extractors

```rust
use kerniq_server::prelude::*;
use kerniq_server::axum;

async fn axum_handler(axum::extract::Path(id): axum::extract::Path<u32>) -> axum::Json<u32> {
    axum::Json(id)
}

let app = App::new()
    .get("/items/{id}", axum_handler)
    .get("/ext", |axum::Extension(v): axum::Extension<&'static str>| async move { v })
    .layer(axum::Extension("injected"));
```

## Per-route Tower layers

```rust
use std::time::Duration;
use kerniq_server::prelude::*;
use kerniq_server::routing::methods;
use kerniq_server::tower::ServiceBuilder;

let app = App::new().route(
    "/reports",
    methods::get(|| async { "slow report" }).layer(
        ServiceBuilder::new()
            .layer(kerniq_server::axum::error_handling::HandleErrorLayer::new(
                |_: kerniq_server::tower::BoxError| async { StatusCode::GATEWAY_TIMEOUT },
            ))
            .timeout(Duration::from_secs(60)),
    ),
);
```

## Testing with Tower

Because the app becomes a `tower::Service`, tests can call it in-process
without a socket:

```rust
use kerniq_server::prelude::*;
use kerniq_server::axum::body::Body;
use kerniq_server::http::Request;
use tower::ServiceExt;

# #[tokio::main(flavor = "current_thread")]
# async fn main() {
let router = App::new().get("/", || async { "ok" }).into_axum_router();
let response = router
    .oneshot(Request::get("/").body(Body::empty()).unwrap())
    .await
    .unwrap();
assert_eq!(response.status(), StatusCode::OK);
# }
```

The integration tests in `crates/kerniq-server/tests/interop.rs` verify each
of these paths.
