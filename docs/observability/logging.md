# Structured logging

Logging uses `tracing`: one pipeline for console output and OpenTelemetry
log export. There is no second logging system.

```rust,no_run
use kerniq_server::observability::prelude::*;

# async fn demo() -> Result<(), kerniq_server::observability::Error> {
let observability = Observability::builder()
    .service_name("payment-api")
    .logs(LogsConfig::json().filter("info,payment_api=debug"))
    .build()
    .await?;
# Ok(()) }
```

| Constructor | Output |
|---|---|
| `LogsConfig::json()` | JSON lines on stdout (production, log shippers) |
| `LogsConfig::pretty()` | multi-line, colored when a terminal (development) |
| `LogsConfig::compact()` | single-line, human-readable |
| `LogsConfig::otlp()` | OTLP export only |
| `.with_console(f)` / `.with_otlp(e)` | combine console and export |
| `.stderr()` | console on stderr |

## Writing logs

```rust
use tracing::{info, instrument};

#[instrument(skip(password))]
async fn authenticate(username: &str, password: &str) {
    info!(username, "Authentication request received");
    # let _ = password;
}
```

Fields are structured, not interpolated. Disabled levels cost a callsite
check: arguments are not formatted (`tests/logging.rs` verifies this with a
`Debug` impl that panics).

## The JSON format

```json
{"timestamp":"2026-11-08T13:16:18.459124Z","level":"INFO","target":"payment_api",
 "message":"payment received","amount_minor":1250,"currency":"USD",
 "trace_id":"20e58e139a8f2514b3c87560184d4b38","span_id":"9e4c8daa643bb397","trace_sampled":true,
 "spans":[{"name":"HTTP request","http.request.method":"POST","http.route":"/payments",
           "url.path":"/payments","url.scheme":"http","user_agent.original":"curl/8.7.1"}]}
```

* event fields are top-level keys,
* `trace_id` / `span_id` are present inside a traced span (tracing enabled),
* `spans` lists the enclosing spans from the root, with their fields
  (internal `otel.*` fields omitted),
* strings are escaped per RFC 8259; one object per line.

The formatter is `kerniq_observability::logging::json_layer`, usable in your
own subscriber.

## Filtering

`LogsConfig::filter` takes `EnvFilter` directives. Default: `RUST_LOG`, else
`info`. `LogsConfig::otlp_filter` sets a separate filter for export (for
example ship `warn` and print `info`).

The exporters' own dependencies (`hyper`, `tonic`, `h2`, `reqwest`,
`opentelemetry*`) are always excluded from OTLP log export, so exporting
cannot generate more logs to export.

## Non-blocking console output

Console output goes through a bounded buffer (16 384 lines) drained by one
thread, so a slow terminal, pipe or container log driver never blocks a
request. When the buffer is full, lines are dropped and counted
(`TelemetryStats::console_lines_dropped`, `kerniq_telemetry_items_total{
kerniq_signal="console_logs",kerniq_outcome="dropped"}`). Use
`.non_blocking(None)` to write synchronously instead, or
`.non_blocking(Some(n))` to resize.

## Log export and trace correlation

With OTLP log export and tracing both enabled, every exported record carries
the trace and span id of the span it was emitted in; Grafana (Loki → Tempo),
Elastic, Datadog and others link logs to traces from that.

## Sensitive data

Kerniq never logs request or response bodies, headers (other than the
user agent on spans) or query strings. Do not put passwords, tokens,
authorization headers, card numbers or payloads in fields; use
`#[instrument(skip(..))]` or `skip_all` for arguments.

## Existing subscribers

If the application already installs a subscriber, `build()` fails with
`Error::SubscriberAlreadySet` instead of silently replacing it. Compose
instead:

```rust,no_run
use kerniq_server::observability::prelude::*;
use tracing_subscriber::layer::SubscriberExt;

# async fn demo() -> Result<(), kerniq_server::observability::Error> {
let observability = Observability::builder()
    .logs(LogsConfig::json())
    .install_subscriber(false)
    .build()
    .await?;
let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
tracing::subscriber::set_global_default(subscriber).expect("no other subscriber");
# Ok(()) }
```
