//! OTLP export against an in-process OTLP/HTTP receiver, and the failure
//! modes: unreachable Collector, hanging Collector (queue saturation),
//! failing Collector during shutdown.
//!
//! These tests prove the performance rules that matter most: recording
//! never waits for an exporter, queues stay bounded, and shutdown is
//! bounded even when the backend is down.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use kerniq_observability::prelude::*;
use kerniq_observability::{OtlpExporter, Retry};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Accept,
    Fail,
    Hang,
}

#[derive(Default)]
struct Received {
    spans: Vec<String>,
    logs: Vec<String>,
    metrics: Vec<String>,
    service_names: Vec<String>,
    requests: usize,
}

#[derive(Clone)]
struct Collector {
    mode: Mode,
    received: Arc<Mutex<Received>>,
}

fn service_name(attrs: &[opentelemetry_proto::tonic::common::v1::KeyValue]) -> Option<String> {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    attrs
        .iter()
        .find(|kv| kv.key == "service.name")
        .and_then(|kv| match kv.value.as_ref()?.value.as_ref()? {
            Value::StringValue(s) => Some(s.clone()),
            _ => None,
        })
}

async fn respond(c: &Collector) -> StatusCode {
    c.received.lock().unwrap().requests += 1;
    match c.mode {
        Mode::Accept => StatusCode::OK,
        Mode::Fail => StatusCode::INTERNAL_SERVER_ERROR,
        Mode::Hang => {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }
}

async fn traces(State(c): State<Collector>, body: Bytes) -> StatusCode {
    if let Ok(req) = ExportTraceServiceRequest::decode(body) {
        let mut r = c.received.lock().unwrap();
        for rs in req.resource_spans {
            if let Some(res) = &rs.resource {
                r.service_names.extend(service_name(&res.attributes));
            }
            for ss in rs.scope_spans {
                r.spans.extend(ss.spans.into_iter().map(|s| s.name));
            }
        }
    }
    respond(&c).await
}

async fn logs(State(c): State<Collector>, body: Bytes) -> StatusCode {
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    if let Ok(req) = ExportLogsServiceRequest::decode(body) {
        let mut r = c.received.lock().unwrap();
        for rl in req.resource_logs {
            for sl in rl.scope_logs {
                for log in sl.log_records {
                    if let Some(Value::StringValue(s)) = log.body.and_then(|b| b.value) {
                        r.logs.push(s);
                    }
                }
            }
        }
    }
    respond(&c).await
}

async fn metrics(State(c): State<Collector>, body: Bytes) -> StatusCode {
    if let Ok(req) = ExportMetricsServiceRequest::decode(body) {
        let mut r = c.received.lock().unwrap();
        for rm in req.resource_metrics {
            for sm in rm.scope_metrics {
                r.metrics.extend(sm.metrics.into_iter().map(|m| m.name));
            }
        }
    }
    respond(&c).await
}

async fn start_collector(mode: Mode) -> (SocketAddr, Arc<Mutex<Received>>) {
    let received = Arc::new(Mutex::new(Received::default()));
    let app = Router::new()
        .route("/v1/traces", post(traces))
        .route("/v1/logs", post(logs))
        .route("/v1/metrics", post(metrics))
        .with_state(Collector {
            mode,
            received: received.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, received)
}

fn http_exporter(addr: SocketAddr) -> OtlpExporter {
    OtlpExporter::http()
        .endpoint(format!("http://{addr}"))
        .timeout(Duration::from_secs(2))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exports_all_three_signals_over_otlp_http() {
    let (addr, received) = start_collector(Mode::Accept).await;
    let observability = Observability::builder()
        .service_name("otlp-test")
        .metrics(MetricsConfig::new().with_otlp(http_exporter(addr)))
        .traces(TracesConfig::new().with_otlp(http_exporter(addr)))
        .logs(
            LogsConfig::json()
                .without_console()
                .with_otlp(http_exporter(addr)),
        )
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());

    tracing::subscriber::with_default(subscriber, || {
        tracing::info_span!("checkout").in_scope(|| {
            tracing::info!("order placed");
        });
    });
    observability
        .meter("shop")
        .u64_counter("orders.placed")
        .build()
        .add(1, &[KeyValue::new("channel", "web")]);

    let report = observability.shutdown().await;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!report.timed_out);

    let r = received.lock().unwrap();
    assert!(
        r.spans.contains(&"checkout".to_owned()),
        "spans: {:?}",
        r.spans
    );
    assert!(
        r.logs.contains(&"order placed".to_owned()),
        "logs: {:?}",
        r.logs
    );
    assert!(
        r.metrics.contains(&"orders.placed".to_owned()),
        "metrics: {:?}",
        r.metrics
    );
    assert!(r.service_names.iter().all(|s| s == "otlp-test"));
    assert_eq!(report.stats.traces.exported, 1);
    assert_eq!(report.stats.traces.unexported, 0);
    assert_eq!(report.stats.logs.exported, 1);
    assert!(report.stats.metrics.exported > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_endpoints_per_signal() {
    let (traces_addr, traces_rx) = start_collector(Mode::Accept).await;
    let (logs_addr, logs_rx) = start_collector(Mode::Accept).await;
    let observability = Observability::builder()
        .traces(TracesConfig::new().with_otlp(http_exporter(traces_addr)))
        .logs(
            LogsConfig::json()
                .without_console()
                .with_otlp(http_exporter(logs_addr)),
        )
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info_span!("work").in_scope(|| tracing::warn!("careful"));
    });
    observability.shutdown().await;

    assert_eq!(traces_rx.lock().unwrap().spans, ["work"]);
    assert!(traces_rx.lock().unwrap().logs.is_empty());
    assert_eq!(logs_rx.lock().unwrap().logs, ["careful"]);
    assert!(logs_rx.lock().unwrap().spans.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_exporters_for_one_signal() {
    let (a, a_rx) = start_collector(Mode::Accept).await;
    let (b, b_rx) = start_collector(Mode::Accept).await;
    let observability = Observability::builder()
        .traces(
            TracesConfig::new()
                .with_otlp(http_exporter(a))
                .with_otlp(http_exporter(b)),
        )
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info_span!("fan-out").in_scope(|| {});
    });
    let report = observability.shutdown().await;
    assert_eq!(a_rx.lock().unwrap().spans, ["fan-out"]);
    assert_eq!(b_rx.lock().unwrap().spans, ["fan-out"]);
    assert_eq!(report.stats.traces.offered, 2, "one per pipeline");
    assert_eq!(report.stats.traces.exported, 2);
}

/// Nothing listens on the endpoint. Recording must stay fast, and shutdown
/// must return within its timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collector_unavailable() {
    // Bind and drop: the port is now (almost certainly) closed.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let observability = Observability::builder()
        .traces(
            TracesConfig::new().with_otlp(
                OtlpExporter::grpc()
                    .endpoint(format!("http://{closed}"))
                    .timeout(Duration::from_millis(200))
                    .retry(Retry::DISABLED),
            ),
        )
        .metrics(
            MetricsConfig::new().with_otlp(
                OtlpExporter::http()
                    .endpoint(format!("http://{closed}"))
                    .retry(Retry::DISABLED),
            ),
        )
        .shutdown_timeout(Duration::from_secs(2))
        .install_subscriber(false)
        .build()
        .await
        .expect("building succeeds even though the Collector is down");
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
    let counter = observability.meter("t").u64_counter("work").build();

    let start = Instant::now();
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..10_000 {
            tracing::info_span!("request").in_scope(|| counter.add(1, &[]));
        }
    });
    let recording = start.elapsed();
    // Generous bound for debug builds on shared CI: the point is that no
    // span waits for a network round trip (10k × timeout would be ~30 min).
    assert!(
        recording < Duration::from_secs(5),
        "recording took {recording:?}"
    );

    let report = observability.shutdown().await;
    assert!(
        report.duration < Duration::from_secs(3),
        "shutdown bounded by its timeout: {:?}",
        report.duration
    );
    let traces = report.stats.traces;
    assert_eq!(traces.offered, 10_000);
    assert_eq!(traces.exported, 0);
    assert_eq!(traces.failed + traces.unexported, 10_000, "{traces:?}");
}

/// The Collector accepts connections but never answers. The queue (16
/// spans) fills up; further spans are dropped, not buffered without bound,
/// and recording never blocks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_saturation_drops_instead_of_blocking() {
    let (addr, received) = start_collector(Mode::Hang).await;
    let observability = Observability::builder()
        .traces(
            TracesConfig::new()
                .batch(BatchConfig {
                    max_queue_size: 16,
                    max_export_batch_size: 16,
                    scheduled_delay: Duration::from_millis(10),
                    export_timeout: Duration::from_millis(300),
                })
                .with_otlp(
                    http_exporter(addr)
                        .timeout(Duration::from_millis(300))
                        .retry(Retry::DISABLED),
                ),
        )
        .shutdown_timeout(Duration::from_secs(2))
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());

    let start = Instant::now();
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..50_000 {
            tracing::info_span!("burst").in_scope(|| {});
        }
    });
    let recording = start.elapsed();
    assert!(
        recording < Duration::from_secs(10),
        "recording took {recording:?}"
    );

    let report = observability.shutdown().await;
    assert!(
        report.duration < Duration::from_secs(3),
        "{:?}",
        report.duration
    );
    let traces = report.stats.traces;
    assert_eq!(traces.offered, 50_000);
    assert_eq!(traces.exported, 0);
    // Only queue-sized batches ever reached the exporter; the rest were
    // dropped at enqueue time.
    assert!(traces.unexported > 40_000, "{traces:?}");
    assert!(received.lock().unwrap().requests < 1_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_failure_during_shutdown_is_reported_not_propagated() {
    let (addr, received) = start_collector(Mode::Fail).await;
    let observability = Observability::builder()
        .logs(
            LogsConfig::json()
                .without_console()
                .with_otlp(http_exporter(addr).retry(Retry::DISABLED)),
        )
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
    tracing::subscriber::with_default(subscriber, || {
        for i in 0..5 {
            tracing::error!(attempt = i, "backend exploded");
        }
    });
    let report = observability.shutdown().await;
    assert!(!report.timed_out);
    assert_eq!(report.stats.logs.offered, 5);
    assert_eq!(report.stats.logs.failed, 5);
    assert!(report.stats.logs.export_failures >= 1);
    assert!(received.lock().unwrap().requests >= 1);
}

#[tokio::test]
async fn builder_validation() {
    let bad_batch = Observability::builder()
        .traces(TracesConfig::new().batch(BatchConfig {
            max_queue_size: 8,
            max_export_batch_size: 64,
            ..BatchConfig::TRACES
        }))
        .install_subscriber(false)
        .build()
        .await;
    assert!(matches!(
        bad_batch,
        Err(kerniq_observability::Error::Config(_))
    ));

    let bad_endpoint = Observability::builder()
        .traces(TracesConfig::new().with_otlp(OtlpExporter::grpc().endpoint("collector:4317")))
        .install_subscriber(false)
        .build()
        .await;
    assert!(matches!(
        bad_endpoint,
        Err(kerniq_observability::Error::Config(_))
    ));

    let zero_limit = Observability::builder()
        .metrics(MetricsConfig::new().cardinality_limit(0))
        .install_subscriber(false)
        .build()
        .await;
    assert!(zero_limit.is_err());
}

#[test]
#[should_panic(expected = "sampling ratio")]
fn sampling_ratio_is_validated() {
    let _ = TracesConfig::new().sampling_ratio(1.5);
}
