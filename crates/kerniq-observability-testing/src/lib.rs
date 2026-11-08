//! Deterministic telemetry assertions for Kerniq applications.
//!
//! [`TestTelemetry`] builds a real [`Observability`] whose exporters are the
//! OpenTelemetry SDK's in-memory exporters, collected synchronously on
//! demand: no timers, no network, no sleeps.
//!
//! ```
//! use kerniq_observability_testing::TestTelemetry;
//! use kerniq_observability::KeyValue;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! let telemetry = TestTelemetry::new().await;
//!
//! let counter = telemetry
//!     .observability()
//!     .meter("payments")
//!     .u64_counter("payments.processed")
//!     .build();
//! counter.add(2, &[KeyValue::new("payment.type", "transfer")]);
//! tracing::info_span!("settle").in_scope(|| tracing::info!("settled"));
//!
//! let metrics = telemetry.metrics();
//! assert_eq!(metrics.total("payments.processed", &[("payment.type", "transfer")]), 2.0);
//! telemetry.assert_bounded_metric_labels();
//!
//! let span = telemetry.span("settle");
//! let logs = telemetry.logs();
//! assert_eq!(logs[0].body, "settled");
//! assert_eq!(logs[0].trace_id.as_deref(), Some(span.span_context.trace_id().to_string().as_str()));
//! # }
//! ```
//!
//! The subscriber is installed as the **thread-local** default for the
//! lifetime of the `TestTelemetry`, so tests do not interfere with each
//! other. Use a current-thread runtime (the `#[tokio::test]` default) so
//! spawned tasks run on the same thread.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod snapshot;

use kerniq_observability::{
    LogsConfig, MetricsConfig, Observability, ObservabilityBuilder, Sampling, TracesConfig,
};
use opentelemetry_sdk::logs::{InMemoryLogExporter, SimpleLogProcessor};
use opentelemetry_sdk::metrics::InMemoryMetricExporter;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SimpleSpanProcessor, SpanData};

pub use snapshot::{MetricSnapshot, Point, PointValue};

/// A log record captured by the OpenTelemetry log bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedLog {
    /// Severity text (`INFO`, `WARN`, ...).
    pub severity: String,
    /// The message.
    pub body: String,
    /// The `tracing` target.
    pub target: String,
    /// Structured fields.
    pub attributes: Vec<(String, String)>,
    /// Trace id of the span the event was emitted in.
    pub trace_id: Option<String>,
    /// Span id of the span the event was emitted in.
    pub span_id: Option<String>,
}

impl CapturedLog {
    /// A structured field's value.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// An [`Observability`] wired to in-memory exporters.
pub struct TestTelemetry {
    observability: Observability,
    metrics: InMemoryMetricExporter,
    spans: InMemorySpanExporter,
    logs: InMemoryLogExporter,
    _guard: tracing::subscriber::DefaultGuard,
}

impl std::fmt::Debug for TestTelemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestTelemetry")
            .field("observability", &self.observability)
            .finish_non_exhaustive()
    }
}

impl TestTelemetry {
    /// Every signal enabled, every trace sampled, `trace`-level logs.
    pub async fn new() -> Self {
        Self::with(|builder| builder).await
    }

    /// Like [`new`](Self::new), with extra builder configuration (service
    /// name, HTTP exclusions, ...). Signal configuration set here is
    /// replaced by the in-memory pipelines.
    pub async fn with(
        configure: impl FnOnce(ObservabilityBuilder) -> ObservabilityBuilder,
    ) -> Self {
        let metrics = InMemoryMetricExporter::default();
        let spans = InMemorySpanExporter::default();
        let logs = InMemoryLogExporter::default();
        let builder = configure(
            Observability::builder()
                .service_name("test-service")
                .install_subscriber(false),
        )
        .install_subscriber(false)
        .metrics(MetricsConfig::new().exporter(metrics.clone()))
        .traces(
            TracesConfig::new()
                .sampling(Sampling::AlwaysOn)
                .filter("trace")
                .processor(SimpleSpanProcessor::new(spans.clone())),
        )
        .logs(
            LogsConfig::json()
                .without_console()
                .filter("trace")
                .processor(SimpleLogProcessor::new(logs.clone())),
        );
        let observability = builder
            .build()
            .await
            .expect("in-memory telemetry configuration is valid");

        use tracing_subscriber::layer::SubscriberExt as _;
        let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
        let guard = tracing::subscriber::set_default(subscriber);

        Self {
            observability,
            metrics,
            spans,
            logs,
            _guard: guard,
        }
    }

    /// The handle under test; pass it to `App::observability`.
    pub fn observability(&self) -> &Observability {
        &self.observability
    }

    /// Collects metrics now (cumulative totals since start).
    pub fn metrics(&self) -> MetricSnapshot {
        let provider = self
            .observability
            .meter_provider()
            .expect("metrics are enabled");
        provider
            .force_flush()
            .expect("in-memory export cannot fail");
        let collected = self.metrics.get_finished_metrics().unwrap_or_default();
        self.metrics.reset();
        MetricSnapshot::latest(&collected)
    }

    /// Finished spans so far.
    pub fn spans(&self) -> Vec<SpanData> {
        self.spans.get_finished_spans().unwrap_or_default()
    }

    /// The finished span named `name`.
    ///
    /// # Panics
    ///
    /// Panics (listing the spans that exist) if there is none.
    pub fn span(&self, name: &str) -> SpanData {
        let spans = self.spans();
        spans
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .unwrap_or_else(|| {
                let names: Vec<_> = spans.iter().map(|s| s.name.to_string()).collect();
                panic!("no finished span named `{name}`; finished spans: {names:?}")
            })
    }

    /// Log records bridged so far.
    pub fn logs(&self) -> Vec<CapturedLog> {
        self.logs
            .get_emitted_logs()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let r = &entry.record;
                let (trace_id, span_id) = r
                    .trace_context()
                    .map(|tc| (Some(tc.trace_id.to_string()), Some(tc.span_id.to_string())))
                    .unwrap_or((None, None));
                CapturedLog {
                    severity: r.severity_text().unwrap_or_default().to_owned(),
                    body: r.body().map(any_value_to_string).unwrap_or_default(),
                    target: r.target().map(|t| t.to_string()).unwrap_or_default(),
                    attributes: r
                        .attributes_iter()
                        .map(|(k, v)| (k.as_str().to_owned(), any_value_to_string(v)))
                        .collect(),
                    trace_id,
                    span_id,
                }
            })
            .collect()
    }

    /// Clears captured spans and logs (metrics are cumulative and cannot be
    /// reset).
    pub fn reset(&self) {
        self.spans.reset();
        self.logs.reset();
    }

    /// Asserts that no collected metric uses a forbidden attribute key
    /// (user ids, request ids, raw paths, ...; see
    /// `kerniq_telemetry::attributes::FORBIDDEN_METRIC_KEYS`).
    ///
    /// # Panics
    ///
    /// Panics naming the offending metric and attribute.
    pub fn assert_bounded_metric_labels(&self) {
        for point in &self.metrics().points {
            for key in point.attributes.keys() {
                assert!(
                    !kerniq_telemetry::attributes::is_forbidden_metric_key(key),
                    "metric `{}` uses the high-cardinality attribute `{key}`",
                    point.name
                );
            }
        }
    }
}

fn any_value_to_string(value: &opentelemetry::logs::AnyValue) -> String {
    use opentelemetry::logs::AnyValue;
    match value {
        AnyValue::String(s) => s.to_string(),
        AnyValue::Int(i) => i.to_string(),
        AnyValue::Double(d) => d.to_string(),
        AnyValue::Boolean(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}
