//! Export-pipeline health counters.
//!
//! The SDK's batch processors drop items when their queue is full rather
//! than block, but do not expose how many. Kerniq wraps each pipeline's
//! processor and exporter to count items *offered* to the pipeline, items
//! *exported*, and items whose export *failed*. While the application runs,
//! `offered − exported − failed` is the number queued plus the number
//! dropped; queued is bounded by the queue capacity, so a gap that keeps
//! growing past it means the backend cannot keep up. After
//! [`Observability::shutdown`](crate::Observability::shutdown) has drained
//! the queues, the gap is exactly the number dropped.

use std::fmt;
#[allow(unused_imports)]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counters for one signal's export pipelines.
#[derive(Debug, Default)]
pub(crate) struct SignalCounters {
    pub(crate) offered: AtomicU64,
    pub(crate) exported: AtomicU64,
    pub(crate) failed: AtomicU64,
    pub(crate) export_calls: AtomicU64,
    pub(crate) export_failures: AtomicU64,
}

impl SignalCounters {
    fn snapshot(&self) -> SignalStats {
        // Read `offered` last so the derived gap is never negative.
        let exported = self.exported.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        let export_calls = self.export_calls.load(Ordering::Relaxed);
        let export_failures = self.export_failures.load(Ordering::Relaxed);
        let offered = self.offered.load(Ordering::Relaxed);
        SignalStats {
            offered,
            exported,
            failed,
            unexported: offered.saturating_sub(exported + failed),
            export_calls,
            export_failures,
        }
    }

    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    pub(crate) fn record_export(&self, items: u64, ok: bool) {
        self.export_calls.fetch_add(1, Ordering::Relaxed);
        if ok {
            self.exported.fetch_add(items, Ordering::Relaxed);
        } else {
            self.failed.fetch_add(items, Ordering::Relaxed);
            self.export_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Shared counters for every signal.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub(crate) traces: SignalCounters,
    pub(crate) logs: SignalCounters,
    pub(crate) metrics: SignalCounters,
    /// Console log lines dropped by the non-blocking writer.
    #[cfg(feature = "logs")]
    pub(crate) console_dropped: std::sync::OnceLock<tracing_appender::non_blocking::ErrorCounter>,
}

impl Counters {
    pub(crate) fn snapshot(&self) -> TelemetryStats {
        TelemetryStats {
            traces: self.traces.snapshot(),
            logs: self.logs.snapshot(),
            metrics: self.metrics.snapshot(),
            #[cfg(feature = "logs")]
            console_lines_dropped: self
                .console_dropped
                .get()
                .map_or(0, |c| c.dropped_lines() as u64),
            #[cfg(not(feature = "logs"))]
            console_lines_dropped: 0,
        }
    }
}

/// Health of one signal's export pipelines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SignalStats {
    /// Items handed to the pipelines (spans ended while sampled, log
    /// records emitted). For metrics: data points in export calls.
    pub offered: u64,
    /// Items the exporter accepted.
    pub exported: u64,
    /// Items in export calls that failed (after retries).
    pub failed: u64,
    /// `offered − exported − failed`: queued plus dropped. After shutdown,
    /// the number dropped. Always 0 for metrics, which aggregate in memory
    /// instead of queuing.
    pub unexported: u64,
    /// Export calls made.
    pub export_calls: u64,
    /// Export calls that failed.
    pub export_failures: u64,
}

/// A point-in-time view of every export pipeline. See the module docs for
/// how to read it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TelemetryStats {
    /// Span pipelines.
    pub traces: SignalStats,
    /// Log pipelines (OTLP and custom processors).
    pub logs: SignalStats,
    /// Push-based metric exporters (OTLP). Prometheus scrapes are not
    /// counted.
    pub metrics: SignalStats,
    /// Console log lines dropped because the non-blocking buffer was full.
    pub console_lines_dropped: u64,
}

impl fmt::Display for TelemetryStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, s) in [
            ("traces", self.traces),
            ("logs", self.logs),
            ("metrics", self.metrics),
        ] {
            writeln!(
                f,
                "{name}: offered={} exported={} failed={} unexported={} export_failures={}",
                s.offered, s.exported, s.failed, s.unexported, s.export_failures
            )?;
        }
        write!(f, "console lines dropped={}", self.console_lines_dropped)
    }
}

// ---------------------------------------------------------------------------
// Wrappers
// ---------------------------------------------------------------------------

#[cfg(all(feature = "tracing", feature = "otlp"))]
pub(crate) mod traces {
    use super::*;
    use opentelemetry::Context;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::{Span, SpanData, SpanExporter, SpanProcessor};
    use std::time::Duration;

    /// Counts spans offered to the wrapped processor.
    #[derive(Debug)]
    pub(crate) struct CountingProcessor<P> {
        pub(crate) inner: P,
        pub(crate) counters: Arc<Counters>,
    }

    impl<P: SpanProcessor> SpanProcessor for CountingProcessor<P> {
        fn on_start(&self, span: &mut Span, cx: &Context) {
            self.inner.on_start(span, cx);
        }
        fn on_end(&self, span: SpanData) {
            if span.span_context.is_sampled() {
                self.counters.traces.offered.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.on_end(span);
        }
        fn force_flush(&self) -> OTelSdkResult {
            self.inner.force_flush()
        }
        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }
        fn set_resource(&mut self, resource: &Resource) {
            self.inner.set_resource(resource);
        }
    }

    /// Counts exported / failed spans.
    #[derive(Debug)]
    pub(crate) struct CountingExporter<E> {
        pub(crate) inner: E,
        pub(crate) counters: Arc<Counters>,
    }

    impl<E: SpanExporter> SpanExporter for CountingExporter<E> {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            let n = batch.len() as u64;
            let result = self.inner.export(batch).await;
            self.counters.traces.record_export(n, result.is_ok());
            result
        }
        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }
        fn force_flush(&self) -> OTelSdkResult {
            self.inner.force_flush()
        }
        fn set_resource(&mut self, resource: &Resource) {
            self.inner.set_resource(resource);
        }
    }
}

#[cfg(all(feature = "logs", feature = "otlp"))]
pub(crate) mod logs {
    use super::*;
    use opentelemetry::InstrumentationScope;
    use opentelemetry::logs::Severity;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::logs::{LogBatch, LogExporter, LogProcessor, SdkLogRecord};
    use std::time::Duration;

    #[derive(Debug)]
    pub(crate) struct CountingProcessor<P> {
        pub(crate) inner: P,
        pub(crate) counters: Arc<Counters>,
    }

    impl<P: LogProcessor> LogProcessor for CountingProcessor<P> {
        fn emit(&self, data: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
            self.counters.logs.offered.fetch_add(1, Ordering::Relaxed);
            self.inner.emit(data, instrumentation);
        }
        fn force_flush(&self) -> OTelSdkResult {
            self.inner.force_flush()
        }
        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }
        fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
            self.inner.event_enabled(level, target, name)
        }
        fn set_resource(&mut self, resource: &Resource) {
            self.inner.set_resource(resource);
        }
    }

    #[derive(Debug)]
    pub(crate) struct CountingExporter<E> {
        pub(crate) inner: E,
        pub(crate) counters: Arc<Counters>,
    }

    impl<E: LogExporter> LogExporter for CountingExporter<E> {
        async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
            let n = batch.iter().count() as u64;
            let result = self.inner.export(batch).await;
            self.counters.logs.record_export(n, result.is_ok());
            result
        }
        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }
        fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
            self.inner.event_enabled(level, target, name)
        }
        fn set_resource(&mut self, resource: &Resource) {
            self.inner.set_resource(resource);
        }
    }
}

#[cfg(feature = "metrics")]
pub(crate) mod metrics {
    use super::*;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::metrics::Temporality;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
    use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
    use std::time::Duration;

    #[derive(Debug)]
    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    pub(crate) struct CountingExporter<E> {
        pub(crate) inner: E,
        pub(crate) counters: Arc<Counters>,
    }

    #[cfg_attr(not(feature = "otlp"), allow(dead_code))]
    fn points(metrics: &ResourceMetrics) -> u64 {
        let mut n = 0u64;
        for scope in metrics.scope_metrics() {
            for metric in scope.metrics() {
                macro_rules! count {
                    ($d:expr) => {
                        match $d {
                            MetricData::Sum(s) => s.data_points().count(),
                            MetricData::Gauge(g) => g.data_points().count(),
                            MetricData::Histogram(h) => h.data_points().count(),
                            MetricData::ExponentialHistogram(h) => h.data_points().count(),
                        }
                    };
                }
                n += match metric.data() {
                    AggregatedMetrics::F64(d) => count!(d),
                    AggregatedMetrics::U64(d) => count!(d),
                    AggregatedMetrics::I64(d) => count!(d),
                } as u64;
            }
        }
        n
    }

    impl<E: PushMetricExporter> PushMetricExporter for CountingExporter<E> {
        async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
            let n = points(metrics);
            self.counters
                .metrics
                .offered
                .fetch_add(n, Ordering::Relaxed);
            let result = self.inner.export(metrics).await;
            self.counters.metrics.record_export(n, result.is_ok());
            result
        }
        fn force_flush(&self) -> OTelSdkResult {
            self.inner.force_flush()
        }
        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }
        fn temporality(&self) -> Temporality {
            self.inner.temporality()
        }
    }

    /// Publishes the counters as `kerniq.telemetry.*` observable counters.
    pub(crate) fn register(meter: &opentelemetry::metrics::Meter, counters: &Arc<Counters>) {
        use kerniq_telemetry::semconv::kerniq as k;
        use opentelemetry::KeyValue;

        let c = counters.clone();
        meter
            .u64_observable_counter(k::TELEMETRY_ITEMS)
            .with_unit("{item}")
            .with_description("Telemetry items by signal and outcome (offered, exported, failed).")
            .with_callback(move |observer| {
                let s = c.snapshot();
                for (signal, stats) in [("traces", s.traces), ("logs", s.logs)] {
                    for (outcome, value) in [
                        ("offered", stats.offered),
                        ("exported", stats.exported),
                        ("failed", stats.failed),
                    ] {
                        observer.observe(
                            value,
                            &[
                                KeyValue::new(k::SIGNAL, signal),
                                KeyValue::new(k::OUTCOME, outcome),
                            ],
                        );
                    }
                }
                observer.observe(
                    s.console_lines_dropped,
                    &[
                        KeyValue::new(k::SIGNAL, "console_logs"),
                        KeyValue::new(k::OUTCOME, "dropped"),
                    ],
                );
            })
            .build();
        let c = counters.clone();
        meter
            .u64_observable_counter(k::TELEMETRY_EXPORTS)
            .with_unit("{export}")
            .with_description("Export calls by signal and outcome (success, failure).")
            .with_callback(move |observer| {
                let s = c.snapshot();
                for (signal, stats) in [
                    ("traces", s.traces),
                    ("logs", s.logs),
                    ("metrics", s.metrics),
                ] {
                    observer.observe(
                        stats.export_calls - stats.export_failures,
                        &[
                            KeyValue::new(k::SIGNAL, signal),
                            KeyValue::new(k::OUTCOME, "success"),
                        ],
                    );
                    observer.observe(
                        stats.export_failures,
                        &[
                            KeyValue::new(k::SIGNAL, signal),
                            KeyValue::new(k::OUTCOME, "failure"),
                        ],
                    );
                }
            })
            .build();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexported_is_never_negative() {
        let c = SignalCounters::default();
        c.offered.fetch_add(10, Ordering::Relaxed);
        c.record_export(6, true);
        c.record_export(2, false);
        let s = c.snapshot();
        assert_eq!(s.unexported, 2);
        assert_eq!(s.export_calls, 2);
        assert_eq!(s.export_failures, 1);
    }
}
