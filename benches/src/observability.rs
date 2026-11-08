//! Observability overhead scenarios.
//!
//! Every scenario serves the same two routes with the same handlers; only
//! the observability configuration differs:
//!
//! * `GET /users/{id}` — a small text response; the handler emits one
//!   `INFO` event (a no-op when no subscriber is installed),
//! * `GET /stream` — a streaming response of 64 frames × 64 bytes, to
//!   measure per-frame instrumentation cost.
//!
//! OTLP scenarios export over OTLP/HTTP to `KERNIQ_BENCH_SINK`
//! (`bench-server sink <addr>` in another process), so export work is real
//! but the receiving side is not charged to the measured server.

use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use kerniq_server::observability::prelude::*;
use kerniq_server::observability::{OtlpExporter, Retry};
use kerniq_server::prelude::*;

/// Observability configurations under test.
pub const SCENARIOS: &[&str] = &[
    "baseline",
    "metrics",
    "prometheus",
    "otlp-metrics",
    "json-logs",
    "otlp-logs",
    "traces-sampled",
    "all",
    "collector-down",
    "logs-noop-export",
    "traces-noop-export",
];

async fn user(Path(id): Path<u64>) -> &'static str {
    tracing::info!(user_id = id, "user fetched");
    "ok"
}

async fn stream() -> Body {
    static CHUNK: [u8; 64] = [b'x'; 64];
    let frames = futures_util::stream::iter(
        (0..64).map(|_| Ok::<_, Infallible>(Bytes::from_static(&CHUNK))),
    );
    Body::from_stream(frames)
}

fn sink() -> String {
    std::env::var("KERNIQ_BENCH_SINK").unwrap_or_else(|_| "http://127.0.0.1:4399".into())
}

fn exporter(endpoint: &str) -> OtlpExporter {
    OtlpExporter::http()
        .endpoint(endpoint)
        .timeout(Duration::from_secs(2))
}

/// Builds the observability handle for `scenario` (`None` for the
/// baseline). Console logs go to stdout; redirect it.
pub async fn build(scenario: &str, install_subscriber: bool) -> Option<Observability> {
    let sink = sink();
    let down = "http://127.0.0.1:9";
    let base = Observability::builder()
        .service_name("kerniq-bench")
        .install_subscriber(install_subscriber)
        .shutdown_timeout(Duration::from_secs(5));
    let builder = match scenario {
        "baseline" => return None,
        "metrics" => base.metrics(MetricsConfig::new()),
        "prometheus" => base.metrics(MetricsConfig::prometheus()),
        "otlp-metrics" => base.metrics(
            MetricsConfig::new()
                .with_otlp(exporter(&sink))
                .export_interval(Duration::from_secs(1)),
        ),
        "json-logs" => base.logs(LogsConfig::json().filter("info")),
        "otlp-logs" => base.logs(
            LogsConfig::json()
                .without_console()
                .filter("info")
                .with_otlp(exporter(&sink)),
        ),
        "traces-sampled" => base.traces(
            TracesConfig::new()
                .with_otlp(exporter(&sink))
                .sampling_ratio(0.1),
        ),
        "all" => base
            .metrics(
                MetricsConfig::prometheus()
                    .with_otlp(exporter(&sink))
                    .export_interval(Duration::from_secs(1))
                    .runtime(true),
            )
            .logs(LogsConfig::json().filter("info").with_otlp(exporter(&sink)))
            .traces(
                TracesConfig::new()
                    .with_otlp(exporter(&sink))
                    .sampling_ratio(0.1),
            ),
        "collector-down" => base
            .metrics(
                MetricsConfig::new()
                    .with_otlp(exporter(down).retry(Retry::DISABLED))
                    .export_interval(Duration::from_secs(1)),
            )
            .logs(
                LogsConfig::json()
                    .without_console()
                    .filter("info")
                    .with_otlp(exporter(down).retry(Retry::DISABLED)),
            )
            .traces(
                TracesConfig::new()
                    .with_otlp(exporter(down).retry(Retry::DISABLED))
                    .sampling_ratio(0.1),
            ),
        // The SDK batch pipelines with exporters that do nothing: isolates
        // in-process pipeline cost from export (serialization, network).
        "logs-noop-export" => base.logs(
            LogsConfig::json()
                .without_console()
                .filter("info")
                .processor(
                    opentelemetry_sdk::logs::BatchLogProcessor::builder(in_process::NoopLogs)
                        .build(),
                ),
        ),
        "traces-noop-export" => base.traces(TracesConfig::new().sampling_ratio(0.1).processor(
            opentelemetry_sdk::trace::BatchSpanProcessor::builder(in_process::NoopSpans).build(),
        )),
        other => panic!("unknown observability scenario `{other}`"),
    };
    Some(builder.build().await.expect("valid scenario configuration"))
}

/// The application, optionally observed.
pub fn app(observability: Option<Observability>) -> App {
    let app = App::new().get("/users/{id}", user).get("/stream", stream);
    match observability {
        Some(o) => app.observability(o),
        None => app,
    }
}

/// An OTLP/HTTP sink that accepts and discards everything (`200 OK`).
pub fn sink_router() -> axum::Router {
    use axum::routing::post;
    let ok = || async { StatusCode::OK };
    axum::Router::new()
        .route("/v1/traces", post(ok))
        .route("/v1/logs", post(ok))
        .route("/v1/metrics", post(ok))
}

/// In-process configurations for allocation counting: real SDK pipelines
/// with no-op exporters, so no sockets are involved.
pub mod in_process {
    use super::*;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::logs::{BatchLogProcessor, LogBatch, LogExporter};
    use opentelemetry_sdk::trace::{BatchSpanProcessor, SpanData, SpanExporter};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Debug)]
    pub(crate) struct NoopSpans;
    impl SpanExporter for NoopSpans {
        async fn export(&self, _: Vec<SpanData>) -> OTelSdkResult {
            Ok(())
        }
    }

    #[derive(Debug)]
    pub(crate) struct NoopLogs;
    impl LogExporter for NoopLogs {
        async fn export(&self, _: LogBatch<'_>) -> OTelSdkResult {
            Ok(())
        }
    }

    /// Scenario names.
    pub const SCENARIOS: &[&str] = &[
        "baseline",
        "metrics",
        "traces-100%",
        "traces-10%",
        "json-logs",
        "log-export",
        "all",
    ];

    /// Builds the router and the thread-local subscriber guard for one
    /// scenario (the guard must outlive the measurement).
    pub async fn build(
        scenario: &str,
    ) -> (
        axum::Router,
        Option<tracing::subscriber::DefaultGuard>,
        Option<Observability>,
    ) {
        let base = Observability::builder().install_subscriber(false);
        let traces = |ratio: f64| {
            TracesConfig::new()
                .sampling_ratio(ratio)
                .processor(BatchSpanProcessor::builder(NoopSpans).build())
        };
        let log_export = || {
            LogsConfig::json()
                .without_console()
                .filter("info")
                .processor(BatchLogProcessor::builder(NoopLogs).build())
        };
        let (builder, json) = match scenario {
            "baseline" => return (app(None).into_axum_router(), None, None),
            "metrics" => (base.metrics(MetricsConfig::new()), false),
            "traces-100%" => (base.traces(traces(1.0)), false),
            "traces-10%" => (base.traces(traces(0.1)), false),
            // Console JSON to a sink writer (see `json_layer` below).
            "json-logs" => (
                base.logs(LogsConfig::json().without_console().filter("info")),
                true,
            ),
            "log-export" => (base.logs(log_export()), false),
            "all" => (
                base.metrics(MetricsConfig::new())
                    .traces(traces(0.1))
                    .logs(log_export()),
                true,
            ),
            other => panic!("unknown scenario {other}"),
        };
        let observability = builder.build().await.expect("valid");
        let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
        let guard = if json {
            let console = kerniq_server::observability::logging::json_layer(std::io::sink);
            tracing::subscriber::set_default(subscriber.with(console))
        } else {
            tracing::subscriber::set_default(subscriber)
        };
        // Console-only JSON still creates the request span, as in production.
        let router = if json && scenario == "json-logs" {
            app(None)
                .layer(kerniq_server::observability::HttpInstrumentation::new().into_layer())
                .into_axum_router()
        } else {
            app(Some(observability.clone())).into_axum_router()
        };
        (router, Some(guard), Some(observability))
    }
}
