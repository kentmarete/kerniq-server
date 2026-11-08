//! Conventions for instrumenting AI inference workloads.
//!
//! Kerniq does not run models. These are *application* instruments for
//! services that do: queue wait, time to first token, token throughput,
//! active streams, total duration, cancellations and errors, recorded with
//! aggregated measurements (one set per request, never one event per
//! token).
//!
//! | Metric | Instrument | Unit |
//! |---|---|---|
//! | `inference.requests` | Counter | `{request}` |
//! | `inference.errors` | Counter | `{error}` |
//! | `inference.queue.duration` | Histogram | `s` |
//! | `inference.time_to_first_token` | Histogram | `s` |
//! | `inference.duration` | Histogram | `s` |
//! | `inference.tokens.generated` | Counter | `{token}` |
//! | `inference.tokens.input` | Counter | `{token}` |
//! | `inference.token_throughput` | Histogram | `{token}/s` |
//! | `inference.active_streams` | UpDownCounter | `{stream}` |
//!
//! Attributes: whatever bounded attributes the caller supplies (typically a
//! model *family* through [`BoundedLabel`](crate::BoundedLabel)), plus
//! `inference.outcome` (`success`, `error`, `cancelled`) on `duration` and
//! `requests`, and `error.type` on `errors`. Never pass prompts, generated
//! text, customer ids or free-form model names.
//!
//! These names map onto the OpenTelemetry GenAI conventions as follows, for
//! teams that standardize on them: `inference.time_to_first_token` ≈
//! `gen_ai.server.time_to_first_token`, `inference.duration` ≈
//! `gen_ai.server.request.duration`, `inference.tokens.*` ≈
//! `gen_ai.client.token.usage` by `gen_ai.token.type`.
//!
//! ```
//! use kerniq_telemetry::inference::InferenceMetrics;
//! use kerniq_telemetry::{BoundedLabel, KeyValue};
//! use opentelemetry::metrics::MeterProvider;
//! # let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
//!
//! static MODEL: BoundedLabel = BoundedLabel::new(&["chat-small", "chat-large"], "other");
//!
//! let metrics = InferenceMetrics::new(&provider.meter("inference"));
//!
//! // Per request:
//! let mut request = metrics.start([KeyValue::new("inference.model", MODEL.get("chat-small"))]);
//! request.execution_started();   // left the queue
//! request.tokens_generated(1);   // first token: records time to first token
//! request.tokens_generated(41);  // later tokens only update a local counter
//! request.finish();              // or `fail("timeout")`; dropping = cancelled
//! ```

use std::sync::Arc;
use std::time::Instant;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};

use crate::semconv::buckets;

/// `inference.requests`.
pub const REQUESTS: &str = "inference.requests";
/// `inference.errors`.
pub const ERRORS: &str = "inference.errors";
/// `inference.queue.duration`.
pub const QUEUE_DURATION: &str = "inference.queue.duration";
/// `inference.time_to_first_token`.
pub const TIME_TO_FIRST_TOKEN: &str = "inference.time_to_first_token";
/// `inference.duration`.
pub const DURATION: &str = "inference.duration";
/// `inference.tokens.generated`.
pub const TOKENS_GENERATED: &str = "inference.tokens.generated";
/// `inference.tokens.input`.
pub const TOKENS_INPUT: &str = "inference.tokens.input";
/// `inference.token_throughput`.
pub const TOKEN_THROUGHPUT: &str = "inference.token_throughput";
/// `inference.active_streams`.
pub const ACTIVE_STREAMS: &str = "inference.active_streams";
/// `inference.outcome` attribute.
pub const OUTCOME: &str = "inference.outcome";

/// Time-to-first-token buckets, seconds.
pub const TTFT_BUCKETS: &[f64] = &[
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 10.0, 30.0,
];
/// Throughput buckets, tokens per second.
pub const THROUGHPUT_BUCKETS: &[f64] = &[
    1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0,
];

/// The instruments, created once and shared (cloning is cheap).
#[derive(Debug, Clone)]
pub struct InferenceMetrics {
    requests: Counter<u64>,
    errors: Counter<u64>,
    queue_duration: Histogram<f64>,
    ttft: Histogram<f64>,
    duration: Histogram<f64>,
    tokens_generated: Counter<u64>,
    tokens_input: Counter<u64>,
    throughput: Histogram<f64>,
    active_streams: UpDownCounter<i64>,
}

impl InferenceMetrics {
    /// Creates the instruments on `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self {
            requests: meter
                .u64_counter(REQUESTS)
                .with_unit("{request}")
                .with_description("Inference requests, by outcome.")
                .build(),
            errors: meter
                .u64_counter(ERRORS)
                .with_unit("{error}")
                .with_description("Failed inference requests, by error.type.")
                .build(),
            queue_duration: meter
                .f64_histogram(QUEUE_DURATION)
                .with_unit("s")
                .with_description("Time spent waiting before model execution started.")
                .with_boundaries(buckets::DURATION_SECONDS.to_vec())
                .build(),
            ttft: meter
                .f64_histogram(TIME_TO_FIRST_TOKEN)
                .with_unit("s")
                .with_description("Time from request arrival to the first generated token.")
                .with_boundaries(TTFT_BUCKETS.to_vec())
                .build(),
            duration: meter
                .f64_histogram(DURATION)
                .with_unit("s")
                .with_description("Total inference time, arrival to completion.")
                .with_boundaries(buckets::DURATION_SECONDS.to_vec())
                .build(),
            tokens_generated: meter
                .u64_counter(TOKENS_GENERATED)
                .with_unit("{token}")
                .with_description("Generated (output) tokens.")
                .build(),
            tokens_input: meter
                .u64_counter(TOKENS_INPUT)
                .with_unit("{token}")
                .with_description("Input (prompt) tokens.")
                .build(),
            throughput: meter
                .f64_histogram(TOKEN_THROUGHPUT)
                .with_unit("{token}/s")
                .with_description("Generated tokens per second of generation time, per request.")
                .with_boundaries(THROUGHPUT_BUCKETS.to_vec())
                .build(),
            active_streams: meter
                .i64_up_down_counter(ACTIVE_STREAMS)
                .with_unit("{stream}")
                .with_description("Inference requests currently generating.")
                .build(),
        }
    }

    /// Starts tracking a request that just arrived (queue wait starts now).
    ///
    /// `attributes` must be bounded; they are attached to every measurement
    /// of this request. They are stored once, in an `Arc`.
    pub fn start(&self, attributes: impl Into<Vec<KeyValue>>) -> InferenceRequest {
        InferenceRequest {
            metrics: self.clone(),
            attributes: Arc::from(attributes.into()),
            arrived: Instant::now(),
            execution_started: None,
            first_token: None,
            generated: 0,
            input: 0,
            state: State::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    Done,
}

/// Measurements for one inference request.
///
/// Token counts accumulate locally and are recorded once when the request
/// ends, so per-token cost is an integer add. If the guard is dropped
/// without [`finish`](Self::finish) or [`fail`](Self::fail) — for example
/// because the client disconnected and the response stream was dropped —
/// the request is recorded as `cancelled`.
#[derive(Debug)]
pub struct InferenceRequest {
    metrics: InferenceMetrics,
    attributes: Arc<[KeyValue]>,
    arrived: Instant,
    execution_started: Option<Instant>,
    first_token: Option<Instant>,
    generated: u64,
    input: u64,
    state: State,
}

impl InferenceRequest {
    /// The request left the queue and the model started executing. Records
    /// `inference.queue.duration` and increments `inference.active_streams`.
    /// Idempotent.
    pub fn execution_started(&mut self) {
        if self.execution_started.is_some() {
            return;
        }
        let now = Instant::now();
        self.execution_started = Some(now);
        self.metrics.queue_duration.record(
            now.duration_since(self.arrived).as_secs_f64(),
            &self.attributes,
        );
        self.metrics.active_streams.add(1, &self.attributes);
    }

    /// Records the number of input (prompt) tokens.
    pub fn input_tokens(&mut self, count: u64) {
        self.input = self.input.saturating_add(count);
    }

    /// `count` tokens were generated. The first call records
    /// `inference.time_to_first_token`.
    pub fn tokens_generated(&mut self, count: u64) {
        if count == 0 {
            return;
        }
        if self.first_token.is_none() {
            // A model that produced a token has started executing.
            self.execution_started();
            let now = Instant::now();
            self.first_token = Some(now);
            self.metrics.ttft.record(
                now.duration_since(self.arrived).as_secs_f64(),
                &self.attributes,
            );
        }
        self.generated = self.generated.saturating_add(count);
    }

    /// Tokens generated so far.
    pub fn generated(&self) -> u64 {
        self.generated
    }

    /// The request completed successfully.
    pub fn finish(mut self) {
        self.complete("success", None);
    }

    /// The request failed. `error_type` must be a bounded, static value
    /// (`"timeout"`, `"overloaded"`, `"invalid_request"` ...), never an error
    /// message.
    pub fn fail(mut self, error_type: &'static str) {
        self.complete("error", Some(error_type));
    }

    fn complete(&mut self, outcome: &'static str, error_type: Option<&'static str>) {
        if self.state == State::Done {
            return;
        }
        self.state = State::Done;
        let now = Instant::now();
        let m = &self.metrics;

        if self.execution_started.is_some() {
            m.active_streams.add(-1, &self.attributes);
        }
        if self.generated > 0 {
            m.tokens_generated.add(self.generated, &self.attributes);
            if let Some(first) = self.first_token {
                let seconds = now.duration_since(first).as_secs_f64();
                // Throughput over generation time; a single-token response
                // has no meaningful rate.
                if self.generated > 1 && seconds > 0.0 {
                    m.throughput
                        .record((self.generated - 1) as f64 / seconds, &self.attributes);
                }
            }
        }
        if self.input > 0 {
            m.tokens_input.add(self.input, &self.attributes);
        }

        let mut attrs = Vec::with_capacity(self.attributes.len() + 1);
        attrs.extend_from_slice(&self.attributes);
        attrs.push(KeyValue::new(OUTCOME, outcome));
        m.requests.add(1, &attrs);
        m.duration
            .record(now.duration_since(self.arrived).as_secs_f64(), &attrs);
        if outcome != "success" {
            attrs.pop();
            attrs.push(KeyValue::new(
                crate::semconv::http::ERROR_TYPE,
                error_type.unwrap_or(outcome),
            ));
            m.errors.add(1, &attrs);
        }
    }
}

impl Drop for InferenceRequest {
    fn drop(&mut self) {
        self.complete("cancelled", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    fn setup() -> (SdkMeterProvider, InMemoryMetricExporter, InferenceMetrics) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        let metrics = InferenceMetrics::new(&provider.meter("test"));
        (provider, exporter, metrics)
    }

    fn sum_u64(exporter: &InMemoryMetricExporter, name: &str, outcome: Option<&str>) -> u64 {
        let mut total = 0;
        for rm in exporter.get_finished_metrics().unwrap() {
            for scope in rm.scope_metrics() {
                for metric in scope.metrics().filter(|m| m.name() == name) {
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() {
                        for point in sum.data_points() {
                            let matches = outcome.is_none_or(|o| {
                                point
                                    .attributes()
                                    .any(|kv| kv.key.as_str() == OUTCOME && kv.value.as_str() == o)
                            });
                            if matches {
                                total += point.value();
                            }
                        }
                    }
                }
            }
        }
        total
    }

    #[test]
    fn success_records_aggregates_once() {
        let (provider, exporter, metrics) = setup();
        let mut req = metrics.start([KeyValue::new("inference.model", "small")]);
        req.input_tokens(10);
        for _ in 0..100 {
            req.tokens_generated(1);
        }
        req.finish();
        provider.force_flush().unwrap();
        assert_eq!(sum_u64(&exporter, TOKENS_GENERATED, None), 100);
        assert_eq!(sum_u64(&exporter, TOKENS_INPUT, None), 10);
        assert_eq!(sum_u64(&exporter, REQUESTS, Some("success")), 1);
        assert_eq!(sum_u64(&exporter, ERRORS, None), 0);
    }

    #[test]
    fn drop_is_cancellation() {
        let (provider, exporter, metrics) = setup();
        {
            let mut req = metrics.start(Vec::new());
            req.tokens_generated(3);
        }
        provider.force_flush().unwrap();
        assert_eq!(sum_u64(&exporter, REQUESTS, Some("cancelled")), 1);
        assert_eq!(sum_u64(&exporter, ERRORS, None), 1);
        assert_eq!(sum_u64(&exporter, TOKENS_GENERATED, None), 3);
    }

    #[test]
    fn failure() {
        let (provider, exporter, metrics) = setup();
        metrics.start(Vec::new()).fail("timeout");
        provider.force_flush().unwrap();
        assert_eq!(sum_u64(&exporter, REQUESTS, Some("error")), 1);
        assert_eq!(sum_u64(&exporter, ERRORS, None), 1);
    }
}
