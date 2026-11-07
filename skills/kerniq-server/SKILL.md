---
name: kerniq-server
description: Build HTTP/JSON services and REST APIs in Rust with the Kerniq Server framework (crate `kerniq-server`, CLI `kerniq`). Use when creating a new Kerniq service, adding routes, handlers, extractors, shared state, middleware, authentication or role/permission authorization, configuration, background tasks, databases or message brokers, OpenAPI, or tests to a project that depends on `kerniq-server` or uses `kerniq_server::prelude`.
---

# Building services with Kerniq Server

Kerniq is a thin, Flask-style layer over Axum, Tower, Hyper and Tokio.
Handlers are plain Axum handlers, middleware is Tower middleware, and
`App::into_axum_router()` gives you an `axum::Router`. Prefer Kerniq's API
(`App`, `Router`, `Error`, built-in middleware) and fall back to Axum/Tower
only for things Kerniq does not wrap.

Requires Rust 1.88+ (edition 2024). Always start from
`use kerniq_server::prelude::*;`.

## 1. Create the project

Prefer the CLI; every template compiles, passes lint and passes its tests.

```sh
kerniq new my-service --template modular   # basic | rest | modular | production
cd my-service && kerniq dev                # run with auto-restart
```

| Template | Pick it for |
|---|---|
| `basic` | One endpoint, a spike or demo |
| `rest` | Small CRUD API in a few files |
| `modular` | Most real services: `config`, `state`, `error`, `models`, `services`, `routes` modules |
| `production` | Deployable service: TOML + env config, JSON logs, request IDs, security headers, CORS, compression, metrics, health probes, background task, Dockerfile |

Until Kerniq is on crates.io, add `--kerniq-path /path/to/kerniq-server` or
`--kerniq-git <url>`. Without the CLI, add:

```toml
[dependencies]
kerniq-server = { version = "0.1", features = [] } # see feature table
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }

[dev-dependencies]
http-body-util = "0.1"
serde_json = "1"
tower = { version = "0.5", features = ["util"] }
```

| Feature | Default | Enables |
|---|---|---|
| `json` | yes | `Json<T>`, `Created<T>` |
| `tracing` | yes | `Logger` middleware |
| `cors` | | `Cors` |
| `compression` | | `Compression` |
| `validation` | | `ValidatedJson`, `ValidatedQuery` (`validator` crate) |
| `openapi` | | `kerniq_server::openapi` (Utoipa + Swagger UI) |
| `websocket` | | `kerniq_server::extract::ws` |
| `config-file` | | `ServerConfig::from_file` (TOML) |
| `macros` | | `#[authorize(..)]` |
| `full` | | everything |

Enable only what the service uses.

## 2. Structure

For anything beyond a demo, follow the `modular` layout:

```
src/
  main.rs       # load config, build App, run
  config.rs     # config from env/file
  state.rs      # #[derive(Clone)] AppState
  error.rs      # domain errors + From<DomainError> for kerniq_server::Error
  models/       # serde types
  services/     # business logic, no HTTP types
  routes/       # one module per feature, each returning Router<AppState>
```

Keep an `app(config) -> App<AppState>` function separate from `main` so
tests can build the app without binding a port.

```rust
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::from_env()?; // defaults + KERNIQ_* overrides
    let address = config.address();
    app(config).run(address).await // graceful shutdown on Ctrl+C / SIGTERM
}

pub fn app(config: ServerConfig) -> App<AppState> {
    App::new()
        .config(config)
        .state(AppState::default())
        .merge(routes::router())
}
```

## 3. Routes and handlers

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .get("/health", health)
        .mount("/api/v1/users", users::router()) // prefix + scoped middleware
}

// users.rs
pub fn router() -> Router<AppState> {
    Router::new()
        .get("/", list)
        .post("/", create)
        .get("/{id}", show)        // `:id` also works
        .delete("/{id}", remove)
}

async fn show(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<User>> {
    Ok(Json(state.users.get(id).await?))
}

async fn create(State(state): State<AppState>, Json(input): Json<NewUser>) -> Result<Created<User>> {
    Ok(Created(state.users.create(input).await?))
}
```

- Methods: `get`, `post`, `put`, `patch`, `delete`, `head`, `options`, `trace`, plus
  `on`/`route` for anything else. `group(|r| { .. })` registers several
  routes on a router; `merge` combines routers at their own prefix.
- Return `Result<T>` (Kerniq's `Result<T, Error>`) where `T: IntoResponse`:
  `Json<T>`, `Created<T>` (201), `NoContent` (204), `Html`, `Redirect`,
  `&'static str`, `String`, `StatusCode`, tuples, `Response`.
- Extractors: `Path`, `Query`, `Json`, `State`, `Header<T>` (e.g.
  `BearerToken`, `UserAgent`), `HeaderMap`, `Extension`, `ConnectInfo`,
  `Bytes`, `Body`, and `ValidatedJson`/`ValidatedQuery` (feature
  `validation`, with `#[derive(Validate)]`). The body extractor must be the
  **last** parameter.
- Bad input (unparseable path, malformed JSON, failed validation) is
  rejected automatically with a JSON error; don't hand-validate types.

## 4. Errors

Every error renders as `{"error":{"code":"...","message":"...","details":...}}`.

```rust
Err(Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND"))
```

Constructors: `bad_request`, `unauthorized`, `forbidden`, `not_found`,
`conflict`, `unprocessable`, `too_many_requests`, `service_unavailable`,
`validation(details)`, `internal(source)`, `Error::new(status, code, msg)`.
Builders: `with_code`, `with_message`, `with_details`, `with_header`.

Rules:

- **Map domain errors once** with `impl From<MyError> for Error`, keep
  services free of HTTP types, and use `?` in handlers.
- **Unexpected failures** (DB, IO, third-party): `.or_internal()?`. It logs
  the cause and returns a generic 500, so internals never leak. Never put
  raw error strings from dependencies in a client-facing message.
- Prefer the prelude helpers over `match` boilerplate:
  `result.or_error(Error::...)`, `option.or_not_found("...")`,
  `option.or_error(Error::...)`.

## 5. State

```rust
#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,         // pools/clients are cheap Arc handles
    pub users: UserService,       // services hold Arc<...> internally
}
```

State is cloned per request: every field must be cheap to clone (`Arc`,
pools, clients). Use `tokio::sync::RwLock`/`Mutex` inside an `Arc` for
in-memory mutable data. `App::new().state(s)` turns `App` into
`App<AppState>`; routers that use `State<AppState>` are `Router<AppState>`.

## 6. Middleware

Middleware applies to every route of the app or router it is declared on,
regardless of where routes are declared. **The first declared is the
outermost.** A typical production order:

```rust
app.middleware(RequestId::new())
    .middleware(Logger::new())
    .middleware(metrics)               // Metrics::new(); serve metrics.registry()
    .middleware(SecurityHeaders::new())
    .middleware(Cors::default())       // feature `cors`
    .middleware(Compression::new())    // feature `compression`
    .middleware(Timeout::seconds(30))
    .middleware(RateLimit::per_second(50))
```

Also available: `BodyLimit`, `ConcurrencyLimit`. Router-level
`.middleware(..)` scopes to that router's routes only. Any Tower layer works
via `.layer(..)`.

## 7. Authentication and authorization

Kerniq provides the plumbing, not a provider: validate tokens with your own
JWT library, session store or IdP.

```rust
use kerniq_server::middleware::auth::{Auth, Authorities, Identity, Require};

#[derive(Clone)]
struct User { id: u64, roles: Vec<String>, permissions: Vec<String> }

impl Authorities for User {
    fn has_role(&self, role: &str) -> bool { self.roles.iter().any(|r| r == role) }
    fn has_permission(&self, p: &str) -> bool { self.permissions.iter().any(|x| x == p) }
}

let auth = Auth::bearer(|token: String| async move {
    verify_jwt(&token).await.map_err(|_| Error::unauthorized("Invalid token"))
});

async fn me(Identity(user): Identity<User>) -> String { format!("user {}", user.id) }
```

- Put `Auth` on a router that holds only protected routes, then `mount` it;
  keep public routes in another router. For other schemes implement
  `Authenticator`.
- `Identity<T>` returns 401 if no identity is present (fails closed);
  `Option<Identity<T>>` for optional auth.
- **Per router (role/permission):** `.middleware(Require::<User>::role("ADMIN"))`,
  also `any_role([..])`, `all_roles`, `permission`, `any_permission`,
  `all_permissions`, `check(|u: &User| ..)`, `policy::<P>()`, combined with
  `.and`, `.or`, `.message(..)`. Declare it after `Auth`. This replaces
  Spring-style `requestMatchers(..).hasRole(..)`: group routes by
  requirement instead of listing path patterns.
- **Per handler** (feature `macros`):
  `#[authorize(has_any_role("USER", "ADMIN"))]` on a free `async fn`;
  rules `has_role`, `has_any_role`, `has_all_roles`, `has_permission`,
  `has_any_permission`, `has_all_permissions`, `authenticated()`,
  `check(fn)` with `&&`, `||`, `!`. Add `identity = User` if the handler has
  no `Identity<User>` parameter.
- **Without macros:** implement `Policy<User>` and take
  `Authorized<User, MyPolicy>` as a parameter.
- Missing identity → 401, failed rule → 403 `FORBIDDEN`.

## 8. Configuration

`ServerConfig` covers host, port, workers, request timeout, max body size,
connection/concurrency limits, shutdown timeout and header read timeout.
Load with `ServerConfig::from_env()` (`KERNIQ_HOST`, `KERNIQ_PORT`,
`KERNIQ_REQUEST_TIMEOUT`, `KERNIQ_MAX_BODY_SIZE`, …) or
`ServerConfig::from_file("kerniq.toml")?.apply_env()?` (feature
`config-file`). Keep app-specific settings (database URL, secrets) in your
own config struct read from the environment; never put secrets in the TOML
file.

## 9. Databases, brokers and background work

Kerniq has no DB layer; use `sqlx`, `redis`, `mongodb`, `rdkafka`, `lapin`,
etc. (see `docs/integrations.md` in the Kerniq repo):

1. Connect once at startup, before `run`; fail fast on bad config.
2. Put the pool/client in `AppState`.
3. Map expected client errors (not found, unique violation) to 4xx;
   everything else `.or_internal()`.
4. Run consumers/pollers as `.background_task("name", |shutdown| async move { .. })`
   and exit when `shutdown.wait()` completes (use `tokio::select!`).
5. Close connections after `run(..).await` returns. By then requests have
   drained and background tasks have stopped.

Add a readiness probe (`/health/ready`) that pings dependencies and returns
`Error::service_unavailable(..)` when they are down.

## 10. Testing

Test through the router without binding a port:

```rust
#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use kerniq_server::axum::body::Body;
    use kerniq_server::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn creates_user() {
        let router = app(ServerConfig::default()).into_axum_router();
        let request = Request::post("/api/v1/users")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Ada","email":"ada@example.com"}"#))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), 201);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "Ada");
    }
}
```

Assert on status **and** the error `code` for failure cases
(`json["error"]["code"]`). Cover: success, validation failure, not found,
conflict, and 401/403 for protected routes.

## 11. Optional features

- **OpenAPI** (`openapi`): annotate handlers with `#[utoipa::path(..)]`,
  derive `OpenApi` for a doc struct and `ToSchema` for models, then
  `.merge(OpenApiDocs::new(ApiDoc::openapi()).swagger_ui("/docs").into_router())`
  (`use kerniq_server::openapi::OpenApiDocs;`). Serves `/openapi.json` and
  `/docs`.
- **WebSockets** (`websocket`): `kerniq_server::extract::ws`.
- **Axum interop:** `.mount_axum(path, axum_router)` to embed existing
  Axum routers; any Axum extractor or Tower layer works unchanged.

## 12. Before finishing

Run `kerniq check` (fmt, check, clippy `-D warnings`) and `kerniq test`, or
the equivalent `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
Then verify:

- [ ] Handlers return `Result<..>` with domain errors mapped via `From`; no
      dependency error text reaches clients (`.or_internal()`).
- [ ] Shared state is `Clone` and cheap to clone.
- [ ] Protected routes sit behind `Auth`; role rules use `Require` or
      `#[authorize]`; public routes are in a separate router.
- [ ] Middleware order is intentional (first declared = outermost).
- [ ] Config comes from env/file; no hard-coded secrets or addresses.
- [ ] Background tasks stop on shutdown; connections close after `run`.
- [ ] Tests exercise the router via `into_axum_router()` + `oneshot`.

## Reference

The Kerniq repository's `docs/` folder has the full guides: `getting-started`,
`routing`, `handlers`, `extractors`, `state-management`, `middleware`,
`authorization`, `configuration`, `runtime`, `integrations`, `openapi`,
`websockets`, `axum-interop`, `performance`, `architecture`.
