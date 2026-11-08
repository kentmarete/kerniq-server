# Observability (Kerniq Observe)

Kerniq Observe gives a Kerniq application metrics, structured logs and
distributed traces through one builder, on top of OpenTelemetry, `tracing`
and Prometheus. It is vendor-neutral: the application speaks OpenTelemetry
(OTLP) or exposes Prometheus metrics, and the choice of backend — Prometheus
and Grafana, Datadog, Elastic, Splunk, Sumo Logic, Azure Monitor, Jaeger,
Tempo, Loki — is deployment configuration, not code.

```toml
[dependencies]
kerniq-server = { version = "0.1", features = ["full-observability"] }
```

```rust,no_run
use kerniq_server::observability::prelude::*;
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let observability = Observability::builder()
        .service_name("payment-api")
        .service_version("1.0.0")
        .environment("production")
        .metrics(MetricsConfig::prometheus())           // GET /metrics
        .logs(LogsConfig::json())                       // JSON lines on stdout
        .traces(TracesConfig::otlp().sampling_ratio(0.10)) // 10 % of traces → Collector
        .build()
        .await?;

    App::new()
        .observability(observability)
        .get("/health", || async { "OK" })
        .run("0.0.0.0:3000")
        .await
}
```

That is the whole integration. Every request is now measured
(`http.server.request.duration`, active requests, body sizes, time to first
byte), traced (one span per request, continuing the caller's W3C
`traceparent`), and every log line inside it carries the `trace_id`.

## What you get

| Signal | How | Guide |
|---|---|---|
| HTTP metrics | automatic, OpenTelemetry semantic conventions | [Prometheus](prometheus.md) |
| Application metrics | OpenTelemetry `Meter` API, no wrapper | [Custom metrics](custom-metrics.md) |
| Runtime metrics | Tokio + process, optional | [Configuration](configuration.md#runtime-metrics) |
| Structured logs | `tracing` events, JSON or human-readable | [Logging](logging.md) |
| Traces | `tracing` spans → OpenTelemetry | [Tracing](tracing.md) |
| Export | Prometheus scrape (Option A), OTLP (Option B), or both | [OTLP](otlp.md) |
| Dashboards | four Grafana dashboards, local stack | [Grafana](grafana.md) |
| Vendors | through the OpenTelemetry Collector | [Vendor integrations](vendor-integrations.md) |

## Two architectures

**Option A — Prometheus scrapes the application.** No extra infrastructure:
Prometheus pulls `/metrics` (ideally from a separate listener), Grafana
queries Prometheus.

**Option B — OTLP to an OpenTelemetry Collector.** The application pushes
metrics, logs and traces over OTLP; the Collector routes them to any number
of backends. This is the recommended production setup when you need traces
and logs centrally, or more than one backend.

Both can run at once. Both produce the same metric names, so the same
dashboards work for either. See [Architecture](architecture.md).

## Guarantees

* **No network I/O on the request path.** Recording a measurement is a
  local operation; export happens on SDK-managed background threads.
* **Bounded memory.** Every queue has a capacity; when full, telemetry is
  dropped and counted, never buffered without limit and never blocking a
  request.
* **Bounded shutdown.** `Observability::shutdown` returns within its
  timeout even if the Collector is down.
* **Bounded cardinality.** HTTP metrics use route templates (`/users/{id}`),
  never raw paths.
* **No global state unless asked.** Kerniq installs the `tracing` subscriber
  (and fails loudly if one exists); it never sets OpenTelemetry global
  providers unless you call `install_global_providers`.
* **Pay for what you enable.** Each signal and exporter is a Cargo feature; a
  default Kerniq build contains no OpenTelemetry code.

The cost of each signal is measured, not assumed: see
[Performance](performance.md).

## Examples

| Example | Shows |
|---|---|
| `examples/observability-minimal` | Prometheus `/metrics` + console logs in 20 lines |
| `examples/observability-demo` | Every feature: business metrics (banking, loans, database, authentication, outbound HTTP, queues), AI inference streaming, both architectures |
| `observability/docker-compose.yml` | The demo with Prometheus, Grafana, the Collector, Tempo and Loki |
