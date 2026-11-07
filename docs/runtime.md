# Tokio runtime, lifecycle and graceful shutdown

Kerniq runs on Tokio and never implements its own executor. It also never
creates a runtime behind your back.

## Two execution modes

### You own the runtime (recommended)

```rust,no_run
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    App::new()
        .get("/", || async { "OK" })
        .run("0.0.0.0:3000")
        .await
}
```

Configure the runtime with Tokio's own attributes, for example
`#[tokio::main(worker_threads = 8)]`.

### Kerniq owns the runtime

```rust,no_run
use kerniq_server::prelude::*;

fn main() -> Result<()> {
    App::new()
        .config(ServerConfig::new().workers(4))
        .get("/", || async { "OK" })
        .run_blocking("0.0.0.0:3000")
}
```

`run_blocking` builds a multi-threaded runtime (honoring
`ServerConfig::workers`), runs the server, and shuts the runtime down.
Nested runtimes are impossible in Tokio, so calling `run_blocking` from
inside a runtime returns an error instead of panicking:

```rust
use kerniq_server::prelude::*;

# #[tokio::main(flavor = "current_thread")]
# async fn main() {
let err = App::new().run_blocking("127.0.0.1:0").unwrap_err();
assert!(err.to_string().contains("use `app.run(addr).await` instead"));
# }
```

| Method | Runtime | Use when |
|---|---|---|
| `run(addr).await` | caller's | normal applications |
| `serve(listener).await` | caller's | pre-bound sockets, tests on port 0 |
| `run_blocking(addr)` | Kerniq's | synchronous `main` |
| `into_axum_router()` | n/a | `axum::serve`, embedding, in-process tests |

## Startup

`run` resolves and binds the address. Bind failures are returned as errors
naming the address, for example
`failed to bind to "0.0.0.0:80": Permission denied`. The server logs
`kerniq server listening` with the bound address and, at `DEBUG`, every
registered route.

## Graceful shutdown

By default shutdown starts on **Ctrl+C** (SIGINT) or **SIGTERM** (Unix).
The sequence:

1. The listener is closed: new connections are refused.
2. Open connections are asked to finish. HTTP/1 connections close after the
   in-flight response; HTTP/2 connections receive `GOAWAY`.
3. After `ServerConfig::shutdown_timeout` (default 30 s), remaining
   connections are closed.
4. Background tasks are given up to `shutdown_timeout` to finish.
5. `run` returns `Ok(())`.

Configure or trigger it:

```rust,no_run
use std::time::Duration;
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Keep `_stop` somewhere; sending on it (or dropping it) stops the server.
    let (_stop, rx) = tokio::sync::oneshot::channel::<()>();

    let app = App::new()
        .config(ServerConfig::new().shutdown_timeout(Duration::from_secs(15)))
        // Replace the default Ctrl+C / SIGTERM trigger with any future:
        .shutdown_signal(async { let _ = rx.await; })
        .get("/", || async { "OK" });

    // Or trigger it from anywhere through a handle:
    let shutdown = app.shutdown_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        shutdown.trigger();
    });

    app.run("127.0.0.1:3000").await
}
```

### Readiness during shutdown

Put the `Shutdown` handle in your state so a readiness probe can fail as soon
as shutdown begins, and load balancers stop sending traffic while requests
drain:

```rust
use kerniq_server::prelude::*;

#[derive(Clone)]
struct AppState { shutdown: Shutdown }

async fn ready(State(state): State<AppState>) -> Result<&'static str> {
    if state.shutdown.is_triggered() {
        Err(Error::service_unavailable("Shutting down"))
    } else {
        Ok("READY")
    }
}

let app = App::new();
let shutdown = app.shutdown_handle();
let app = app.state(AppState { shutdown }).get("/health/ready", ready);
```

### Long-lived connections

WebSocket and streaming connections are upgraded out of Hyper's connection
management, so they must watch the `Shutdown` handle themselves. See
[WebSockets](websockets.md).

## Background tasks

```rust
use std::time::Duration;
use kerniq_server::prelude::*;

let app = App::new().background_task("refresh-rates", |shutdown: Shutdown| async move {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = interval.tick() => { /* refresh a cache, flush metrics, ... */ }
            () = shutdown.wait() => break, // finish promptly when asked
        }
    }
});
```

Tasks start when the server starts and are awaited during shutdown. This is
deliberately not a job queue: there are no retries, persistence or
distribution. For standalone use, `kerniq_server::runtime::BackgroundTasks`
offers the same tracking.

## Limits

| Setting | Layer | Effect |
|---|---|---|
| `max_connections` | accept loop | stop accepting at N open connections |
| `max_concurrent_requests` | middleware | at most N requests in handlers at once; others wait |
| `request_timeout` | middleware | `408` after the deadline; handler cancelled |
| `max_body_size` | extractors | `413` for larger bodies |
| `header_read_timeout` | Hyper | close connections that do not send headers in time (`0` disables) |

`workers` controls Tokio threads and only applies to `run_blocking`.
