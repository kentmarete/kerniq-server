# Shared state

Applications share resources such as database pools, configuration and
clients through typed state, not global variables.

```rust,no_run
use std::sync::Arc;
use kerniq_server::prelude::*;
use serde::Serialize;

#[derive(Clone)]
struct Database; // e.g. sqlx::PgPool, already an Arc-backed handle

#[derive(Clone)]
struct AppState {
    database: Database,
    service_name: Arc<str>,
}

#[derive(Serialize)]
struct HealthResponse {
    service: String,
    status: String,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        service: state.service_name.to_string(),
        status: "healthy".into(),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState {
        database: Database,
        service_name: "customer-api".into(),
    };

    App::new()
        .state(state)
        .get("/health", health)
        .run("127.0.0.1:3000")
        .await
}
```

## How it works

* `App::new()` is an `App<()>`. `.state(value)` turns it into `App<T>`.
* Handlers with `State<T>` can only be registered on an `App<T>` or a
  `Router<T>`. A mismatch is a compile error, not a runtime failure.
* Routes registered before `.state(..)` are kept.
* Routers inherit the state type from where they are mounted:
  `Router::new()` mounted into an `App<AppState>` is a `Router<AppState>`.

## Cloning cost

Axum clones the state each time a handler extracts it. Keep the state cheap
to clone:

* put expensive or large values behind `Arc` (`Arc<str>`, `Arc<Config>`),
* connection pools (`sqlx`, `deadpool`, `bb8`) and HTTP clients
  (`reqwest::Client`) are already `Arc` handles,
* or use `Arc<AppState>` as the state type.

## Mutable state

Use interior mutability scoped to the data that changes, never a lock around
the whole state:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use kerniq_server::prelude::*;

#[derive(Clone, Default)]
struct Counters {
    requests: Arc<AtomicU64>,
}

async fn count(State(c): State<Counters>) -> String {
    (c.requests.fetch_add(1, Ordering::Relaxed) + 1).to_string()
}

let app = App::new().state(Counters::default()).get("/count", count);
```

Prefer atomics or `tokio::sync::RwLock` for small shared structures, and do
not hold a `std::sync::Mutex` guard across an `.await`. Kerniq's test suite
runs hundreds of concurrent requests against shared state to check for lost
updates.

## Sub-state with `FromRef`

Handlers can extract just the part of the state they need:

```rust
use kerniq_server::axum::extract::FromRef;
use kerniq_server::prelude::*;

#[derive(Clone)]
struct Database;

#[derive(Clone)]
struct AppState {
    database: Database,
}

impl FromRef<AppState> for Database {
    fn from_ref(state: &AppState) -> Self {
        state.database.clone()
    }
}

async fn query(State(_db): State<Database>) -> &'static str {
    "ok"
}

let app = App::new().state(AppState { database: Database }).get("/q", query);
```
