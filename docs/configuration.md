# Configuration

`ServerConfig` holds server settings. Build it in code, read it from the
environment, load it from a TOML file (feature `config-file`), or layer these.

```rust
use std::time::Duration;
use kerniq_server::prelude::*;

let config = ServerConfig::new()
    .host("0.0.0.0")
    .port(3000)
    .workers(4)
    .request_timeout(Duration::from_secs(30))
    .max_body_size(10 * 1024 * 1024);

let address = config.address(); // "0.0.0.0:3000"
let app = App::new().config(config).get("/", || async { "OK" });
// app.run(address).await
```

## Settings

| Field | Default | Enforced by | Env var |
|---|---|---|---|
| `host` | `127.0.0.1` | your `run(config.address())` call | `KERNIQ_HOST` |
| `port` | `3000` | your `run(config.address())` call | `KERNIQ_PORT` |
| `workers` | CPU cores | Tokio, **`run_blocking` only** | `KERNIQ_WORKERS` |
| `request_timeout` | none | `Timeout` middleware (`408`) | `KERNIQ_REQUEST_TIMEOUT` (s) |
| `max_body_size` | 2 MiB | body extractors (`413`) | `KERNIQ_MAX_BODY_SIZE` (bytes) |
| `max_connections` | unlimited | accept loop | `KERNIQ_MAX_CONNECTIONS` |
| `max_concurrent_requests` | unlimited | global concurrency limit | `KERNIQ_MAX_CONCURRENT_REQUESTS` |
| `shutdown_timeout` | 30 s | graceful drain, background tasks | `KERNIQ_SHUTDOWN_TIMEOUT` (s) |
| `header_read_timeout` | 30 s (`0` = off) | Hyper (slow-header protection) | `KERNIQ_HEADER_READ_TIMEOUT` (s) |
| `tcp_nodelay` | `true` | accepted sockets | `KERNIQ_TCP_NODELAY` |
| `connect_info` | `true` | `ConnectInfo<SocketAddr>` for handlers and `RateLimit::by_ip` | `KERNIQ_CONNECT_INFO` |

Kerniq only exposes settings the underlying stack actually honors. In
particular there are three distinct kinds of concurrency:

* **worker threads** (Tokio): how many OS threads run tasks,
* **connections** (accept loop): how many TCP connections are open,
* **requests** (middleware): how many requests are being processed.

With `#[tokio::main]` you own the runtime, so `workers` does nothing and a
warning is logged. Set threads with `#[tokio::main(worker_threads = N)]`.

Durations from environment variables and files are in seconds and may be
fractional (`0.5`). Booleans accept `true/false/1/0/yes/no/on/off`. Invalid
values are reported with the variable name:
`invalid value "seventy" for KERNIQ_PORT: invalid digit found in string`.

## Environment

```rust,no_run
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::from_env()?; // defaults + KERNIQ_* overrides
    let address = config.address();
    App::new().config(config).get("/", || async { "OK" }).run(address).await
}
```

## Files

With `features = ["config-file"]`:

```toml
# kerniq.toml — durations in seconds, unknown keys are rejected
host = "0.0.0.0"
port = 8080
request_timeout = 30
max_concurrent_requests = 1024
shutdown_timeout = 20
```

```rust,no_run
use kerniq_server::prelude::*;

fn load() -> Result<ServerConfig> {
    // Defaults, then the file, then the environment.
    Ok(ServerConfig::from_file("kerniq.toml")?.apply_env()?)
}
```

## Application settings and secrets

`ServerConfig` covers the server only. Keep application settings (database
URLs, API keys) in your own configuration type, read secrets from the
environment or a secret manager at startup, and never commit them in
`kerniq.toml`. The `modular` and `production` CLI templates show this layout.
