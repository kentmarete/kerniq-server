# Distributed tracing

Spans are ordinary `tracing` spans. `tracing-opentelemetry` turns them into
OpenTelemetry spans, which are sampled, batched and exported by the SDK.

```rust,no_run
use kerniq_server::observability::prelude::*;

# async fn demo() -> Result<(), kerniq_server::observability::Error> {
let observability = Observability::builder()
    .service_name("payment-api")
    .traces(TracesConfig::otlp().sampling_ratio(0.10))
    .build()
    .await?;
# Ok(()) }
```

## The request span

For every request, the HTTP layer opens an `INFO` span (target
`kerniq::http`) named `{method} {route}`, e.g. `GET /users/{id}`, with kind
`server` and these attributes:

| Attribute | |
|---|---|
| `http.request.method`, `http.route`, `url.path`, `url.scheme` | request |
| `network.protocol.version`, `user_agent.original` | request |
| `http.response.status_code`, `http.response.body.size` | response |
| `error.type`, status `Error` | 5xx, aborted, body errors (4xx are not errors) |
| `kerniq.request_id` | from `x-request-id` (request, or response when an inner `RequestId` generates it) |

The query string, headers and bodies are never captured. The span stays open
until the response body ends, so a streaming response's span covers the
whole stream, and code that produces the stream runs inside it.

## Context propagation

Incoming W3C `traceparent` / `tracestate` and `baggage` headers are
extracted, and the request span continues the caller's trace. For outgoing
calls, inject the current context:

```rust
use kerniq_server::observability::propagation::inject_current_context;
# fn demo(observability: &kerniq_server::observability::Observability) {
let mut request = http::Request::get("http://fx-service/rates").body(()).unwrap();
inject_current_context(observability.propagator().as_ref(), request.headers_mut());
// send `request` with your HTTP client
# }
```

`examples/observability-demo` does this for an outbound HTTP call; in Tempo
the trace shows `POST /payments` (server) → `GET /internal/fx-rate`
(client) → `GET /internal/fx-rate` (server).

Async code keeps its context through `tracing::Instrument` /
`#[instrument]`: spans follow futures across `.await` and threads.

## Sampling

| Strategy | |
|---|---|
| `Sampling::AlwaysOn` | every trace (default) |
| `Sampling::AlwaysOff` | none (context still propagates) |
| `Sampling::Ratio(r)` / `.sampling_ratio(r)` | a fraction, decided from the trace id |
| `.parent_based(true)` (default) | follow the caller's decision when a `traceparent` arrives |

Sampling reduces export volume and backend cost. It does **not** make
unsampled requests free in-process: `tracing-opentelemetry` still builds the
span's attributes before the sampling decision (about 30 extra allocations
per request in `benches/results/allocations.md`). Measure with your
workload; see [Performance](performance.md).

Custom or advanced samplers (tail sampling, rate limiting) belong in the
Collector (`tail_sampling` processor) or can be added as an SDK
`SpanProcessor` with `TracesConfig::processor`.

## Errors and exceptions

`tracing` events recorded with an `error` field inside a span become
exception events; the HTTP span's status is `Error` for 5xx and failed or
abandoned responses, with `error.type` set.

```rust
# fn handle(result: Result<(), std::io::Error>) {
if let Err(err) = &result {
    tracing::error!(error = %err, "settlement failed");
}
# }
```

Avoid spans for trivial operations: a span per function call costs more than
the work it measures. Instrument I/O boundaries (database, HTTP, queues,
model calls).

## Filtering which spans are exported

`TracesConfig::filter("info")` (the default) decides which `tracing` spans
become OpenTelemetry spans. `debug`/`trace` spans in hot loops are not
exported unless you lower it.
