//! Per-signal configuration.
//!
//! Every signal is configured independently and is off unless configured.
//! Every pipeline is bounded: see [`BatchConfig`] for queue sizes and the
//! overload policy.

#[allow(unused_imports)]
use std::fmt;
use std::time::Duration;

#[cfg(feature = "otlp")]
use crate::otlp::OtlpExporter;

/// Bounded batching for an asynchronous export pipeline (traces, logs).
///
/// Telemetry is handed to a bounded queue drained by one SDK-managed
/// background thread per pipeline. The request path only performs a
/// non-blocking enqueue.
///
/// **Overload policy:** when the queue is full, new items are **dropped**
/// (never blocking the caller) and counted; see
/// [`Observability::stats`](crate::Observability::stats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchConfig {
    /// Maximum queued items. Memory is bounded by this times the item size.
    pub max_queue_size: usize,
    /// Maximum items per export call.
    pub max_export_batch_size: usize,
    /// Delay between scheduled exports.
    pub scheduled_delay: Duration,
    /// Maximum duration of one export request. Applied as the OTLP
    /// exporter's request timeout unless the exporter sets its own
    /// [`timeout`](crate::OtlpExporter::timeout).
    pub export_timeout: Duration,
}

impl BatchConfig {
    /// The OpenTelemetry defaults for spans: queue 2048, batch 512,
    /// every 5 s; 10 s export timeout (the OTLP default).
    pub const TRACES: Self = Self {
        max_queue_size: 2048,
        max_export_batch_size: 512,
        scheduled_delay: Duration::from_secs(5),
        export_timeout: Duration::from_secs(10),
    };
    /// The OpenTelemetry defaults for logs: queue 2048, batch 512, every
    /// 1 s; 10 s export timeout.
    pub const LOGS: Self = Self {
        max_queue_size: 2048,
        max_export_batch_size: 512,
        scheduled_delay: Duration::from_secs(1),
        export_timeout: Duration::from_secs(10),
    };

    #[cfg_attr(not(any(feature = "tracing", feature = "logs")), allow(dead_code))]
    pub(crate) fn validate(&self, signal: &str) -> Result<(), String> {
        if self.max_queue_size == 0 {
            return Err(format!(
                "{signal}: max_queue_size must be greater than zero"
            ));
        }
        if self.max_export_batch_size == 0 || self.max_export_batch_size > self.max_queue_size {
            return Err(format!(
                "{signal}: max_export_batch_size must be between 1 and max_queue_size ({})",
                self.max_queue_size
            ));
        }
        Ok(())
    }
}

/// Trace sampling.
///
/// Metrics are never sampled; logs are filtered by level. Sampling only
/// reduces trace volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sampling {
    /// Record every trace.
    AlwaysOn,
    /// Record no traces (spans still propagate context).
    AlwaysOff,
    /// Record this fraction of traces (0.0..=1.0), decided from the trace
    /// id so every service in a trace makes the same decision.
    Ratio(f64),
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Metrics configuration. Build with [`MetricsConfig::prometheus`] or
/// [`MetricsConfig::otlp`]; both can be combined.
#[cfg(feature = "metrics")]
#[must_use]
pub struct MetricsConfig {
    #[cfg(feature = "prometheus")]
    pub(crate) prometheus: Option<kerniq_observability_prometheus::PrometheusConfig>,
    #[cfg(feature = "otlp")]
    pub(crate) otlp: Vec<OtlpExporter>,
    pub(crate) primary: Primary,
    pub(crate) export_interval: Duration,
    pub(crate) delta_temporality: bool,
    pub(crate) cardinality_limit: Option<usize>,
    pub(crate) http: bool,
    #[cfg(feature = "runtime")]
    pub(crate) runtime: bool,
    pub(crate) readers: Vec<ReaderHook>,
}

#[cfg(feature = "metrics")]
pub(crate) type ReaderHook = Box<
    dyn FnOnce(
            opentelemetry_sdk::metrics::MeterProviderBuilder,
            Duration,
        ) -> opentelemetry_sdk::metrics::MeterProviderBuilder
        + Send,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Primary {
    None,
    #[cfg(feature = "prometheus")]
    Prometheus,
    #[cfg(feature = "otlp")]
    Otlp,
}

#[cfg(feature = "metrics")]
impl fmt::Debug for MetricsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("MetricsConfig");
        #[cfg(feature = "prometheus")]
        d.field("prometheus", &self.prometheus);
        #[cfg(feature = "otlp")]
        d.field("otlp", &self.otlp);
        d.field("export_interval", &self.export_interval)
            .field("delta_temporality", &self.delta_temporality)
            .field("cardinality_limit", &self.cardinality_limit)
            .field("http", &self.http)
            .field("custom_readers", &self.readers.len())
            .finish()
    }
}

#[cfg(feature = "metrics")]
impl MetricsConfig {
    fn empty(primary: Primary) -> Self {
        Self {
            #[cfg(feature = "prometheus")]
            prometheus: None,
            #[cfg(feature = "otlp")]
            otlp: Vec::new(),
            primary,
            export_interval: Duration::from_secs(60),
            delta_temporality: false,
            cardinality_limit: None,
            http: true,
            #[cfg(feature = "runtime")]
            runtime: false,
            readers: Vec::new(),
        }
    }

    /// Metrics recorded in-process with no exporter configured yet. Add one
    /// with [`with_prometheus`](Self::with_prometheus),
    /// [`with_otlp`](Self::with_otlp) or [`exporter`](Self::exporter).
    pub fn new() -> Self {
        Self::empty(Primary::None)
    }

    /// A Prometheus scrape endpoint at `/metrics` (Option A: no Collector
    /// required).
    #[cfg(feature = "prometheus")]
    pub fn prometheus() -> Self {
        let mut config = Self::empty(Primary::Prometheus);
        config.prometheus = Some(Default::default());
        config
    }

    /// Periodic OTLP export over gRPC to `OTEL_EXPORTER_OTLP_ENDPOINT` or
    /// `http://localhost:4317` (Option B).
    #[cfg(feature = "otlp")]
    pub fn otlp() -> Self {
        let mut config = Self::empty(Primary::Otlp);
        config.otlp.push(OtlpExporter::grpc());
        config
    }

    /// For [`prometheus`](Self::prometheus) configs, the scrape path
    /// (`/metrics`); for [`otlp`](Self::otlp) configs, the Collector URL
    /// (`http://otel-collector:4317`).
    ///
    /// # Panics
    ///
    /// Panics on a config created with [`new`](Self::new), where the target
    /// is ambiguous; configure the exporter directly instead.
    #[allow(unused_mut, unreachable_code, unused_variables)]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        match self.primary {
            #[cfg(feature = "prometheus")]
            Primary::Prometheus => {
                let config = self.prometheus.take().unwrap_or_default();
                self.prometheus = Some(config.path(endpoint));
            }
            #[cfg(feature = "otlp")]
            Primary::Otlp => {
                let first = self.otlp.remove(0);
                self.otlp.insert(0, first.endpoint(endpoint));
            }
            Primary::None => {
                panic!("MetricsConfig::endpoint needs a config created with prometheus() or otlp()")
            }
        }
        self
    }

    /// Adds (or replaces) the Prometheus endpoint.
    #[cfg(feature = "prometheus")]
    pub fn with_prometheus(
        mut self,
        config: kerniq_observability_prometheus::PrometheusConfig,
    ) -> Self {
        self.prometheus = Some(config);
        self
    }

    /// Serves the Prometheus endpoint on its own listener (see
    /// [`PrometheusConfig::listen`](kerniq_observability_prometheus::PrometheusConfig::listen)).
    #[cfg(feature = "prometheus")]
    pub fn prometheus_listener(mut self, address: std::net::SocketAddr) -> Self {
        let config = self.prometheus.take().unwrap_or_default();
        self.prometheus = Some(config.listen(address));
        self
    }

    /// Adds an OTLP exporter (several may be configured).
    #[cfg(feature = "otlp")]
    pub fn with_otlp(mut self, exporter: OtlpExporter) -> Self {
        self.otlp.push(exporter);
        self
    }

    /// Replaces the first OTLP exporter's settings.
    #[cfg(feature = "otlp")]
    pub fn otlp_exporter(mut self, exporter: OtlpExporter) -> Self {
        if self.otlp.is_empty() {
            self.otlp.push(exporter);
        } else {
            self.otlp[0] = exporter;
        }
        self
    }

    /// How often OTLP exporters push. Default 60 s (`OTEL_METRIC_EXPORT_INTERVAL`
    /// convention). Prometheus is pull-based and ignores this.
    pub fn export_interval(mut self, interval: Duration) -> Self {
        self.export_interval = interval;
        self
    }

    /// Exports deltas instead of cumulative totals over OTLP (Datadog and
    /// some other backends prefer delta).
    pub fn delta_temporality(mut self, delta: bool) -> Self {
        self.delta_temporality = delta;
        self
    }

    /// Maximum attribute sets per instrument. Measurements beyond it are
    /// folded into one overflow series (`otel.metric.overflow=true`). The
    /// SDK default is 2000.
    pub fn cardinality_limit(mut self, limit: usize) -> Self {
        self.cardinality_limit = Some(limit);
        self
    }

    /// Records the `http.server.*` metrics. Default on.
    pub fn http(mut self, enabled: bool) -> Self {
        self.http = enabled;
        self
    }

    /// Records Tokio runtime and process metrics (`kerniq.runtime.*`,
    /// `process.*`). Default off. Measurements are taken only when metrics
    /// are collected (scrape or export); no background task is added.
    #[cfg(feature = "runtime")]
    pub fn runtime(mut self, enabled: bool) -> Self {
        self.runtime = enabled;
        self
    }

    /// Adds a push exporter (any `PushMetricExporter`: an SDK exporter, an
    /// in-memory exporter for tests, ...) driven by a periodic reader at
    /// [`export_interval`](Self::export_interval).
    pub fn exporter<E>(mut self, exporter: E) -> Self
    where
        E: opentelemetry_sdk::metrics::exporter::PushMetricExporter,
    {
        self.readers.push(Box::new(move |builder, interval| {
            builder.with_reader(
                opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
                    .with_interval(interval)
                    .build(),
            )
        }));
        self
    }
}

#[cfg(feature = "metrics")]
impl Default for MetricsConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Traces
// ---------------------------------------------------------------------------

/// Tracing configuration.
#[cfg(feature = "tracing")]
#[must_use]
pub struct TracesConfig {
    #[cfg(feature = "otlp")]
    pub(crate) otlp: Vec<OtlpExporter>,
    pub(crate) sampling: Sampling,
    pub(crate) parent_based: bool,
    pub(crate) batch: BatchConfig,
    pub(crate) filter: Option<String>,
    pub(crate) activate_context: bool,
    pub(crate) processors: Vec<SpanProcessorHook>,
}

#[cfg(feature = "tracing")]
pub(crate) type SpanProcessorHook = Box<
    dyn FnOnce(
            opentelemetry_sdk::trace::TracerProviderBuilder,
        ) -> opentelemetry_sdk::trace::TracerProviderBuilder
        + Send,
>;

#[cfg(feature = "tracing")]
impl fmt::Debug for TracesConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("TracesConfig");
        #[cfg(feature = "otlp")]
        d.field("otlp", &self.otlp);
        d.field("sampling", &self.sampling)
            .field("parent_based", &self.parent_based)
            .field("batch", &self.batch)
            .field("filter", &self.filter)
            .field("custom_processors", &self.processors.len())
            .finish()
    }
}

#[cfg(feature = "tracing")]
impl TracesConfig {
    /// Tracing with no exporter yet (add one with
    /// [`with_otlp`](Self::with_otlp) or [`processor`](Self::processor)).
    /// Spans still carry trace ids for log correlation.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "otlp")]
            otlp: Vec::new(),
            sampling: Sampling::AlwaysOn,
            parent_based: true,
            batch: BatchConfig::TRACES,
            filter: None,
            activate_context: false,
            processors: Vec::new(),
        }
    }

    /// Batched OTLP export over gRPC.
    #[cfg(feature = "otlp")]
    pub fn otlp() -> Self {
        Self::new().with_otlp(OtlpExporter::grpc())
    }

    /// Sets the first OTLP exporter's endpoint.
    #[cfg(feature = "otlp")]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let first = if self.otlp.is_empty() {
            OtlpExporter::grpc()
        } else {
            self.otlp.remove(0)
        };
        self.otlp.insert(0, first.endpoint(endpoint));
        self
    }

    /// Adds an OTLP exporter.
    #[cfg(feature = "otlp")]
    pub fn with_otlp(mut self, exporter: OtlpExporter) -> Self {
        self.otlp.push(exporter);
        self
    }

    /// Replaces the first OTLP exporter's settings.
    #[cfg(feature = "otlp")]
    pub fn otlp_exporter(mut self, exporter: OtlpExporter) -> Self {
        if self.otlp.is_empty() {
            self.otlp.push(exporter);
        } else {
            self.otlp[0] = exporter;
        }
        self
    }

    /// Records `ratio` (0.0..=1.0) of traces.
    ///
    /// # Panics
    ///
    /// Panics if `ratio` is not within `0.0..=1.0`.
    pub fn sampling_ratio(mut self, ratio: f64) -> Self {
        assert!(
            (0.0..=1.0).contains(&ratio),
            "sampling ratio must be within 0.0..=1.0"
        );
        self.sampling = Sampling::Ratio(ratio);
        self
    }

    /// Sets the sampling strategy.
    pub fn sampling(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    /// Follow the caller's sampling decision when a sampled/unsampled parent
    /// arrives in `traceparent` (default `true`, the OpenTelemetry default).
    pub fn parent_based(mut self, parent_based: bool) -> Self {
        self.parent_based = parent_based;
        self
    }

    /// Queue and batch limits for every exporter of this signal.
    pub fn batch(mut self, batch: BatchConfig) -> Self {
        self.batch = batch;
        self
    }

    /// Which `tracing` spans become OpenTelemetry spans, as an `EnvFilter`
    /// directive. Default `info` (the HTTP request span is `INFO`).
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Makes the OpenTelemetry `Context` of a `tracing` span current
    /// (`opentelemetry::Context::current()`) whenever the span is entered.
    ///
    /// Always on when logs are exported (OTLP or a log processor): the
    /// OpenTelemetry log bridge reads trace ids from the current context.
    ///
    /// Otherwise off by default: it costs a thread-local context push and pop on every
    /// span enter and exit, which for a streamed response is every frame
    /// (measured at ~10% of streaming throughput, see
    /// `docs/observability/performance.md`). Kerniq's own propagation reads
    /// the context from the span and does not need it. Turn it on when
    /// libraries in the request path call the OpenTelemetry API directly.
    pub fn activate_otel_context(mut self, enabled: bool) -> Self {
        self.activate_context = enabled;
        self
    }

    /// Adds an OpenTelemetry `SpanProcessor` (for any SDK-supported
    /// exporter, or an in-memory exporter in tests).
    pub fn processor<P>(mut self, processor: P) -> Self
    where
        P: opentelemetry_sdk::trace::SpanProcessor + 'static,
    {
        self.processors.push(Box::new(move |builder| {
            builder.with_span_processor(processor)
        }));
        self
    }
}

#[cfg(feature = "tracing")]
impl Default for TracesConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

/// Console output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleFormat {
    /// One JSON object per line, with `trace_id`/`span_id` when inside a
    /// traced span. For production and log shippers.
    Json,
    /// Multi-line, human-readable. For development.
    Pretty,
    /// Single-line, human-readable.
    Compact,
}

/// Where console logs go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleTarget {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Logging configuration: console output and/or OTLP export, through one
/// `tracing` pipeline.
#[cfg(feature = "logs")]
#[must_use]
pub struct LogsConfig {
    pub(crate) console: Option<(ConsoleFormat, ConsoleTarget)>,
    pub(crate) filter: Option<String>,
    pub(crate) non_blocking: Option<usize>,
    #[cfg(feature = "otlp")]
    pub(crate) otlp: Vec<OtlpExporter>,
    #[cfg(feature = "otlp")]
    pub(crate) otlp_filter: Option<String>,
    pub(crate) batch: BatchConfig,
    pub(crate) processors: Vec<LogProcessorHook>,
}

#[cfg(feature = "logs")]
pub(crate) type LogProcessorHook = Box<
    dyn FnOnce(
            opentelemetry_sdk::logs::LoggerProviderBuilder,
        ) -> opentelemetry_sdk::logs::LoggerProviderBuilder
        + Send,
>;

#[cfg(feature = "logs")]
impl fmt::Debug for LogsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("LogsConfig");
        d.field("console", &self.console)
            .field("filter", &self.filter)
            .field("non_blocking", &self.non_blocking);
        #[cfg(feature = "otlp")]
        d.field("otlp", &self.otlp)
            .field("otlp_filter", &self.otlp_filter);
        d.field("batch", &self.batch)
            .field("custom_processors", &self.processors.len())
            .finish()
    }
}

#[cfg(feature = "logs")]
impl LogsConfig {
    /// Default buffered lines for the non-blocking console writer.
    pub const DEFAULT_CONSOLE_BUFFER: usize = 16_384;

    fn console(format: ConsoleFormat) -> Self {
        Self {
            console: Some((format, ConsoleTarget::Stdout)),
            filter: None,
            non_blocking: Some(Self::DEFAULT_CONSOLE_BUFFER),
            #[cfg(feature = "otlp")]
            otlp: Vec::new(),
            #[cfg(feature = "otlp")]
            otlp_filter: None,
            batch: BatchConfig::LOGS,
            processors: Vec::new(),
        }
    }

    /// JSON lines on stdout.
    pub fn json() -> Self {
        Self::console(ConsoleFormat::Json)
    }

    /// Human-readable multi-line output on stdout.
    pub fn pretty() -> Self {
        Self::console(ConsoleFormat::Pretty)
    }

    /// Human-readable single-line output on stdout.
    pub fn compact() -> Self {
        Self::console(ConsoleFormat::Compact)
    }

    /// OTLP export only (no console output; add one with
    /// [`with_console`](Self::with_console)).
    #[cfg(feature = "otlp")]
    pub fn otlp() -> Self {
        let mut config = Self::console(ConsoleFormat::Json);
        config.console = None;
        config.otlp.push(OtlpExporter::grpc());
        config
    }

    /// Adds or replaces console output.
    pub fn with_console(mut self, format: ConsoleFormat) -> Self {
        let target = self.console.map_or(ConsoleTarget::Stdout, |(_, t)| t);
        self.console = Some((format, target));
        self
    }

    /// Removes console output (keeps OTLP export and processors).
    pub fn without_console(mut self) -> Self {
        self.console = None;
        self
    }

    /// Writes console logs to stderr instead of stdout.
    pub fn stderr(mut self) -> Self {
        if let Some((format, _)) = self.console {
            self.console = Some((format, ConsoleTarget::Stderr));
        }
        self
    }

    /// The log filter as `EnvFilter` directives (`info`,
    /// `info,my_crate=debug,hyper=warn`). Default: `RUST_LOG`, else `info`.
    pub fn filter(mut self, directives: impl Into<String>) -> Self {
        self.filter = Some(directives.into());
        self
    }

    /// Console writes go through a bounded in-memory buffer drained by a
    /// dedicated thread, so a slow terminal or log pipe never blocks a
    /// request. When the buffer is full, lines are dropped and counted.
    /// `None` writes synchronously. Default `Some(16384)` lines.
    pub fn non_blocking(mut self, buffered_lines: Option<usize>) -> Self {
        self.non_blocking = buffered_lines;
        self
    }

    /// Adds an OTLP log exporter.
    #[cfg(feature = "otlp")]
    pub fn with_otlp(mut self, exporter: OtlpExporter) -> Self {
        self.otlp.push(exporter);
        self
    }

    /// Sets the first OTLP exporter's endpoint.
    #[cfg(feature = "otlp")]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let first = if self.otlp.is_empty() {
            OtlpExporter::grpc()
        } else {
            self.otlp.remove(0)
        };
        self.otlp.insert(0, first.endpoint(endpoint));
        self
    }

    /// A separate filter for OTLP export (for example `warn` to ship only
    /// warnings while printing `info` locally). Defaults to
    /// [`filter`](Self::filter).
    #[cfg(feature = "otlp")]
    pub fn otlp_filter(mut self, directives: impl Into<String>) -> Self {
        self.otlp_filter = Some(directives.into());
        self
    }

    /// Queue and batch limits for OTLP log export.
    pub fn batch(mut self, batch: BatchConfig) -> Self {
        self.batch = batch;
        self
    }

    /// Adds an OpenTelemetry `LogProcessor`; `tracing` events are bridged
    /// to it like to OTLP.
    pub fn processor<P>(mut self, processor: P) -> Self
    where
        P: opentelemetry_sdk::logs::LogProcessor + 'static,
    {
        self.processors.push(Box::new(move |builder| {
            builder.with_log_processor(processor)
        }));
        self
    }

    pub(crate) fn has_bridge(&self) -> bool {
        #[cfg(feature = "otlp")]
        if !self.otlp.is_empty() {
            return true;
        }
        !self.processors.is_empty()
    }
}
