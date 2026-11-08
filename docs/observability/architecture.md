# Observability architecture

Kerniq Observe is configuration and lifecycle over existing libraries. It
contains no telemetry engine, no metrics aggregation, no exporter protocol
and no log formatter framework of its own.

| Concern | Provided by |
|---|---|
| Telemetry API and SDK (aggregation, sampling, batching, export) | `opentelemetry`, `opentelemetry_sdk` 0.33 |
| OTLP gRPC / HTTP exporters, retries | `opentelemetry-otlp` |
| Prometheus exposition | `opentelemetry-prometheus` + `prometheus` |
| Instrumentation and logging API | `tracing` |
| Span → OpenTelemetry bridge | `tracing-opentelemetry` 0.34 |
| Log event → OpenTelemetry bridge | `opentelemetry-appender-tracing` |
| Filtering, console formatting | `tracing-subscriber` |
| Non-blocking console writer | `tracing-appender` |
| HTTP middleware | Tower / Axum |

## Crates

```text
 kerniq-routing        kerniq-telemetry            (OpenTelemetry API only)
 (Middleware trait)    semantic-convention names, bounded labels,
        ▲              resource metadata, inference instruments
        │                 ▲         ▲
        │                 │         │
 kerniq-observability-axum│         │   kerniq-observability-prometheus
 (Tower layer: HTTP metrics,        │   (scrape endpoint over
  request span, W3C extraction;     │    opentelemetry-prometheus)
  OpenTelemetry API only, no SDK)   │         ▲
        ▲                           │         │
        └───────────┬───────────────┴─────────┘
                    │
           kerniq-observability     ── facade + SDK wiring: builder, providers,
                    ▲                  OTLP, logging pipeline, lifecycle, stats
                    │ optional (feature `observability`)
              kerniq-server         ── App::observability
                    ▲
       kerniq-observability-testing ── in-memory exporters + assertions (dev)
```

The graph is acyclic and `kerniq-server` depends on observability only
through an optional feature: a default build compiles none of it (71 crates
vs. 77 with `observability-metrics` and 171 with `full-observability`).

### Why five crates, not six

The proposal had a separate `kerniq-otel` (SDK wiring) under the facade.
The facade would always depend on it and nothing else would, so it was merged
into `kerniq-observability` as modules (`otlp`, `stats`, `logging`,
`runtime`). The remaining crates each exist because of a distinct dependency
profile:

* `kerniq-telemetry` — OpenTelemetry API only; usable by libraries.
* `kerniq-observability-axum` — Tower + OpenTelemetry API; usable with any
  SDK or with plain Axum, without pulling the SDK.
* `kerniq-observability-prometheus` — the `prometheus` crate, only when
  scraping is wanted.
* `kerniq-observability-testing` — the SDK's `testing` feature, dev only.

## Request path

```text
request ─▶ HttpService::call
             ├─ route template from MatchedPath (cached per route: no allocation)
             ├─ active_requests += 1
             ├─ tracing span "HTTP request" (+ W3C parent from traceparent/baggage)
             └─ inner service ─▶ … handler …
response ◀─ ResponseFuture: status recorded, body wrapped in ObservedBody
body     ◀─ ObservedBody::poll_frame: frame forwarded as-is, length counted,
             first frame → time_to_first_byte
end      ◀─ end of stream / body error / drop (client gone)
             └─ duration, body size, active -= 1, span closed  (once)
```

Everything on this path is in-process: atomic increments and hash lookups in
the SDK's aggregation. Spans and log records are handed to bounded queues
with a non-blocking `try_send`.

## Export path

```text
                    ┌─ PeriodicReader thread ──▶ OTLP metrics (every 60 s)
SdkMeterProvider ───┤
                    └─ Prometheus reader ◀── GET /metrics (collects on scrape)
SdkTracerProvider ── BatchSpanProcessor thread ──▶ OTLP traces
SdkLoggerProvider ── BatchLogProcessor thread  ──▶ OTLP logs
console fmt layer ── tracing-appender thread   ──▶ stdout / stderr
```

Each pipeline is one SDK-managed OS thread with a bounded queue, created once
at startup. Nothing is spawned per measurement and no Tokio runtime is
created; the gRPC exporter uses the application's runtime handle. The HTTP
exporter uses a blocking client on its export thread.

## Lifecycle

1. `Observability::builder()…build().await` creates the resource and the
   providers, and installs the `tracing` subscriber (or fails with
   `SubscriberAlreadySet` instead of replacing one). Exporters connect
   lazily, so a missing Collector does not fail startup.
2. `App::observability(o)` adds the HTTP layer as the **outermost**
   middleware at finalization, and mounts `/metrics` *outside* every
   middleware (or starts the separate metrics listener).
3. On shutdown, Kerniq stops accepting, drains connections and background
   tasks, then calls `Observability::shutdown`, which flushes traces, logs
   and metrics in that order on the blocking pool, bounded by
   `shutdown_timeout` (default 5 s), and finally flushes console logs.

## Decisions and deviations from the specification

* **Feature names.** `kerniq-server` already has a default feature called
  `tracing` (the `Logger` middleware), so observability features are
  `observability`, `observability-metrics`, `observability-logs`,
  `observability-tracing`, `otlp`, `otlp-tls`, `prometheus`,
  `runtime-metrics` and `full-observability`. `full` does not include them,
  so existing `full` users do not suddenly compile gRPC and HTTP clients.
* **`MetricsConfig::endpoint`** means the scrape path for
  `MetricsConfig::prometheus()` and the Collector URL for
  `MetricsConfig::otlp()`, matching the specification's examples.
* **Duration semantics.** `http.server.request.duration` ends when the
  response *body* ends, so streaming responses report their full lifetime;
  `kerniq.http.server.time_to_first_byte` reports when the first chunk was
  produced. Active requests likewise include the streaming phase. Active
  *connections* are not measured by the request layer.
* **The existing `Metrics` middleware** (hand-written Prometheus text) is
  unchanged for compatibility. New applications should prefer Kerniq
  Observe, which follows the OpenTelemetry conventions and supports OTLP.
* **Request ids.** The HTTP layer is outermost, so an inner `RequestId`
  middleware generates the id after the span starts. The id is read from the
  response and recorded on the span; log lines emitted *during* the request
  correlate through `trace_id` instead (or through an incoming
  `x-request-id`).
* **OpenTelemetry context activation** (making a span's context
  `Context::current()` on every enter) is off unless logs are exported or
  `TracesConfig::activate_otel_context(true)` is set; it costs about 10 % of
  streaming throughput and Kerniq's own propagation does not need it.
* **Dropped-telemetry counts.** The SDK drops on full queues but does not
  expose counts, so Kerniq counts items offered to and exported by each
  pipeline (see [Configuration](configuration.md#export-statistics)).
