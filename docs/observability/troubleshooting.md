# Troubleshooting

**`build()` fails with `SubscriberAlreadySet`.** Something (another crate,
`tracing_subscriber::fmt::init()`, a test harness) installed a global
subscriber first. Use `.install_subscriber(false)` and add
`observability.tracing_layer()` to your subscriber — see
[Logging](logging.md#existing-subscribers).

**No spans in the backend.**
* Is `TracesConfig` configured with an exporter (`TracesConfig::otlp()`,
  not `TracesConfig::new()`)?
* `TracesConfig::filter` defaults to `info`; a `RUST_LOG=warn` only affects
  console logs, not span export.
* Sampling: `sampling_ratio(0.01)` exports 1 %. With `parent_based(true)`,
  callers that send unsampled `traceparent`s are not sampled.
* Check `observability.stats().traces`: `offered` grows when spans end;
  `export_failures` grows when the backend rejects them.

**`http_route` missing on some metrics.** Requests that match no route (the
404 fallback) have no route template by design. If *all* are missing, the
layer was applied outside the router (e.g. `ServiceBuilder` around the whole
app): use `App::observability` or Axum's `Router::layer`.

**Metric names differ between Prometheus and the vendor.** Prometheus names
include unit suffixes (`_seconds`, `_bytes`, `_total`); OTLP backends see
the OpenTelemetry names (`http.server.request.duration`). Both are correct.

**`unknown_service` in the backend.** Set `service_name(..)` or
`OTEL_SERVICE_NAME`.

**Logs exported but without `trace_id`.** The event was emitted outside any
span, or tracing is disabled. Within request handlers, spans exist when
tracing or console logs are enabled.

**Too many series / memory growth in the backend.** A metric attribute is
unbounded. Find it with
`topk(10, count by (__name__) ({__name__=~".+"}))` and the per-label counts;
fix it with `BoundedLabel` or move the value to spans. In tests,
`TestTelemetry::assert_bounded_metric_labels` catches common mistakes.

**Collector unavailable.** Expected behavior: requests are unaffected; the
SDK logs export errors (target `opentelemetry_otlp`, rate-limited); spans and
logs are dropped once queues fill (`stats().*.unexported` grows); metrics
continue aggregating and are sent once the Collector returns.

**Shutdown takes the full timeout.** The final flush could not reach the
backend. `ShutdownReport::errors` says which signal. Reduce
`shutdown_timeout` or exporter timeouts if the platform kills the process
first.

**`/metrics` returns 404.** With `prometheus_listener(..)` metrics are only
on that port. With `into_axum_router()`, the route is merged into the
router, but a separate listener is only started by `App::run`/`serve`.

**OpenTelemetry API code doesn't see the current span.** `Context::current()`
inside a `tracing` span is only set when
`TracesConfig::activate_otel_context(true)` (or log export) is enabled. Use
`tracing_opentelemetry::OpenTelemetrySpanExt::context()` or enable it.

**Docker stack shows no data.** Wait one scrape interval (15 s) plus the
dashboard rate window. Check http://localhost:9090/targets: in the
`prometheus` profile the `otel-collector` targets are expected to be down,
and vice versa.
