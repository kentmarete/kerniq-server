//! A payments-and-inference service that exercises every Kerniq Observe
//! feature. It is the sample application of `observability/docker-compose.yml`.
//!
//! Configuration (environment):
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `PORT` | `3000` | application port |
//! | `KERNIQ_METRICS` | `prometheus` | `prometheus` (Option A), `otlp` (Option B) or `both` |
//! | `KERNIQ_METRICS_PORT` | — | serve `/metrics` on its own port instead of `PORT` |
//! | `KERNIQ_OTLP` | `false` | export traces and logs over OTLP |
//! | `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | Collector (gRPC) |
//! | `KERNIQ_TRACE_RATIO` | `1.0` | trace sampling ratio |
//! | `KERNIQ_LOG_FORMAT` | `json` | `json`, `pretty` or `compact` |
//! | `KERNIQ_ENVIRONMENT` | `development` | `deployment.environment.name` |
//!
//! ```text
//! cargo run -p example-observability-demo
//! curl -X POST 'localhost:3000/payments?type=transfer&amount_minor=1250&currency=USD'
//! curl -N 'localhost:3000/v1/generate?model=chat-small&tokens=40'
//! curl localhost:3000/metrics
//! ```

mod business;
mod inference;

use std::time::Duration;

use kerniq_server::observability::prelude::*;
use kerniq_server::prelude::*;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn env_flag(name: &str) -> bool {
    env(name).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

fn metrics_config() -> MetricsConfig {
    let mode = env("KERNIQ_METRICS").unwrap_or_else(|| "prometheus".into());
    let mut config = match mode.as_str() {
        "otlp" => MetricsConfig::otlp(),
        "both" => MetricsConfig::prometheus().with_otlp(OtlpExporter::grpc()),
        _ => MetricsConfig::prometheus(),
    };
    if mode != "otlp"
        && let Some(port) = env("KERNIQ_METRICS_PORT").and_then(|p| p.parse::<u16>().ok())
    {
        config = config.prometheus_listener(([0, 0, 0, 0], port).into());
    }
    config
        .export_interval(Duration::from_secs(10))
        .runtime(true)
}

fn logs_config(otlp: bool) -> LogsConfig {
    let config = match env("KERNIQ_LOG_FORMAT").as_deref() {
        Some("pretty") => LogsConfig::pretty(),
        Some("compact") => LogsConfig::compact(),
        _ => LogsConfig::json(),
    };
    if otlp {
        config.with_otlp(OtlpExporter::grpc())
    } else {
        config
    }
}

fn traces_config(otlp: bool) -> TracesConfig {
    let ratio = env("KERNIQ_TRACE_RATIO")
        .and_then(|r| r.parse::<f64>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let config = if otlp {
        TracesConfig::otlp()
    } else {
        TracesConfig::new()
    };
    config.sampling_ratio(ratio)
}

#[tokio::main]
async fn main() -> Result<()> {
    let otlp = env_flag("KERNIQ_OTLP");
    let port: u16 = env("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000);

    let observability = Observability::builder()
        .service_name("kerniq-demo")
        .service_version(env!("CARGO_PKG_VERSION"))
        .service_namespace("kerniq")
        .environment(env("KERNIQ_ENVIRONMENT").unwrap_or_else(|| "development".into()))
        .metrics(metrics_config())
        .logs(logs_config(otlp))
        .traces(traces_config(otlp))
        .exclude_http_route("/health")
        .build()
        .await?;

    let state = business::AppState::new(&observability, port);
    let worker_state = state.clone();

    tracing::info!(port, otlp, "starting kerniq-demo");
    App::new()
        .observability(observability)
        .middleware(RequestId::new())
        // Long enough for generation streams; the timeout covers the
        // handler, not the streamed body.
        .config(ServerConfig::new().request_timeout(Duration::from_secs(30)))
        .get("/health", || async { "OK" })
        .state(state)
        .merge(business::routes())
        .merge(inference::routes())
        .background_task("payment-queue-worker", move |stop| {
            business::queue_worker(worker_state, stop)
        })
        .run(format!("0.0.0.0:{port}"))
        .await
}
