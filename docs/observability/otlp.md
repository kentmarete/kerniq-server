# OTLP and the OpenTelemetry Collector (Option B)

The application pushes all three signals over OTLP; an OpenTelemetry
Collector routes them to backends.

```text
Kerniq app ──OTLP gRPC/HTTP──▶ Collector ──┬─▶ Prometheus / Mimir
  (batched, background threads)            ├─▶ Tempo / Jaeger
                                           ├─▶ Loki / Elastic / Splunk
                                           └─▶ Datadog / Azure Monitor / …
```

```rust,no_run
use kerniq_server::observability::prelude::*;

# async fn demo() -> Result<(), kerniq_server::observability::Error> {
let observability = Observability::builder()
    .service_name("payment-api")
    .metrics(MetricsConfig::otlp().endpoint("http://otel-collector:4317"))
    .logs(LogsConfig::otlp().endpoint("http://otel-collector:4317"))
    .traces(TracesConfig::otlp().endpoint("http://otel-collector:4317"))
    .build()
    .await?;
# Ok(()) }
```

Without `.endpoint(..)`, the standard `OTEL_EXPORTER_OTLP_ENDPOINT`,
`OTEL_EXPORTER_OTLP_<SIGNAL>_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS` and
`OTEL_EXPORTER_OTLP_TIMEOUT` variables apply, then `http://localhost:4317`.

## Exporter options

```rust
use std::time::Duration;
use kerniq_server::observability::{OtlpExporter, Retry};

let exporter = OtlpExporter::http()                 // or ::grpc() (default)
    .endpoint("https://otlp.example.com")           // /v1/<signal> appended for HTTP
    .header("x-api-key", "…")                      // never logged
    .timeout(Duration::from_secs(5))
    .gzip()
    .retry(Retry { max_retries: 5, initial_delay: Duration::from_millis(200), max_delay: Duration::from_secs(5) });
# let _ = exporter;
```

| Protocol | Feature | Runs on |
|---|---|---|
| gRPC (`OtlpExporter::grpc`) | `otlp` | the application's Tokio runtime (tonic) |
| HTTP/protobuf (`OtlpExporter::http`) | `otlp` | a blocking client on the export thread |
| TLS (`OtlpExporter::tls`, `https://`) | `otlp-tls` | rustls with platform roots; custom CA and mTLS for gRPC |

## Separate endpoints and multiple exporters

Each signal is configured independently and can have several exporters:

```rust
use kerniq_server::observability::prelude::*;

let traces = TracesConfig::new()
    .with_otlp(OtlpExporter::grpc().endpoint("http://tempo:4317"))
    .with_otlp(OtlpExporter::http().endpoint("https://vendor.example.com").header("api-key", "…"));
let logs = LogsConfig::json()                         // keep stdout
    .with_otlp(OtlpExporter::http().endpoint("http://loki:3100/otlp"));
let metrics = MetricsConfig::prometheus()             // scrape *and* push
    .with_otlp(OtlpExporter::grpc().endpoint("http://otel-collector:4317"));
# let _ = (traces, logs, metrics);
```

Each exporter gets its own bounded pipeline, so a slow backend cannot hold
up the others. Prefer **one** Collector with several exporters (see
`observability/otel-collector/config-multi-exporter.yaml`) to many exporters
in the application: fan-out, credentials and retries then live in the
Collector.

## Batching, limits and failure policy

| Signal | Mechanism | Defaults | When the backend is down |
|---|---|---|---|
| Traces | `BatchSpanProcessor` | queue 2048, batch 512, every 5 s | queue fills; new spans **dropped**, counted |
| Logs | `BatchLogProcessor` | queue 2048, batch 512, every 1 s | queue fills; new records **dropped**, counted |
| Metrics | `PeriodicReader` | every 60 s | the export fails; aggregation continues in memory (bounded by cardinality limits) and the next export carries cumulative totals |

```rust
use std::time::Duration;
use kerniq_server::observability::prelude::*;

let traces = TracesConfig::otlp().batch(BatchConfig {
    max_queue_size: 8192,
    max_export_batch_size: 1024,
    scheduled_delay: Duration::from_secs(2),
    export_timeout: Duration::from_secs(5),
});
# let _ = traces;
```

Every export call is bounded by its timeout, retried with exponential
backoff up to the configured limit, then given up. Failures are logged by
the SDK (target `opentelemetry_sdk` / `opentelemetry_otlp`, never
re-exported over OTLP) and counted in
[`Observability::stats`](configuration.md#export-statistics).

Requests never wait for any of this. In the test suite
(`kerniq-observability-testing/tests/otlp.rs`), 10 000 spans recorded
against an unreachable Collector, and 50 000 against one that accepts
connections but never answers, complete in milliseconds, and shutdown
returns within its timeout.

## Temporality

OTLP metrics are cumulative by default. Backends that prefer deltas
(Datadog, some SaaS) can use `MetricsConfig::otlp().delta_temporality(true)`.

## Running a Collector

`observability/otel-collector/config.yaml` is a working gateway
configuration (memory limiter, batching, Prometheus/Tempo/Loki exporters).
Start the whole stack with:

```text
docker compose -f observability/docker-compose.yml --profile otlp up --build
```
