# Prometheus (Option A)

Prometheus scrapes the application directly. No Collector is required.

```text
Kerniq app ──/metrics──▶ Prometheus ──PromQL──▶ Grafana
```

```rust,no_run
use kerniq_server::observability::prelude::*;
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let observability = Observability::builder()
        .service_name("payment-api")
        .metrics(MetricsConfig::prometheus().endpoint("/metrics"))
        .build()
        .await?;

    App::new()
        .observability(observability)
        .get("/users/{id}", |Path(id): Path<u64>| async move { format!("user {id}") })
        .run("0.0.0.0:3000")
        .await
}
```

The endpoint is backed by the OpenTelemetry Prometheus exporter
(`opentelemetry-prometheus`), which Kerniq does not reimplement. It is pull
based: there is no background task and no second copy of the metrics; each
scrape asks the SDK to aggregate its current state and encodes it on Tokio's
blocking pool, so scrapes never occupy a worker that serves requests.

## A separate listener (recommended)

```rust,no_run
# use kerniq_server::observability::prelude::*;
# async fn demo() -> Result<(), kerniq_server::observability::Error> {
let observability = Observability::builder()
    .metrics(MetricsConfig::prometheus().prometheus_listener(([0, 0, 0, 0], 9464).into()))
    .build()
    .await?;
# Ok(()) }
```

Under `App::run`, the listener is bound when the app starts (startup fails if
the port is taken) and stops with the app. Benefits:

* firewall or network-policy the port to the Prometheus servers only,
* scrapes never share connection limits, timeouts, rate limits or
  authentication middleware with user traffic,
* the application port does not expose internal metrics.

On the application listener, `/metrics` is mounted *outside* every
middleware: it is never rate limited, timed out, authenticated or recorded
in the HTTP metrics. If it must be protected there, use the separate
listener, or protect it at the ingress.

## Configuration

`PrometheusConfig` (via `MetricsConfig::with_prometheus`):

| Option | Default | |
|---|---|---|
| `path` | `/metrics` | scrape path |
| `listen` | — | separate listener address |
| `resource_labels` | `["service.name"]` | resource attributes copied onto every series |
| `target_info` | on | the `target_info` series with every resource attribute |
| `scope_labels` | off | `otel_scope_name` / `otel_scope_version` labels |
| `namespace` | — | prefix every metric name |

## Naming

OpenTelemetry names are translated by the exporter: dots become underscores,
the unit becomes a suffix, counters get `_total`.

| OpenTelemetry | Prometheus |
|---|---|
| `http.server.request.duration` (s) | `http_server_request_duration_seconds_{bucket,sum,count}` |
| `http.server.active_requests` | `http_server_active_requests` |
| `http.server.response.body.size` (By) | `http_server_response_body_size_bytes_*` |
| `kerniq.http.server.time_to_first_byte` (s) | `kerniq_http_server_time_to_first_byte_seconds_*` |
| `payments.processed` (counter) | `payments_processed_total` |

Labels: `http_request_method`, `http_route`, `http_response_status_code`,
`network_protocol_version`, `url_scheme`, `error_type`, `service_name`.

## Useful queries

```promql
# Request rate by route
sum by (http_route) (rate(http_server_request_duration_seconds_count[5m]))

# p95 latency by route
histogram_quantile(0.95, sum by (le, http_route) (rate(http_server_request_duration_seconds_bucket[5m])))

# Error ratio (5xx, aborted, body errors)
sum(rate(http_server_request_duration_seconds_count{error_type!=""}[5m]))
  / sum(rate(http_server_request_duration_seconds_count[5m]))

# Abandoned streams (client disconnected)
sum(rate(http_server_request_duration_seconds_count{error_type="kerniq.aborted"}[5m]))
```

## Scrape configuration

```yaml
scrape_configs:
  - job_name: payment-api
    scrape_interval: 15s
    static_configs:
      - targets: ["payment-api:9464"]
```

`observability/prometheus/prometheus.yml` has a complete example.

## Without Kerniq's `App`

```rust
use kerniq_server::observability::{PrometheusConfig, PrometheusExporter};
use kerniq_server::observability::opentelemetry_sdk::metrics::SdkMeterProvider;

let (reader, exporter) = PrometheusExporter::build(PrometheusConfig::default()).unwrap();
let provider = SdkMeterProvider::builder().with_reader(reader).build();
let router: axum::Router = exporter.router(); // merge into any Axum app
# let _ = (provider, router);
```
