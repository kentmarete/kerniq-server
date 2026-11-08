# Getting started

## Installation

Kerniq Server requires Rust 1.88 or newer (stable).

Add the framework and Tokio to an existing project:

```toml
[dependencies]
kerniq-server = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }   # for JSON types
```

Or let the CLI create a project for you:

```sh
cargo install kerniq-cli                    # installs the `kerniq` command
kerniq new customer-api --template rest
cd customer-api
kerniq dev
```

Generated projects depend on `kerniq-server` from crates.io. To work against
unreleased changes, point them at a local checkout with
`--kerniq-path /path/to/kerniq-server` (or set `KERNIQ_SERVER_PATH`), or at a
repository with `--kerniq-git <url>`.

### Optional features

Default features are `json` and `tracing`. Enable more as needed:

```toml
kerniq-server = { version = "0.1", features = ["cors", "compression", "websocket", "openapi", "validation", "config-file"] }
```

`features = ["full"]` enables everything. `default-features = false` gives
the smallest build.

## Your first application

```rust,no_run
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::new()
        .get("/", || async { "Hello from Kerniq" });

    app.run("127.0.0.1:3000").await
}
```

```text
$ curl -i http://127.0.0.1:3000/
HTTP/1.1 200 OK
content-type: text/plain; charset=utf-8

Hello from Kerniq
```

`App::run` binds the address, serves until Ctrl+C or SIGTERM, drains
in-flight requests and returns.

## A small JSON API

```rust,no_run
use kerniq_server::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
}

async fn get_user(Path(id): Path<u64>) -> Result<Json<User>> {
    if id == 0 {
        return Err(Error::not_found("The requested user does not exist").with_code("USER_NOT_FOUND"));
    }
    Ok(Json(User { id, name: "Ada".into() }))
}

async fn create_user(Json(input): Json<CreateUser>) -> Created<User> {
    Created(User { id: 1, name: input.name })
}

#[tokio::main]
async fn main() -> Result<()> {
    App::new()
        .get("/users/{id}", get_user)
        .post("/users", create_user)
        .run("127.0.0.1:3000")
        .await
}
```

Malformed input is rejected with a structured error and no extra code:

```text
$ curl localhost:3000/users/abc
{"error":{"code":"INVALID_PATH_PARAMETER","message":"Invalid URL: Cannot parse `abc` to a `u64`"}}
```

## The CLI

| Command | What it does |
|---|---|
| `kerniq new <name> [--template basic\|rest\|modular\|production]` | Create a project (validated name, refuses to overwrite) |
| `kerniq dev [-- args]` | Build and run; rebuild and restart when `src/` or `*.toml` change. A failed build keeps the old server running. |
| `kerniq build` | `cargo build --release` |
| `kerniq check` | `cargo fmt --check`, `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings` |
| `kerniq test` | `cargo test` |

Templates:

* **basic** — one endpoint and a test.
* **rest** — CRUD handlers, models, an in-memory store, an integration test.
* **modular** — `config`, `state`, `error`, `models`, `services` and `routes`
  modules, with domain errors mapped to HTTP.
* **production** — file and environment configuration, structured (JSON)
  logging, request IDs, security headers, CORS, compression, metrics,
  liveness/readiness probes, a background task, graceful shutdown and a
  Dockerfile.

Every template compiles, passes `kerniq check` and passes its tests right
after generation.

## Next steps

* [Routing](routing.md) — methods, paths, groups and mounting
* [Handlers and errors](handlers.md)
* [Extractors](extractors.md)
* [Shared state](state-management.md)
* [Middleware](middleware.md)
* [Configuration](configuration.md)
* [Runtime and graceful shutdown](runtime.md)
