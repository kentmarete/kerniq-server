# Configuration reference

## Cargo features

`kerniq-server`:

| Feature | Enables | Pulls in |
|---|---|---|
| `observability` | `App::observability`, `kerniq_server::observability` | facade, HTTP layer (OpenTelemetry API) |
| `observability-metrics` | metrics SDK, HTTP metrics | `opentelemetry_sdk` |
| `observability-logs` | console logs, log bridge | `tracing-subscriber`, `tracing-appender` |
| `observability-tracing` | spans → OpenTelemetry | `tracing-opentelemetry` |
| `otlp` | OTLP gRPC + HTTP for enabled signals | `opentelemetry-otlp`, `tonic`, `reqwest` |
| `otlp-tls` | TLS for OTLP | rustls |
| `prometheus` | scrape endpoint | `opentelemetry-prometheus`, `prometheus` |
| `runtime-metrics` | Tokio and process metrics | `cpu-time`, `memory-stats` |
| `full-observability` | all except `otlp-tls` | |

`kerniq-observability` uses the same names without the prefix (`metrics`,
`logs`, `tracing`, `otlp`, `otlp-tls`, `prometheus`, `runtime`, `full`);
its defaults are `metrics`, `logs`, `tracing`.

Maturity: the OpenTelemetry Rust crates are pre-1.0 (0.33). Their metrics,
logs and traces APIs are what Kerniq builds on; experimental SDK features
(async-runtime processors, bound instruments, custom readers) are not used.

## Builder

| Method | Default |
|---|---|
| `service_name`, `service_version`, `service_namespace` | `OTEL_SERVICE_NAME`, else `unknown_service` |
| `environment` | — (`deployment.environment.name`) |
| `instance_id` | `<host>-<pid>-<start>` |
| `resource_attribute(k, v)` | plus `OTEL_RESOURCE_ATTRIBUTES` |
| `metrics(..)`, `logs(..)`, `traces(..)` | each signal off until configured |
| `install_subscriber(bool)` | `true`; fails if a subscriber exists |
| `shutdown_timeout(d)` | 5 s |
| `exclude_http_route(r)` | none (the Prometheus path is always excluded) |
| `http_scheme(s)` | `"http"`; set `"https"` behind TLS termination |

`build().await` must run inside a Tokio runtime and never creates one.

## Metrics

| `MetricsConfig` | Default |
|---|---|
| `prometheus()` / `otlp()` / `new()` | — |
| `endpoint(..)` | scrape path or Collector URL |
| `with_prometheus(cfg)`, `prometheus_listener(addr)` | — |
| `with_otlp(exporter)` | — |
| `export_interval(d)` | 60 s (OTLP only) |
| `delta_temporality(bool)` | cumulative |
| `cardinality_limit(n)` | 2 000 per instrument |
| `http(bool)` | `true` |
| `runtime(bool)` | `false` |
| `exporter(e)` | any `PushMetricExporter` |

### Runtime metrics

Observable instruments, read only at collection time (no polling task):

| Metric | Status |
|---|---|
| `kerniq.runtime.tokio.workers` | measured |
| `kerniq.runtime.tokio.alive_tasks` | measured |
| `kerniq.runtime.tokio.global_queue.depth` | measured |
| `process.cpu.time` | measured |
| `process.memory.usage`, `process.memory.virtual` | measured |
| `process.unix.file_descriptor.count` | measured (Unix only) |
| blocking pool, per-worker busy time, steals, poll times | **not collected**: need `tokio_unstable`; use `tokio-metrics` / `tokio-console` |

## Traces

| `TracesConfig` | Default |
|---|---|
| `otlp()` / `new()` | — |
| `endpoint`, `with_otlp`, `otlp_exporter` | — |
| `sampling_ratio(r)`, `sampling(Sampling)` | `AlwaysOn` |
| `parent_based(bool)` | `true` |
| `batch(BatchConfig)` | `BatchConfig::TRACES` |
| `filter(directives)` | `info` |
| `activate_otel_context(bool)` | `false` (on automatically when logs are exported) |
| `processor(p)` | any `SpanProcessor` |

## Logs

| `LogsConfig` | Default |
|---|---|
| `json()` / `pretty()` / `compact()` / `otlp()` | — |
| `with_console`, `without_console`, `stderr` | stdout |
| `filter(directives)` | `RUST_LOG`, else `info` |
| `non_blocking(Some(n) / None)` | 16 384 lines, lossy |
| `with_otlp`, `endpoint`, `otlp_filter` | — |
| `batch(BatchConfig)` | `BatchConfig::LOGS` |
| `processor(p)` | any `LogProcessor` |

## Resource limits and failure policy

| Resource | Bound | When exceeded |
|---|---|---|
| Span queue | `BatchConfig::max_queue_size` (2 048) per exporter | span dropped, counted |
| Log queue | `max_queue_size` (2 048) per exporter | record dropped, counted |
| Console buffer | 16 384 lines | line dropped, counted |
| Metric series | 2 000 attribute sets per instrument | folded into the overflow series |
| Export request | exporter timeout (10 s) | failed, retried per `Retry` |
| Retries | `Retry::RECOMMENDED`: 3, 100 ms → 1.6 s backoff | items counted as failed |
| Shutdown | `shutdown_timeout` (5 s) | returns, `ShutdownReport::timed_out` |
| Threads | one per pipeline + one console writer | — |

## Export statistics

`Observability::stats()` and the `kerniq.telemetry.items` /
`kerniq.telemetry.exports` metrics report, per signal: items offered,
exported and failed, and export calls and failures. While running,
`offered − exported − failed` (`unexported`) is queued plus dropped and is
bounded by the queue size unless spans are being dropped; after
`shutdown()`, it is exactly the number dropped. `ShutdownReport` contains the
final counts, the duration and any errors.

## Production deployment

* Prefer **Option B** with a Collector per node or per service (sidecar);
  keep vendor credentials in the Collector.
* Serve Prometheus on a **separate listener** and restrict it at the
  network layer.
* Set `service_name`, `service_version`, `environment`; let the platform
  add `k8s.*` / `cloud.*` attributes through `OTEL_RESOURCE_ATTRIBUTES` or
  the Collector's `k8sattributes` / `resourcedetection` processors.
* Sample traces (`sampling_ratio(0.01..0.1)` at high volume, or tail-sample
  in the Collector); never sample metrics.
* Ship `info` logs as JSON to stdout and let the platform collect them, or
  export `warn`+ over OTLP with `otlp_filter("warn")`.
* Size queues for bursts × export latency; watch
  `kerniq_telemetry_items_total` for a growing `offered − exported` gap.
* Give the Collector a `memory_limiter` (see `observability/otel-collector`).
* Set the platform's termination grace period above Kerniq's
  `shutdown_timeout` (server drain) plus the observability
  `shutdown_timeout`.
