//! # Kerniq Observe
//!
//! Vendor-neutral metrics, structured logs and distributed tracing for
//! Kerniq Server, built on OpenTelemetry, `tracing` and Prometheus.
//!
//!
//! ## What Kerniq adds, and what it reuses
//!
//! Aggregation, sampling, batching, export and retries are the OpenTelemetry
//! SDK's. Log formatting and filtering are `tracing-subscriber`'s. Prometheus
//! encoding is `opentelemetry-prometheus`'s. Kerniq contributes:
//!
//! * one builder that configures every signal independently,
//! * explicit provider ownership: no global OpenTelemetry provider is set
//!   unless you call [`Observability::install_global_providers`],
//! * bounded pipelines with drop accounting ([`Observability::stats`]),
//! * a bounded [`shutdown`](Observability::shutdown),
//! * HTTP instrumentation with streaming-aware measurements
//!   (`kerniq-observability-axum`),
//! * a JSON log format that carries `trace_id`/`span_id`,
//! * safeguards against telemetry feedback loops (exporter and HTTP-client
//!   crates are excluded from the OpenTelemetry log and trace bridges).
//!
//! ## Crate features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `metrics` | ✓ | metrics SDK, HTTP metrics |
//! | `logs` | ✓ | console logs (JSON / pretty / compact), log bridge |
//! | `tracing` | ✓ | spans → OpenTelemetry, W3C propagation |
//! | `otlp` | | OTLP gRPC + HTTP exporters for the enabled signals |
//! | `otlp-tls` | | TLS for OTLP |
//! | `prometheus` | | Prometheus scrape endpoint |
//! | `runtime` | | Tokio and process metrics |
//! | `full` | | all of the above except `otlp-tls` |

#![cfg_attr(
    all(feature = "metrics", feature = "logs", feature = "tracing"),
    doc = r####"
## Example

```no_run
use kerniq_observability::prelude::*;

# async fn demo() -> Result<(), kerniq_observability::Error> {
let observability = Observability::builder()
    .service_name("payment-api")
    .service_version("1.0.0")
    .environment("production")
    // With the `prometheus` feature: MetricsConfig::prometheus().endpoint("/metrics")
    .metrics(MetricsConfig::new())
    .logs(LogsConfig::json())
    // With the `otlp` feature: TracesConfig::otlp().sampling_ratio(0.10)
    .traces(TracesConfig::new().sampling_ratio(0.10))
    .build()
    .await?;

// With kerniq-server (feature `observability`):
//     App::new().observability(observability.clone()).get(...).run(..).await

// Application metrics use the OpenTelemetry API directly:
let payments = observability
    .meter("payment-service")
    .u64_counter("payments.processed")
    .build();
payments.add(1, &[KeyValue::new("payment.type", "transfer")]);

// Flush and stop exporters (Kerniq's `App` does this after draining).
let report = observability.shutdown().await;
# Ok(()) }
```
"####
)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod config;
#[cfg(feature = "logs")]
pub mod logging;
#[cfg(feature = "otlp")]
mod otlp;
#[cfg(feature = "runtime")]
mod runtime;
mod stats;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(feature = "logs")]
pub use config::LogsConfig;
#[cfg(feature = "metrics")]
pub use config::MetricsConfig;
#[cfg(feature = "tracing")]
pub use config::TracesConfig;
pub use config::{BatchConfig, ConsoleFormat, ConsoleTarget, Sampling};
#[cfg(feature = "otlp-tls")]
pub use otlp::TlsConfig;
#[cfg(feature = "otlp")]
pub use otlp::{OtlpExporter, Protocol, Retry};
pub use stats::{SignalStats, TelemetryStats};

pub use kerniq_observability_axum::HttpInstrumentation;
/// W3C trace-context helpers for outbound calls.
#[cfg(feature = "tracing")]
pub use kerniq_observability_axum::propagation;
#[cfg(feature = "prometheus")]
pub use kerniq_observability_prometheus::{PrometheusConfig, PrometheusExporter};
pub use kerniq_telemetry::{self as telemetry, BoundedLabel, ServiceInfo};

/// Re-export of the OpenTelemetry API.
pub use opentelemetry;
pub use opentelemetry::KeyValue;
/// Re-export of the OpenTelemetry SDK.
#[cfg(any(feature = "metrics", feature = "tracing", feature = "logs"))]
pub use opentelemetry_sdk;

use stats::Counters;

/// Everything most applications need.
pub mod prelude {
    #[cfg(feature = "logs")]
    pub use crate::LogsConfig;
    #[cfg(feature = "metrics")]
    pub use crate::MetricsConfig;
    #[cfg(feature = "prometheus")]
    pub use crate::PrometheusConfig;
    #[cfg(feature = "tracing")]
    pub use crate::TracesConfig;
    pub use crate::{
        BatchConfig, BoundedLabel, ConsoleFormat, Observability, ObservabilityBuilder, Sampling,
    };
    #[cfg(feature = "otlp")]
    pub use crate::{OtlpExporter, Protocol};
    pub use opentelemetry::KeyValue;
    #[cfg(feature = "metrics")]
    pub use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, UpDownCounter};
}

/// Observability setup errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The configuration is invalid.
    #[error("invalid observability configuration: {0}")]
    Config(String),
    /// An exporter could not be built.
    #[error("failed to build the {signal} exporter: {source}")]
    Exporter {
        /// `traces`, `logs` or `metrics`.
        signal: &'static str,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A global `tracing` subscriber was already installed.
    #[error(
        "a global tracing subscriber is already installed; call \
         `.install_subscriber(false)` and add `observability.tracing_layer()` to your own subscriber"
    )]
    SubscriberAlreadySet,
    /// The Prometheus exporter could not be built.
    #[cfg(feature = "prometheus")]
    #[error(transparent)]
    Prometheus(#[from] kerniq_observability_prometheus::Error),
}

impl From<Error> for kerniq_core::Error {
    fn from(error: Error) -> Self {
        kerniq_core::Error::internal(error)
    }
}

/// Targets whose events and spans are never bridged to OpenTelemetry: the
/// exporters' own HTTP/gRPC stacks and the SDK's diagnostics. Bridging them
/// would make every export produce more telemetry to export.
#[cfg(any(feature = "tracing", feature = "logs"))]
const BRIDGE_EXCLUSIONS: &str = "opentelemetry=off,opentelemetry_sdk=off,opentelemetry_otlp=off,\
                                 opentelemetry_prometheus=off,tonic=off,h2=off,hyper=off,\
                                 hyper_util=off,reqwest=off,tower=off,tower_http=off";

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builds an [`Observability`]. Every signal is optional.
#[must_use]
pub struct ObservabilityBuilder {
    service: ServiceInfo,
    #[cfg(feature = "metrics")]
    metrics: Option<MetricsConfig>,
    #[cfg(feature = "tracing")]
    traces: Option<TracesConfig>,
    #[cfg(feature = "logs")]
    logs: Option<LogsConfig>,
    install_subscriber: bool,
    shutdown_timeout: Duration,
    http_exclude: Vec<String>,
    http_scheme: &'static str,
}

impl std::fmt::Debug for ObservabilityBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("ObservabilityBuilder");
        d.field("service", &self.service);
        #[cfg(feature = "metrics")]
        d.field("metrics", &self.metrics);
        #[cfg(feature = "tracing")]
        d.field("traces", &self.traces);
        #[cfg(feature = "logs")]
        d.field("logs", &self.logs);
        d.field("install_subscriber", &self.install_subscriber)
            .field("shutdown_timeout", &self.shutdown_timeout)
            .field("http_exclude", &self.http_exclude)
            .finish()
    }
}

impl Default for ObservabilityBuilder {
    fn default() -> Self {
        Self {
            service: ServiceInfo::default(),
            #[cfg(feature = "metrics")]
            metrics: None,
            #[cfg(feature = "tracing")]
            traces: None,
            #[cfg(feature = "logs")]
            logs: None,
            install_subscriber: true,
            shutdown_timeout: Duration::from_secs(5),
            http_exclude: Vec::new(),
            http_scheme: "http",
        }
    }
}

impl ObservabilityBuilder {
    /// `service.name`. Falls back to `OTEL_SERVICE_NAME`, then
    /// `unknown_service`.
    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.service.name = Some(name.into());
        self
    }

    /// `service.version`.
    pub fn service_version(mut self, version: impl Into<String>) -> Self {
        self.service.version = Some(version.into());
        self
    }

    /// `service.namespace`.
    pub fn service_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.service.namespace = Some(namespace.into());
        self
    }

    /// `deployment.environment.name` (`production`, `staging`, ...).
    pub fn environment(mut self, environment: impl Into<String>) -> Self {
        self.service.environment = Some(environment.into());
        self
    }

    /// `service.instance.id`. Default: `<hostname>-<pid>-<start time>`.
    pub fn instance_id(mut self, id: impl Into<String>) -> Self {
        self.service.instance_id = Some(id.into());
        self
    }

    /// Adds a resource attribute (`cloud.region`, `k8s.pod.name`, ...).
    /// `OTEL_RESOURCE_ATTRIBUTES` is also honored.
    pub fn resource_attribute(
        mut self,
        key: impl Into<opentelemetry::Key>,
        value: impl Into<opentelemetry::Value>,
    ) -> Self {
        self.service.attributes.push(KeyValue::new(key, value));
        self
    }

    /// Enables metrics.
    #[cfg(feature = "metrics")]
    pub fn metrics(mut self, config: MetricsConfig) -> Self {
        self.metrics = Some(config);
        self
    }

    /// Enables tracing.
    #[cfg(feature = "tracing")]
    pub fn traces(mut self, config: TracesConfig) -> Self {
        self.traces = Some(config);
        self
    }

    /// Enables logging.
    #[cfg(feature = "logs")]
    pub fn logs(mut self, config: LogsConfig) -> Self {
        self.logs = Some(config);
        self
    }

    /// Whether [`build`](Self::build) installs the global `tracing`
    /// subscriber (default `true`). Building fails with
    /// [`Error::SubscriberAlreadySet`] instead of silently replacing one.
    /// Set `false` to compose [`Observability::tracing_layer`] into your
    /// own subscriber.
    pub fn install_subscriber(mut self, install: bool) -> Self {
        self.install_subscriber = install;
        self
    }

    /// Upper bound on [`Observability::shutdown`]. Default 5 s.
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Excludes a route template (or, for unmatched requests, a path) from
    /// HTTP metrics and traces, e.g. `/health`.
    pub fn exclude_http_route(mut self, route: impl Into<String>) -> Self {
        self.http_exclude.push(route.into());
        self
    }

    /// The `url.scheme` to report for HTTP/1 requests (`"https"` when TLS
    /// terminates in front of the service). Default `"http"`.
    pub fn http_scheme(mut self, scheme: &'static str) -> Self {
        self.http_scheme = scheme;
        self
    }

    /// Builds the providers and exporters and, unless disabled, installs the
    /// `tracing` subscriber.
    ///
    /// Must be called inside a Tokio runtime: the OTLP/gRPC exporter and the
    /// runtime metrics bind to the current runtime. No runtime is created.
    /// Exporters connect lazily, so an unreachable Collector does not fail
    /// the build.
    pub async fn build(self) -> Result<Observability, Error> {
        self.build_sync()
    }

    fn build_sync(self) -> Result<Observability, Error> {
        let ObservabilityBuilder {
            mut service,
            #[cfg(feature = "metrics")]
            metrics,
            #[cfg(feature = "tracing")]
            traces,
            #[cfg(feature = "logs")]
            logs,
            install_subscriber,
            shutdown_timeout,
            http_exclude,
            http_scheme,
        } = self;

        if service.instance_id.is_none() {
            service.instance_id = Some(kerniq_telemetry::resource::default_instance_id());
        }
        #[cfg(any(feature = "metrics", feature = "tracing", feature = "logs"))]
        let resource = {
            let mut builder = opentelemetry_sdk::Resource::builder();
            if let Some(name) = &service.name {
                builder = builder.with_service_name(name.clone());
            }
            builder.with_attributes(service.to_attributes()).build()
        };

        let counters = Arc::new(Counters::default());

        // --- metrics ------------------------------------------------------
        #[cfg(feature = "metrics")]
        let mut http_metrics = false;
        #[cfg(feature = "prometheus")]
        let mut prometheus_handle = None;
        #[cfg(feature = "metrics")]
        let meter_provider = match metrics {
            None => None,
            Some(config) => {
                http_metrics = config.http;
                let (provider, _prometheus) = build_metrics(config, &resource, &counters)?;
                #[cfg(feature = "prometheus")]
                {
                    prometheus_handle = _prometheus;
                }
                Some(provider)
            }
        };

        // --- traces -------------------------------------------------------
        #[cfg(feature = "tracing")]
        let (tracer_provider, traces_filter, activate_context) = match traces {
            None => (None, None, false),
            Some(config) => {
                let filter = config.filter.clone();
                let activate = config.activate_context;
                (
                    Some(build_traces(config, &resource, &counters)?),
                    filter,
                    activate,
                )
            }
        };

        // --- logs ---------------------------------------------------------
        #[cfg(feature = "logs")]
        let mut console = None;
        #[cfg(feature = "logs")]
        let mut console_guard = None;
        #[cfg(feature = "logs")]
        let (logger_provider, logs_filter, bridge_filter) = match logs {
            None => (None, None, None),
            Some(config) => {
                let filter = config.filter.clone();
                #[cfg(feature = "otlp")]
                let bridge_filter = config.otlp_filter.clone().or_else(|| filter.clone());
                #[cfg(not(feature = "otlp"))]
                let bridge_filter = filter.clone();
                if let Some((format, target)) = config.console {
                    let (writer, guard) = console_writer(target, config.non_blocking, &counters);
                    console = Some(ConsoleSetup {
                        format,
                        writer,
                        target,
                    });
                    console_guard = guard;
                }
                let provider = if config.has_bridge() {
                    Some(build_logs(config, &resource, &counters)?)
                } else {
                    None
                };
                (provider, filter, bridge_filter)
            }
        };

        // --- HTTP instrumentation -----------------------------------------
        #[cfg(feature = "tracing")]
        let propagator: Arc<
            dyn opentelemetry::propagation::TextMapPropagator + Send + Sync,
        > = Arc::new(opentelemetry::propagation::TextMapCompositePropagator::new(
            vec![
                Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()),
                Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new()),
            ],
        ));

        #[cfg_attr(not(feature = "prometheus"), allow(unused_mut))]
        let mut exclude = http_exclude;
        #[cfg(feature = "prometheus")]
        if let Some(prom) = &prometheus_handle {
            exclude.push(prom.config().scrape_path().to_owned());
        }

        let inner = Inner {
            #[cfg(any(feature = "metrics", feature = "tracing", feature = "logs"))]
            resource,
            #[cfg(feature = "metrics")]
            meter_provider,
            #[cfg(feature = "metrics")]
            http_metrics,
            #[cfg(feature = "prometheus")]
            prometheus: prometheus_handle,
            #[cfg(feature = "tracing")]
            tracer_provider,
            #[cfg(feature = "tracing")]
            traces_filter,
            #[cfg(feature = "tracing")]
            activate_context,
            #[cfg(feature = "tracing")]
            propagator,
            #[cfg(feature = "logs")]
            logger_provider,
            #[cfg(feature = "logs")]
            console,
            #[cfg(feature = "logs")]
            console_guard: std::sync::Mutex::new(console_guard),
            #[cfg(feature = "logs")]
            logs_filter,
            #[cfg(feature = "logs")]
            bridge_filter,
            service,
            counters,
            http_exclude: exclude,
            http_scheme,
            shutdown_timeout,
            shut_down: AtomicBool::new(false),
        };
        let observability = Observability {
            inner: Arc::new(inner),
        };

        if install_subscriber && observability.has_subscriber_layers() {
            observability.install_subscriber()?;
        }
        Ok(observability)
    }
}

#[cfg(feature = "metrics")]
fn build_metrics(
    config: MetricsConfig,
    resource: &opentelemetry_sdk::Resource,
    counters: &Arc<Counters>,
) -> Result<
    (
        opentelemetry_sdk::metrics::SdkMeterProvider,
        Option<PrometheusHandle>,
    ),
    Error,
> {
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    let mut builder = SdkMeterProvider::builder().with_resource(resource.clone());
    if let Some(limit) = config.cardinality_limit {
        if limit == 0 {
            return Err(Error::Config(
                "cardinality_limit must be greater than zero".into(),
            ));
        }
        builder = builder.with_view(move |_: &opentelemetry_sdk::metrics::Instrument| {
            opentelemetry_sdk::metrics::Stream::builder()
                .with_cardinality_limit(limit)
                .build()
                .ok()
        });
    }

    #[cfg(feature = "otlp")]
    for exporter in &config.otlp {
        let exporter = stats::metrics::CountingExporter {
            inner: otlp::metric_exporter(exporter, config.delta_temporality)?,
            counters: counters.clone(),
        };
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
            .with_interval(config.export_interval)
            .build();
        builder = builder.with_reader(reader);
    }

    #[cfg(feature = "prometheus")]
    let prometheus = match &config.prometheus {
        Some(prom) => {
            let (reader, handle) = PrometheusExporter::build(prom.clone())?;
            builder = builder.with_reader(reader);
            Some(handle)
        }
        None => None,
    };
    #[cfg(not(feature = "prometheus"))]
    let prometheus = None;

    for hook in config.readers {
        builder = hook(builder, config.export_interval);
    }
    let provider = builder.build();

    let internal = provider.meter("kerniq.observability");
    stats::metrics::register(&internal, counters);
    #[cfg(feature = "runtime")]
    if config.runtime {
        runtime::register(&internal);
    }
    Ok((provider, prometheus))
}

#[cfg(feature = "tracing")]
fn build_traces(
    config: TracesConfig,
    resource: &opentelemetry_sdk::Resource,
    counters: &Arc<Counters>,
) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, Error> {
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};

    config.batch.validate("traces").map_err(Error::Config)?;
    let sampler = match config.sampling {
        Sampling::AlwaysOn => Sampler::AlwaysOn,
        Sampling::AlwaysOff => Sampler::AlwaysOff,
        Sampling::Ratio(ratio) => {
            if !(0.0..=1.0).contains(&ratio) {
                return Err(Error::Config(format!(
                    "sampling ratio {ratio} is outside 0.0..=1.0"
                )));
            }
            Sampler::TraceIdRatioBased(ratio)
        }
    };
    let sampler = if config.parent_based {
        Sampler::ParentBased(Box::new(sampler))
    } else {
        sampler
    };
    let mut builder = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_sampler(sampler);

    #[cfg(feature = "otlp")]
    for exporter in &config.otlp {
        let batch = opentelemetry_sdk::trace::BatchConfigBuilder::default()
            .with_max_queue_size(config.batch.max_queue_size)
            .with_max_export_batch_size(config.batch.max_export_batch_size)
            .with_scheduled_delay(config.batch.scheduled_delay)
            .build();
        let exporter = stats::traces::CountingExporter {
            inner: otlp::span_exporter(
                &exporter.with_default_timeout(config.batch.export_timeout),
            )?,
            counters: counters.clone(),
        };
        let processor = opentelemetry_sdk::trace::BatchSpanProcessor::builder(exporter)
            .with_batch_config(batch)
            .build();
        builder = builder.with_span_processor(stats::traces::CountingProcessor {
            inner: processor,
            counters: counters.clone(),
        });
    }
    #[cfg(not(feature = "otlp"))]
    let _ = counters;

    for hook in config.processors {
        builder = hook(builder);
    }
    Ok(builder.build())
}

#[cfg(feature = "logs")]
fn build_logs(
    config: LogsConfig,
    resource: &opentelemetry_sdk::Resource,
    counters: &Arc<Counters>,
) -> Result<opentelemetry_sdk::logs::SdkLoggerProvider, Error> {
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    config.batch.validate("logs").map_err(Error::Config)?;
    let mut builder = SdkLoggerProvider::builder().with_resource(resource.clone());

    #[cfg(feature = "otlp")]
    for exporter in &config.otlp {
        let batch = opentelemetry_sdk::logs::BatchConfigBuilder::default()
            .with_max_queue_size(config.batch.max_queue_size)
            .with_max_export_batch_size(config.batch.max_export_batch_size)
            .with_scheduled_delay(config.batch.scheduled_delay)
            .build();
        let exporter = stats::logs::CountingExporter {
            inner: otlp::log_exporter(&exporter.with_default_timeout(config.batch.export_timeout))?,
            counters: counters.clone(),
        };
        let processor = opentelemetry_sdk::logs::BatchLogProcessor::builder(exporter)
            .with_batch_config(batch)
            .build();
        builder = builder.with_log_processor(stats::logs::CountingProcessor {
            inner: processor,
            counters: counters.clone(),
        });
    }
    #[cfg(not(feature = "otlp"))]
    let _ = counters;

    for hook in config.processors {
        builder = hook(builder);
    }
    Ok(builder.build())
}

#[cfg(feature = "logs")]
#[derive(Clone)]
enum ConsoleWriter {
    NonBlocking(tracing_appender::non_blocking::NonBlocking),
    Stdout,
    Stderr,
}

#[cfg(feature = "logs")]
struct ConsoleSetup {
    format: ConsoleFormat,
    writer: ConsoleWriter,
    target: ConsoleTarget,
}

#[cfg(feature = "logs")]
fn console_writer(
    target: ConsoleTarget,
    buffered_lines: Option<usize>,
    counters: &Arc<Counters>,
) -> (
    ConsoleWriter,
    Option<tracing_appender::non_blocking::WorkerGuard>,
) {
    let Some(lines) = buffered_lines else {
        return (
            match target {
                ConsoleTarget::Stdout => ConsoleWriter::Stdout,
                ConsoleTarget::Stderr => ConsoleWriter::Stderr,
            },
            None,
        );
    };
    let builder = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(lines.max(1))
        .lossy(true)
        .thread_name("kerniq-log-writer");
    let (writer, guard) = match target {
        ConsoleTarget::Stdout => builder.finish(std::io::stdout()),
        ConsoleTarget::Stderr => builder.finish(std::io::stderr()),
    };
    let _ = counters.console_dropped.set(writer.error_counter());
    (ConsoleWriter::NonBlocking(writer), Some(guard))
}

#[cfg(feature = "prometheus")]
type PrometheusHandle = PrometheusExporter;
#[cfg(all(feature = "metrics", not(feature = "prometheus")))]
type PrometheusHandle = ();

// ---------------------------------------------------------------------------
// Observability
// ---------------------------------------------------------------------------

struct Inner {
    #[cfg(any(feature = "metrics", feature = "tracing", feature = "logs"))]
    resource: opentelemetry_sdk::Resource,
    #[cfg(feature = "metrics")]
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    #[cfg(feature = "metrics")]
    http_metrics: bool,
    #[cfg(feature = "prometheus")]
    prometheus: Option<PrometheusExporter>,
    #[cfg(feature = "tracing")]
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(feature = "tracing")]
    traces_filter: Option<String>,
    #[cfg(feature = "tracing")]
    activate_context: bool,
    #[cfg(feature = "tracing")]
    propagator: Arc<dyn opentelemetry::propagation::TextMapPropagator + Send + Sync>,
    #[cfg(feature = "logs")]
    logger_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
    #[cfg(feature = "logs")]
    console: Option<ConsoleSetup>,
    #[cfg(feature = "logs")]
    console_guard: std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
    #[cfg(feature = "logs")]
    logs_filter: Option<String>,
    #[cfg(feature = "logs")]
    bridge_filter: Option<String>,
    service: ServiceInfo,
    counters: Arc<Counters>,
    http_exclude: Vec<String>,
    http_scheme: &'static str,
    shutdown_timeout: Duration,
    shut_down: AtomicBool,
}

/// The configured telemetry providers. Cheap to clone; every clone refers
/// to the same providers.
///
/// Dropping the last clone without calling [`shutdown`](Self::shutdown)
/// lets the SDK flush on drop, which may block; call `shutdown` explicitly
/// (Kerniq's `App` does so after draining connections).
#[derive(Clone)]
pub struct Observability {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Observability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let i = &*self.inner;
        let mut d = f.debug_struct("Observability");
        d.field("service", &i.service.name);
        #[cfg(feature = "metrics")]
        d.field("metrics", &i.meter_provider.is_some());
        #[cfg(feature = "prometheus")]
        d.field(
            "prometheus",
            &i.prometheus
                .as_ref()
                .map(|p| p.config().scrape_path().to_owned()),
        );
        #[cfg(feature = "tracing")]
        d.field("traces", &i.tracer_provider.is_some());
        #[cfg(feature = "logs")]
        d.field("console_logs", &i.console.as_ref().map(|c| c.format))
            .field("log_export", &i.logger_provider.is_some());
        d.finish_non_exhaustive()
    }
}

/// The result of [`Observability::shutdown`].
#[derive(Debug, Clone)]
pub struct ShutdownReport {
    /// How long shutdown took.
    pub duration: Duration,
    /// Pipeline counters after the final flush. `unexported` is now the
    /// number of items dropped.
    pub stats: TelemetryStats,
    /// Errors reported by providers (export failures during the final flush,
    /// timeouts).
    pub errors: Vec<String>,
    /// Whether the shutdown timeout elapsed before every provider finished.
    pub timed_out: bool,
}

impl Observability {
    /// Starts configuring observability.
    pub fn builder() -> ObservabilityBuilder {
        ObservabilityBuilder::default()
    }

    /// A handle with every signal disabled.
    pub fn disabled() -> Self {
        ObservabilityBuilder::default()
            .install_subscriber(false)
            .build_sync()
            .expect("an empty configuration is always valid")
    }

    /// The service identity.
    pub fn service(&self) -> &ServiceInfo {
        &self.inner.service
    }

    /// The OpenTelemetry resource attached to every signal.
    #[cfg(any(feature = "metrics", feature = "tracing", feature = "logs"))]
    pub fn resource(&self) -> &opentelemetry_sdk::Resource {
        &self.inner.resource
    }

    /// A meter for application instruments. Create instruments once and
    /// reuse them; recording is lock-free in the common case. Returns a
    /// no-op meter when metrics are disabled, so instrumentation code never
    /// needs to check.
    #[cfg(feature = "metrics")]
    pub fn meter(&self, name: &'static str) -> opentelemetry::metrics::Meter {
        use opentelemetry::metrics::MeterProvider as _;
        match &self.inner.meter_provider {
            Some(provider) => provider.meter(name),
            None => opentelemetry::metrics::NoopMeterProvider::new().meter(name),
        }
    }

    /// The SDK meter provider, when metrics are enabled.
    #[cfg(feature = "metrics")]
    pub fn meter_provider(&self) -> Option<&opentelemetry_sdk::metrics::SdkMeterProvider> {
        self.inner.meter_provider.as_ref()
    }

    /// A tracer for manual spans through the OpenTelemetry API. Most code
    /// should use `tracing` spans instead, which are exported through the
    /// same provider.
    #[cfg(feature = "tracing")]
    pub fn tracer(&self, name: &'static str) -> opentelemetry_sdk::trace::Tracer {
        use opentelemetry::trace::TracerProvider as _;
        match &self.inner.tracer_provider {
            Some(provider) => provider.tracer(name),
            None => opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOff)
                .build()
                .tracer(name),
        }
    }

    /// The SDK tracer provider, when tracing is enabled.
    #[cfg(feature = "tracing")]
    pub fn tracer_provider(&self) -> Option<&opentelemetry_sdk::trace::SdkTracerProvider> {
        self.inner.tracer_provider.as_ref()
    }

    /// The W3C TraceContext + Baggage propagator, for outbound calls
    /// (see `kerniq_observability_axum::propagation`).
    #[cfg(feature = "tracing")]
    pub fn propagator(
        &self,
    ) -> Arc<dyn opentelemetry::propagation::TextMapPropagator + Send + Sync> {
        self.inner.propagator.clone()
    }

    /// The SDK logger provider, when log export is enabled.
    #[cfg(feature = "logs")]
    pub fn logger_provider(&self) -> Option<&opentelemetry_sdk::logs::SdkLoggerProvider> {
        self.inner.logger_provider.as_ref()
    }

    /// The Prometheus endpoint, when configured.
    #[cfg(feature = "prometheus")]
    pub fn prometheus(&self) -> Option<&PrometheusExporter> {
        self.inner.prometheus.as_ref()
    }

    /// HTTP instrumentation configured from this handle: metrics when
    /// metrics are enabled, spans and W3C propagation when tracing is
    /// enabled, and the configured exclusions (the Prometheus path is
    /// always excluded).
    pub fn http_instrumentation(&self) -> HttpInstrumentation {
        let i = &*self.inner;
        let mut http = HttpInstrumentation::new().scheme(i.http_scheme);
        #[cfg(feature = "metrics")]
        if i.http_metrics
            && let Some(provider) = &i.meter_provider
        {
            use opentelemetry::metrics::MeterProvider as _;
            let meter = provider.meter_with_scope(
                opentelemetry::InstrumentationScope::builder("kerniq.http")
                    .with_version(env!("CARGO_PKG_VERSION"))
                    .build(),
            );
            http = http.meter(&meter);
        }
        #[cfg(feature = "tracing")]
        {
            http = http.tracing(i.tracer_provider.is_some() || self.has_console_logs());
            if i.tracer_provider.is_some() {
                http = http.shared_propagator(i.propagator.clone());
            }
        }
        for route in &i.http_exclude {
            http = http.exclude(route.clone());
        }
        http
    }

    /// The OpenTelemetry log bridge reads the trace context from
    /// `opentelemetry::Context::current()`, so exported log records only
    /// carry trace ids when spans activate their context.
    #[cfg(feature = "tracing")]
    fn needs_context_activation(&self) -> bool {
        #[cfg(feature = "logs")]
        if self.inner.logger_provider.is_some() {
            return true;
        }
        self.inner.activate_context
    }

    #[cfg(feature = "tracing")]
    fn has_console_logs(&self) -> bool {
        #[cfg(feature = "logs")]
        return self.inner.console.is_some();
        #[cfg(not(feature = "logs"))]
        false
    }

    /// Export-pipeline counters (see [`TelemetryStats`]). Also published as
    /// the `kerniq.telemetry.items` and `kerniq.telemetry.exports` metrics.
    pub fn stats(&self) -> TelemetryStats {
        self.inner.counters.snapshot()
    }

    /// Exports everything buffered now. Blocks the calling thread until the
    /// exporters return; use from `spawn_blocking` in async code.
    pub fn force_flush(&self) -> Vec<String> {
        #[allow(unused_mut)]
        let mut errors = Vec::new();
        #[cfg(feature = "tracing")]
        if let Some(p) = &self.inner.tracer_provider
            && let Err(e) = p.force_flush()
        {
            errors.push(format!("traces: {e}"));
        }
        #[cfg(feature = "logs")]
        if let Some(p) = &self.inner.logger_provider
            && let Err(e) = p.force_flush()
        {
            errors.push(format!("logs: {e}"));
        }
        #[cfg(feature = "metrics")]
        if let Some(p) = &self.inner.meter_provider
            && let Err(e) = p.force_flush()
        {
            errors.push(format!("metrics: {e}"));
        }
        errors
    }

    /// Flushes and shuts down every provider, in at most the configured
    /// shutdown timeout. Idempotent; later calls return immediately.
    ///
    /// Call it after the server has stopped accepting requests and drained
    /// (Kerniq's `App` does this automatically). Providers are shut down on
    /// Tokio's blocking pool, so the async runtime stays responsive even if
    /// a Collector is unreachable; the caller waits at most the timeout.
    pub async fn shutdown(&self) -> ShutdownReport {
        let start = Instant::now();
        if self.inner.shut_down.swap(true, Ordering::AcqRel) {
            return ShutdownReport {
                duration: Duration::ZERO,
                stats: self.stats(),
                errors: Vec::new(),
                timed_out: false,
            };
        }
        let timeout = self.inner.shutdown_timeout;
        let inner = self.inner.clone();
        let task = tokio::task::spawn_blocking(move || shutdown_providers(&inner, timeout));
        #[allow(unused_mut)]
        let (mut errors, timed_out) = match tokio::time::timeout(timeout, task).await {
            Ok(Ok(errors)) => (errors, false),
            Ok(Err(join)) => (vec![format!("shutdown task failed: {join}")], false),
            Err(_) => (
                vec![format!(
                    "telemetry shutdown did not finish within {timeout:?}"
                )],
                true,
            ),
        };
        // Console logs last, so shutdown messages are written.
        #[cfg(feature = "logs")]
        {
            let guard = self
                .inner
                .console_guard
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            if let Some(guard) = guard
                && tokio::task::spawn_blocking(move || drop(guard))
                    .await
                    .is_err()
            {
                errors.push("console log writer did not flush".to_owned());
            }
        }
        let report = ShutdownReport {
            duration: start.elapsed(),
            stats: self.stats(),
            errors,
            timed_out,
        };
        tracing::debug!(duration = ?report.duration, timed_out, "telemetry shut down");
        report
    }

    /// Sets this handle's providers and propagator as the OpenTelemetry
    /// globals (`opentelemetry::global`), for libraries that instrument
    /// through the global API. Never done implicitly.
    pub fn install_global_providers(&self) {
        #[cfg(feature = "metrics")]
        if let Some(p) = &self.inner.meter_provider {
            opentelemetry::global::set_meter_provider(p.clone());
        }
        #[cfg(feature = "tracing")]
        {
            if let Some(p) = &self.inner.tracer_provider {
                opentelemetry::global::set_tracer_provider(p.clone());
            }
            opentelemetry::global::set_text_map_propagator(
                opentelemetry::propagation::TextMapCompositePropagator::new(vec![
                    Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()),
                    Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new()),
                ]),
            );
        }
    }

    fn has_subscriber_layers(&self) -> bool {
        #[allow(unused_mut)]
        let mut any = false;
        #[cfg(feature = "tracing")]
        {
            any |= self.inner.tracer_provider.is_some();
        }
        #[cfg(feature = "logs")]
        {
            any |= self.inner.console.is_some() || self.inner.logger_provider.is_some();
        }
        any
    }

    fn install_subscriber(&self) -> Result<(), Error> {
        #[cfg(any(feature = "tracing", feature = "logs"))]
        {
            use tracing_subscriber::layer::SubscriberExt as _;
            let subscriber = tracing_subscriber::registry().with(self.tracing_layer());
            tracing::subscriber::set_global_default(subscriber)
                .map_err(|_| Error::SubscriberAlreadySet)?;
        }
        Ok(())
    }

    /// The `tracing` layers for this configuration (console output, span
    /// export, log export), each with its own filter, for composing into an
    /// application-owned subscriber (with
    /// [`install_subscriber(false)`](ObservabilityBuilder::install_subscriber)).
    ///
    /// ```no_run
    /// use tracing_subscriber::layer::SubscriberExt;
    /// # async fn demo() -> Result<(), kerniq_observability::Error> {
    /// use kerniq_observability::prelude::*;
    ///
    /// let observability = Observability::builder()
    ///     .logs(LogsConfig::json())
    ///     .install_subscriber(false)
    ///     .build()
    ///     .await?;
    /// let subscriber = tracing_subscriber::registry()
    ///     .with(observability.tracing_layer())
    ///     // .with(your_other_layer)
    ///     ;
    /// tracing::subscriber::set_global_default(subscriber).unwrap();
    /// # Ok(()) }
    /// ```
    #[cfg(any(feature = "tracing", feature = "logs"))]
    pub fn tracing_layer<S>(&self) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
    {
        use tracing_subscriber::Layer as _;
        type Boxed<S> = Box<dyn tracing_subscriber::Layer<S> + Send + Sync>;
        // Chained with `and_then` rather than collected in a `Vec`: a `Vec`
        // of layers does not forward `on_register_dispatch`, which the JSON
        // format needs for trace correlation.
        #[allow(unused_mut)]
        let mut layers: Vec<Boxed<S>> = Vec::new();

        #[cfg(feature = "logs")]
        if let Some(console) = &self.inner.console {
            let filter = env_filter(self.inner.logs_filter.as_deref(), None);
            layers.push(console_layer::<S>(console).with_filter(filter).boxed());
        }

        #[cfg(feature = "tracing")]
        if let Some(provider) = &self.inner.tracer_provider {
            use opentelemetry::trace::TracerProvider as _;
            let tracer = provider.tracer("kerniq");
            let filter = env_filter(
                Some(self.inner.traces_filter.as_deref().unwrap_or("info")),
                Some(BRIDGE_EXCLUSIONS),
            );
            layers.push(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_error_records_to_exceptions(true)
                    // Busy/idle time reads the clock on every enter/exit, which
                    // for a streamed response is every frame.
                    .with_tracked_inactivity(false)
                    .with_context_activation(self.needs_context_activation())
                    .with_filter(filter)
                    .boxed(),
            );
        }

        #[cfg(feature = "logs")]
        if let Some(provider) = &self.inner.logger_provider {
            let filter = env_filter(self.inner.bridge_filter.as_deref(), Some(BRIDGE_EXCLUSIONS));
            layers.push(
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider)
                    .with_filter(filter)
                    .boxed(),
            );
        }

        layers
            .into_iter()
            .reduce(|chain, layer| chain.and_then(layer).boxed())
            .unwrap_or_else(|| tracing_subscriber::layer::Identity::new().boxed())
    }
}

/// The OpenTelemetry context of the current `tracing` span (empty when the
/// span is not exported).
#[cfg(feature = "tracing")]
pub fn current_context() -> opentelemetry::Context {
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    tracing::Span::current().context()
}

/// The trace id of the current `tracing` span, as 32 lowercase hex digits,
/// when it belongs to an exported trace. Useful to return in error
/// responses so users can quote it in support requests.
#[cfg(feature = "tracing")]
pub fn current_trace_id() -> Option<String> {
    use opentelemetry::trace::TraceContextExt as _;
    let cx = current_context();
    let span = cx.span();
    let sc = span.span_context();
    sc.is_valid().then(|| sc.trace_id().to_string())
}

#[cfg(any(feature = "tracing", feature = "logs"))]
fn env_filter(directives: Option<&str>, extra: Option<&str>) -> tracing_subscriber::EnvFilter {
    let base = directives
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("RUST_LOG")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "info".to_owned());
    let full = match extra {
        Some(extra) => format!("{base},{extra}"),
        None => base,
    };
    tracing_subscriber::EnvFilter::builder()
        .parse(&full)
        .unwrap_or_else(|err| {
            eprintln!("kerniq-observability: invalid log filter `{full}` ({err}); using `info`");
            tracing_subscriber::EnvFilter::new("info")
        })
}

#[cfg(feature = "logs")]
fn console_layer<S>(console: &ConsoleSetup) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
{
    use std::io::IsTerminal as _;
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::fmt::writer::BoxMakeWriter;

    let writer = match console.writer.clone() {
        ConsoleWriter::NonBlocking(w) => BoxMakeWriter::new(w),
        ConsoleWriter::Stdout => BoxMakeWriter::new(std::io::stdout),
        ConsoleWriter::Stderr => BoxMakeWriter::new(std::io::stderr),
    };
    let ansi = match console.target {
        ConsoleTarget::Stdout => std::io::stdout().is_terminal(),
        ConsoleTarget::Stderr => std::io::stderr().is_terminal(),
    };
    match console.format {
        ConsoleFormat::Json => logging::json_layer(writer),
        ConsoleFormat::Pretty => tracing_subscriber::fmt::layer()
            .pretty()
            .with_ansi(ansi)
            .with_writer(writer)
            .boxed(),
        ConsoleFormat::Compact => tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(ansi)
            .with_writer(writer)
            .boxed(),
    }
}

fn shutdown_providers(inner: &Inner, timeout: Duration) -> Vec<String> {
    let deadline = Instant::now() + timeout;
    let remaining = || deadline.saturating_duration_since(Instant::now());
    #[allow(unused_mut)]
    let mut errors = Vec::new();
    // Traces and logs first: their final export may still be recorded in
    // metrics (`kerniq.telemetry.*`) by the metrics shutdown that follows.
    #[cfg(feature = "tracing")]
    if let Some(p) = &inner.tracer_provider
        && let Err(e) = p.shutdown_with_timeout(remaining())
    {
        errors.push(format!("traces: {e}"));
    }
    #[cfg(feature = "logs")]
    if let Some(p) = &inner.logger_provider
        && let Err(e) = p.shutdown_with_timeout(remaining())
    {
        errors.push(format!("logs: {e}"));
    }
    #[cfg(feature = "metrics")]
    if let Some(p) = &inner.meter_provider
        && let Err(e) = p.shutdown_with_timeout(remaining())
    {
        errors.push(format!("metrics: {e}"));
    }
    let _ = (remaining, inner);
    errors
}
