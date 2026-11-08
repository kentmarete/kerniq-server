# Tutorial: build a service

This tutorial builds a small task-tracking API with the layout recommended
for real Kerniq services. You start from the `modular` template, tour what it
generates, then add a new `tasks` feature one layer at a time:

1. a **model**, the data the API sends and receives;
2. a **domain error** and its mapping to HTTP;
3. a **service** holding the business logic, with no HTTP types;
4. **state** wiring the service into the app;
5. a **router** with the handlers;
6. **tests** that call the router without opening a port.

The finished project is in
[`examples/task-api`](../examples/task-api), where CI builds, lints and tests
it; run its tests with `cargo test -p example-task-api`. It assumes you have read [Getting started](getting-started.md).

## 1. Create the project

```sh
cargo install kerniq-cli   # once: installs the `kerniq` command
kerniq new task-api --template modular
cd task-api
kerniq dev                 # run with auto-restart on http://127.0.0.1:8080
```

The project depends on `kerniq-server` from crates.io. To try unreleased
changes instead, add `--kerniq-path /path/to/kerniq-server` or
`--kerniq-git <url>` to `kerniq new`.

The generated `Cargo.toml` already has what this tutorial needs. The CLI
writes the latest Kerniq release; in Cargo, `"0.1.3"` means 0.1.3 or any
later 0.1.x:

```toml
[dependencies]
kerniq-server = { version = "0.1.3" }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
http-body-util = "0.1"
serde_json = "1"
tower = { version = "0.5", features = ["util"] }
```

## 2. The layout

```text
task-api/
├── Cargo.toml
└── src/
    ├── main.rs        # load config, build the App, run it
    ├── config.rs      # AppConfig: ServerConfig + app settings, from the environment
    ├── state.rs       # AppState, shared by every handler
    ├── error.rs       # domain errors and their HTTP mapping
    ├── models/        # serde types: what goes over the wire
    │   └── user.rs
    ├── services/      # business logic, no HTTP types
    │   └── users.rs
    └── routes/        # one module per feature, each returning Router<AppState>
        ├── mod.rs
        ├── health.rs
        └── users.rs
```

Each layer depends only on the layers below it:

```text
routes  ──►  services  ──►  models
  │             │
  └──► error ◄──┘        state holds the services; main wires everything
```

A request moves through the layers like this: the router matches the path,
the handler pulls typed input out of the request, calls a service, and turns
the result into a response. The service never sees a request or a status
code. That separation keeps business logic easy to test and lets you swap
the in-memory store for a database without touching handlers.

### `main.rs`: building the app

```rust,ignore
#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let address = config.server.address();
    println!("{} listening on http://{address}", config.service_name);

    app(config).run(address).await
}

pub fn app(config: AppConfig) -> App<AppState> {
    let state = AppState::new(&config);
    App::new()
        .config(config.server)
        .state(state)
        .merge(routes::router())
}
```

`app` is kept separate from `main` so tests can build the whole application
without binding a port. `run` shuts down gracefully on Ctrl+C or SIGTERM.

### `config.rs`: settings from the environment

```rust,ignore
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub service_name: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            server: ServerConfig::from_env()?,
            service_name: std::env::var("SERVICE_NAME")
                .unwrap_or_else(|_| env!("CARGO_PKG_NAME").to_owned()),
        })
    }
}
```

`ServerConfig::from_env` reads `KERNIQ_HOST`, `KERNIQ_PORT` and the other
`KERNIQ_*` variables (see [Configuration](configuration.md)). Put your own
settings, such as a database URL, beside it, and never hard-code secrets.

The template also includes a `users` feature (model, error, service and
routes). The rest of this tutorial adds a second feature, `tasks`, following
the same pattern.

## 3. Add a model

Models are plain serde types. Use separate types for what the client sends
(`NewTask`) and what the API returns (`Task`), so clients can't set fields
like `id`.

```rust,ignore
// src/models/task.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Deserialize)]
pub struct NewTask {
    pub title: String,
}
```

```rust,ignore
// src/models/mod.rs
pub mod task;
pub mod user;
```

## 4. Add a domain error

List what can go wrong in business terms, then decide **once**, in a `From`
implementation, how each case appears over HTTP. Handlers then only need `?`.

```rust,ignore
// src/error.rs (append below UserError)

/// Errors raised by the task service.
#[derive(Debug)]
pub enum TaskError {
    NotFound(u64),
    EmptyTitle,
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskError::NotFound(id) => write!(f, "task {id} not found"),
            TaskError::EmptyTitle => f.write_str("task title is empty"),
        }
    }
}

impl std::error::Error for TaskError {}

impl From<TaskError> for Error {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::NotFound(_) => {
                Error::not_found("The requested task does not exist").with_code("TASK_NOT_FOUND")
            }
            TaskError::EmptyTitle => {
                Error::unprocessable("`title` must not be empty").with_code("EMPTY_TITLE")
            }
        }
    }
}
```

Clients receive a consistent JSON body:

```json
{ "error": { "code": "TASK_NOT_FOUND", "message": "The requested task does not exist" } }
```

For failures you don't expect, such as a lost database connection, use
`.or_internal()?` instead of a variant. It logs the cause and returns a
generic 500 so internal details never reach the client. See
[Handlers & errors](handlers.md).

## 5. Add a service

The service holds the business rules. It takes and returns domain values and
fails with `TaskError`. It has no `Json`, `StatusCode` or extractors, so you
can test it and reuse it outside HTTP.

```rust,ignore
// src/services/tasks.rs
use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::TaskError;
use crate::models::task::{NewTask, Task};

/// Task business logic. No HTTP types here: it takes and returns domain
/// values and fails with `TaskError`.
#[derive(Debug, Clone, Default)]
pub struct TaskService {
    tasks: Arc<RwLock<BTreeMap<u64, Task>>>,
}

impl TaskService {
    pub async fn list(&self) -> Vec<Task> {
        self.tasks.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: u64) -> Result<Task, TaskError> {
        self.tasks
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(TaskError::NotFound(id))
    }

    pub async fn create(&self, input: NewTask) -> Result<Task, TaskError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }
        let mut tasks = self.tasks.write().await;
        let id = tasks.keys().next_back().copied().unwrap_or(0) + 1;
        let task = Task {
            id,
            title: title.to_owned(),
            done: false,
        };
        tasks.insert(id, task.clone());
        Ok(task)
    }

    pub async fn complete(&self, id: u64) -> Result<Task, TaskError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&id).ok_or(TaskError::NotFound(id))?;
        task.done = true;
        Ok(task.clone())
    }

    pub async fn delete(&self, id: u64) -> Result<(), TaskError> {
        self.tasks
            .write()
            .await
            .remove(&id)
            .map(|_| ())
            .ok_or(TaskError::NotFound(id))
    }
}
```

```rust,ignore
// src/services/mod.rs
pub mod tasks;
pub mod users;
```

The map sits behind `Arc<RwLock<..>>`, so cloning the service is cheap and
every clone shares the same data. To use a real database, keep the same
methods and replace the map with an `sqlx::PgPool` or a similar client (see
[Databases & brokers](integrations.md)). Handlers won't need to change.

## 6. Add it to the state

```rust,ignore
// src/state.rs
use std::sync::Arc;

use crate::config::AppConfig;
use crate::services::tasks::TaskService;
use crate::services::users::UserService;

/// Shared state. Cloned per request, so every field is cheap to clone.
#[derive(Debug, Clone)]
pub struct AppState {
    pub service_name: Arc<str>,
    pub users: UserService,
    pub tasks: TaskService,
}

impl AppState {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            service_name: config.service_name.as_str().into(),
            users: UserService::default(),
            tasks: TaskService::default(),
        }
    }
}
```

Kerniq clones the state for every request. Every field must be an `Arc`, a
connection pool, a client, or a service that wraps those. See
[Shared state](state-management.md).

## 7. Add the routes

Each feature has its own router. Handlers stay thin: extract the input, call
the service, wrap the result.

```rust,ignore
// src/routes/tasks.rs
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
```

Then mount it under a prefix:

```rust,ignore
// src/routes/mod.rs
mod health;
mod tasks;
mod users;

use kerniq_server::prelude::*;

use crate::state::AppState;

/// All routes of the application.
pub fn router() -> Router<AppState> {
    Router::new()
        .get("/health", health::health)
        .mount("/api/v1/users", users::router())
        .mount("/api/v1/tasks", tasks::router())
}
```

Things to notice:

- `?` turns a `TaskError` into an HTTP error through the `From` implementation
  from step 4.
- `Created` responds with `201`, `NoContent` with `204`.
- `Json<NewTask>` is the last parameter, because the body extractor must
  come last. Malformed JSON and non-numeric ids are rejected with a JSON
  error before your handler runs.
- `mount` adds the prefix. Middleware you add to `tasks::router()` (for
  example, authentication) applies only to these routes.

## 8. Try it

```sh
curl -s -X POST localhost:8080/api/v1/tasks \
     -H 'content-type: application/json' -d '{"title":"Write docs"}'
# {"id":1,"title":"Write docs","done":false}

curl -s -X POST localhost:8080/api/v1/tasks/1/complete
# {"id":1,"title":"Write docs","done":true}

curl -s localhost:8080/api/v1/tasks/42
# {"error":{"code":"TASK_NOT_FOUND","message":"The requested task does not exist"}}

curl -s -o /dev/null -w '%{http_code}\n' -X DELETE localhost:8080/api/v1/tasks/1
# 204
```

## 9. Test it

Tests build the real app with `app(..)` and send requests to it with
`oneshot`. No port is bound, so they run fast and can run in parallel. Put
them at the bottom of `src/routes/tasks.rs`:

```rust,ignore
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
```

For failures, check both the status code **and** `error.code`. Two
different errors can share a status code, and clients usually branch on
the code.

Then run the same checks CI would:

```sh
kerniq check   # cargo fmt --check, cargo check, cargo clippy -D warnings
kerniq test
```

## Checklist for a new feature

- [ ] `models/`: request and response types, with separate input types.
- [ ] `error.rs`: a domain error enum and one `From<..> for Error` mapping.
- [ ] `services/`: business logic returning `Result<_, DomainError>`, no HTTP types.
- [ ] `state.rs`: the service added to `AppState`, cheap to clone.
- [ ] `routes/`: a router per feature, thin handlers, mounted under a prefix.
- [ ] Tests for success, validation failure, not found, and 401/403 if protected.

## Next steps

- Protect routes with [Authorization](authorization.md): put `Auth` on the
  tasks router and require a role with `Require`.
- Add production middleware (request IDs, logging, timeouts, rate limits),
  in the order described in [Middleware](middleware.md).
- Replace the in-memory map with a database ([Databases & brokers](integrations.md)).
- Generate API docs with [OpenAPI](openapi.md).
- Start from `kerniq new <name> --template production` for a deployable
  service with all of the above preconfigured.
- Using Claude Code? Install the [Claude Code skill](../skills/kerniq-server/SKILL.md)
  so Claude follows this structure for you.
