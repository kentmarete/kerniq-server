# Kerniq Server

**A lightweight, asynchronous web framework for Rust: Flask-like simplicity on
top of Axum, Tower, Hyper and Tokio.**

> Simple by default. Powerful when needed. Extensible by design.

```rust,no_run
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new()
        .get("/", || async { "Hello from Kerniq" });

    app.run("127.0.0.1:3000").await
}
```

## Why Kerniq?

Axum is an excellent foundation, but a production service needs more than a
router: consistent error responses, ordered middleware that cannot silently
skip routes, configuration, graceful shutdown with bounded drain,
connection and concurrency limits, background tasks, and a project layout.
Kerniq packages those decisions behind a small, Flask-like API without
replacing anything underneath:

* **Every Kerniq handler is an Axum handler**, every middleware is Tower
  middleware, and every app converts to an `axum::Router`.
* **No second routing engine, no custom executor, no HTTP implementation.**
* **No mandatory database, ORM or authentication provider.**

Kerniq suits REST APIs, microservices, fintech backends, AI inference
endpoints, WebSocket services and internal enterprise systems.

## Installation

Kerniq requires Rust 1.88+.

```toml
[dependencies]
kerniq-server = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
```

> Kerniq 0.1.0 is not yet published to crates.io. Until it is, depend on a
> checkout: `kerniq-server = { path = "../kerniq-server/crates/kerniq-server" }`,
> or use `kerniq new --kerniq-path <checkout>`.

## A taste of the API

```rust,no_run
use std::sync::Arc;
use std::time::Duration;

use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct AppState {
    service_name: Arc<str>,
}

#[derive(Serialize)]
struct User { id: u64, name: String }

#[derive(Deserialize)]
struct CreateUser { name: String }

async fn get_user(Path(id): Path<u64>) -> Result<Json<User>> {
    if id == 0 {
        return Err(Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND"));
    }
    Ok(Json(User { id, name: "Ada".into() }))
}

async fn create_user(Json(input): Json<CreateUser>) -> Created<User> {
    Created(User { id: 1, name: input.name })
}

async fn health(State(state): State<AppState>) -> String {
    format!("{} is healthy", state.service_name)
}

#[tokio::main]
async fn main() -> Result<()> {
    let users = Router::new()
        .get("/{id}", get_user)
        .post("/", create_user);

    let config = ServerConfig::from_env()?
        .request_timeout(Duration::from_secs(30))
        .max_concurrent_requests(1024);
    let address = config.address();

    App::new()
        .config(config)
        .state(AppState { service_name: "customer-api".into() })
        .middleware(RequestId::new())
        .middleware(Logger::new())
        .middleware(SecurityHeaders::new())
        .get("/health", health)
        .mount("/users", users)
        .run(address)
        .await
}
```

Malformed input, unknown routes, timeouts and handler errors all share one
JSON error format:

```json
{"error":{"code":"USER_NOT_FOUND","message":"The requested user does not exist"}}
```

## Status codes and errors

**Error statuses** (400, 404, 409, …): return an `Error` from a handler that
returns `Result<T>`. Each common status has a constructor, and the builder
methods let you customize the response:

```rust
use kerniq_server::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct NewUser { email: String }

async fn create_user(Json(body): Json<NewUser>) -> Result<NoContent> {
    if body.email.is_empty() {
        return Err(Error::bad_request("Email is required")                 // 400
            .with_code("EMAIL_MISSING")                                     // custom machine code
            .with_details(serde_json::json!({ "field": "email" })));        // structured details
    }
    Ok(NoContent)
}

// Any other status:
fn payment_required() -> Error {
    Error::new(StatusCode::PAYMENT_REQUIRED, "LIMIT_EXCEEDED", "Monthly limit reached")
}
```

Constructors: `bad_request` (400), `unauthorized` (401), `forbidden` (403),
`not_found` (404), `method_not_allowed` (405), `timeout` (408), `conflict`
(409), `payload_too_large` (413), `unsupported_media_type` (415),
`unprocessable` (422), `validation` (422), `too_many_requests` (429),
`internal` (500), `service_unavailable` (503). `.with_header(...)` adds
headers such as `Retry-After`.

`ResultExt` and `OptionExt` make `?` convenient:

```rust
use kerniq_server::prelude::*;

fn lookup(id: u64) -> Result<String> {
    let raw: std::result::Result<Option<String>, std::io::Error> = Ok(Some(format!("user {id}")));
    let user = raw
        .or_internal()?                       // unexpected error → 500 (logged, never leaked)
        .or_not_found("User not found")?;     // None → 404
    Ok(user)
}
```

**Success or arbitrary statuses**: return `StatusCode`, or a `(StatusCode, body)`
tuple, optionally with headers in the middle. `Created(body)` (201) and
`NoContent` (204) are shortcuts.

```rust
use kerniq_server::prelude::*;

async fn accepted() -> (StatusCode, &'static str) {
    (StatusCode::ACCEPTED, "queued")                                       // 202
}

async fn created() -> (StatusCode, [(&'static str, &'static str); 1], &'static str) {
    (StatusCode::CREATED, [("location", "/users/7")], "created")           // 201 + header
}

async fn health() -> StatusCode {
    StatusCode::OK                                                          // empty body
}
```

Use `Error` for failures so clients always get the JSON envelope, and status
tuples for everything else. See [Handlers, responses and
errors](docs/handlers.md) for the full guide.

## Databases and message brokers

Kerniq does not wrap any database or broker client: use the crate you
already know (`sqlx`, `redis`, `mongodb`, `rdkafka`, `lapin`, …). Connect
at startup, share the client through state, run consumers as background
tasks that stop on shutdown, and close connections after `run` returns:

```rust,ignore
use kerniq_server::prelude::*;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Clone)]
struct AppState { db: PgPool }

async fn count_users(State(state): State<AppState>) -> Result<String> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(&state.db)
        .await
        .or_internal()?; // logged, never leaked; the client sees a generic 500
    Ok(n.to_string())
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?; // an unreachable database fails startup, not the first request

    App::new()
        .state(AppState { db: db.clone() })
        .get("/users/count", count_users)
        .background_task("orders-consumer", |shutdown: Shutdown| async move {
            // e.g. a Kafka or RabbitMQ consumer loop:
            // tokio::select! { msg = consumer.recv() => ..., () = shutdown.wait() => break }
            shutdown.wait().await;
        })
        .run("0.0.0.0:3000")
        .await?;

    db.close().await; // requests have drained and tasks have stopped
    Ok(())
}
```

[Databases and message brokers](docs/integrations.md) has complete examples
for PostgreSQL/MySQL/SQLite (`sqlx`), Redis (commands and pub/sub), MongoDB,
Kafka (producer and consumer) and RabbitMQ (publisher and consumer), plus
error mapping, health checks, pool sizing and testing.

## Features

| Area | What you get |
|---|---|
| Routing | `get/post/put/patch/delete/options/head/any/on`, `{id}` or `:id` params, prefixes, groups, mounting, nested fallbacks |
| Extractors | `Path`, `Query`, `Json`, `State`, `Header<T>`, `Body`, plus any Axum extractor |
| Errors | One `Error` type, JSON envelope, internal details logged and never leaked, domain errors via `From` |
| State | Typed `App<S>`, `FromRef` sub-states, no globals |
| Middleware | Ordered (first declared = outermost), applies to all routes, Tower-native, app- or router-scoped |
| Built-in middleware | Logger, RequestId, Timeout, BodyLimit, ConcurrencyLimit, Cors, Compression, SecurityHeaders, RateLimit, Auth, Metrics (Prometheus) |
| Lifecycle | Graceful shutdown (SIGINT/SIGTERM), bounded drain, connection limits, background tasks, `run` / `run_blocking` |
| Configuration | Builder, `KERNIQ_*` environment variables, TOML files |
| Optional | WebSockets, OpenAPI + Swagger UI (Utoipa), validation (`validator`) |
| Interop | `into_axum_router`, `mount_axum`, native Axum handlers, extractors and layers |
| CLI | `kerniq new` (basic, rest, modular, production), `dev` (auto-restart), `build`, `check`, `test` |

Cargo features: `json` and `tracing` (default), `cors`, `compression`,
`websocket`, `openapi`, `validation`, `config-file`, `full`. With
`default-features = false` the dependency tree is about half the size of the
full one.

## CLI

```sh
cargo install --path crates/kerniq-cli
kerniq new customer-api --template production --kerniq-path .
cd customer-api
kerniq dev      # run, rebuild and restart on change
kerniq check    # fmt --check, check, clippy -D warnings
kerniq test
```

Every template compiles, passes `kerniq check`, and passes its own tests
immediately after generation.

## Architecture

```text
kerniq-server  ── App, prelude, re-exports (the crate you depend on)
├── kerniq-core        Error model, ServerConfig
├── kerniq-http        Extractors and responses
├── kerniq-routing     Router, groups, Middleware composition
├── kerniq-middleware  Production middleware (tower / tower-http)
├── kerniq-runtime     Accept loop, graceful shutdown, background tasks
└── kerniq-openapi     Utoipa documents + Swagger UI   (optional)
kerniq-cli             Independent scaffolding tool
```

See [docs/architecture.md](docs/architecture.md) for the design decisions,
including where the specification was adapted to Rust's type system.

## Performance

Kerniq adds a thin layer over Axum, and its overhead is measured, not
assumed. On the benchmark machine (Apple M3 Pro), Kerniq matched native Axum
on allocations and dispatch time for routing, JSON and state, and its fused
middleware stack allocated less than the equivalent tower-http stack. Over
real HTTP it was within a few percent of `axum::serve`. The one measurable
default cost, slow-header protection (about 3.5% on a trivial endpoint), is
documented and can be switched off. Methodology, raw results and limitations
are in [docs/performance.md](docs/performance.md). Run
`benches/run-http.sh` on your own hardware.

## Documentation

* [Getting started](docs/getting-started.md) — installation, first app, CLI
* [Routing](docs/routing.md)
* [Handlers, responses and errors](docs/handlers.md)
* [Extractors](docs/extractors.md) (including validation)
* [Shared state](docs/state-management.md)
* [Middleware](docs/middleware.md) (including custom middleware)
* [Axum and Tower interoperability](docs/axum-interop.md)
* [Runtime, lifecycle and graceful shutdown](docs/runtime.md)
* [WebSockets](docs/websockets.md)
* [Configuration](docs/configuration.md)
* [Databases and message brokers](docs/integrations.md) (PostgreSQL, MySQL,
  SQLite, Redis, MongoDB, Kafka, RabbitMQ)
* [OpenAPI](docs/openapi.md)
* [Performance and benchmarking](docs/performance.md)
* [Architecture](docs/architecture.md)

Runnable examples live in [`examples/`](examples): `hello-world`,
`rest-api`, `shared-state`, `route-groups`, `middleware`,
`custom-middleware`, `axum-integration`, `websocket`, `graceful-shutdown`,
`openapi`, `validation` and `run-blocking`:

```sh
cargo run -p example-rest-api
```

Every Rust snippet in this README and in `docs/` is compiled as a doctest,
except the third-party integration examples (marked `ignore`), which need
crates Kerniq does not depend on.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every change must pass:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
