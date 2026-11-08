//! Metric and attribute names.
//!
//! Names under [`http`] follow the stable OpenTelemetry HTTP semantic
//! conventions. Names prefixed with `kerniq.` are Kerniq extensions for
//! behavior the conventions do not cover (streaming responses, exporter
//! health, Tokio runtime state).
//!
//! When exported to Prometheus, dots become underscores and units become
//! suffixes: `http.server.request.duration` (unit `s`) is scraped as
//! `http_server_request_duration_seconds`. [`prometheus_name`] documents the
//! exact mapping used by the dashboards in `observability/grafana`.

/// HTTP server metrics and attributes (OpenTelemetry semantic conventions).
pub mod http {
    /// Histogram, seconds. Time from receiving the request until the
    /// response body finished (or the request was abandoned).
    pub const SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
    /// UpDownCounter. Requests whose response has not finished yet,
    /// including the body of streaming responses.
    pub const SERVER_ACTIVE_REQUESTS: &str = "http.server.active_requests";
    /// Histogram, bytes. Request body size taken from `Content-Length`
    /// (bodies are never read for telemetry).
    pub const SERVER_REQUEST_BODY_SIZE: &str = "http.server.request.body.size";
    /// Histogram, bytes. Response body bytes actually written.
    pub const SERVER_RESPONSE_BODY_SIZE: &str = "http.server.response.body.size";

    /// `http.request.method` — normalized, `_OTHER` for non-standard methods.
    pub const REQUEST_METHOD: &str = "http.request.method";
    /// `http.route` — the matched route template (`/users/{id}`).
    pub const ROUTE: &str = "http.route";
    /// `http.response.status_code`.
    pub const RESPONSE_STATUS_CODE: &str = "http.response.status_code";
    /// `url.scheme`.
    pub const URL_SCHEME: &str = "url.scheme";
    /// `url.path` (spans only, never metrics).
    pub const URL_PATH: &str = "url.path";
    /// `network.protocol.version` (`1.0`, `1.1`, `2`, `3`).
    pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
    /// `error.type` — status code for 5xx, or a Kerniq outcome
    /// (see [`super::outcome`]).
    pub const ERROR_TYPE: &str = "error.type";
    /// `user_agent.original` (spans only).
    pub const USER_AGENT: &str = "user_agent.original";
}

/// Kerniq extensions to the HTTP conventions.
pub mod kerniq {
    /// Histogram, seconds. Time from receiving the request until the first
    /// response body frame was produced. For streaming responses (server-sent
    /// events, token streams) this is the time to first byte; for buffered
    /// responses it is close to the request duration.
    pub const SERVER_TIME_TO_FIRST_BYTE: &str = "kerniq.http.server.time_to_first_byte";
    /// Attribute on spans: the request id from `x-request-id`
    /// (never a metric label).
    pub const REQUEST_ID: &str = "kerniq.request_id";

    /// ObservableCounter. Telemetry items handed to an export pipeline.
    /// Attributes: `kerniq.signal`, `kerniq.outcome`
    /// (`exported`, `failed`, `dropped`).
    pub const TELEMETRY_ITEMS: &str = "kerniq.telemetry.items";
    /// ObservableCounter. Export calls by `kerniq.signal` and
    /// `kerniq.outcome` (`success`, `failure`).
    pub const TELEMETRY_EXPORTS: &str = "kerniq.telemetry.exports";
    /// Attribute: `traces`, `logs` or `metrics`.
    pub const SIGNAL: &str = "kerniq.signal";
    /// Attribute: outcome of an item or export.
    pub const OUTCOME: &str = "kerniq.outcome";
}

/// Runtime and process metrics.
pub mod runtime {
    /// Gauge. Tokio worker threads (measured).
    pub const TOKIO_WORKERS: &str = "kerniq.runtime.tokio.workers";
    /// Gauge. Alive Tokio tasks (measured).
    pub const TOKIO_ALIVE_TASKS: &str = "kerniq.runtime.tokio.alive_tasks";
    /// Gauge. Tasks in the global (injection) queue (measured).
    pub const TOKIO_GLOBAL_QUEUE_DEPTH: &str = "kerniq.runtime.tokio.global_queue.depth";
    /// Counter, seconds. Process CPU time (user + system), OpenTelemetry
    /// process conventions.
    pub const PROCESS_CPU_TIME: &str = "process.cpu.time";
    /// Gauge, bytes. Resident set size.
    pub const PROCESS_MEMORY_USAGE: &str = "process.memory.usage";
    /// Gauge, bytes. Virtual memory size.
    pub const PROCESS_MEMORY_VIRTUAL: &str = "process.memory.virtual";
    /// Gauge. Open file descriptors (Unix only).
    pub const PROCESS_OPEN_FDS: &str = "process.unix.file_descriptor.count";
}

/// Values for `error.type` and outcome attributes. All are `'static` and
/// bounded.
pub mod outcome {
    /// The client went away (or the request was cancelled) before the
    /// response finished.
    pub const ABORTED: &str = "kerniq.aborted";
    /// The response body returned an error while streaming.
    pub const BODY_ERROR: &str = "kerniq.body_error";
    /// The inner service returned an error instead of a response.
    pub const SERVICE_ERROR: &str = "kerniq.service_error";
}

/// Explicit histogram bucket boundaries.
pub mod buckets {
    /// Request duration, seconds: the semantic-convention advisory buckets,
    /// extended to 5 minutes so long-lived streams are not all in `+Inf`.
    pub const DURATION_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0, 30.0,
        60.0, 120.0, 300.0,
    ];
    /// Body sizes, bytes.
    pub const BODY_SIZE_BYTES: &[f64] = &[
        0.0,
        100.0,
        1_000.0,
        10_000.0,
        100_000.0,
        1_000_000.0,
        10_000_000.0,
        100_000_000.0,
    ];
}

/// The Prometheus name `opentelemetry-prometheus` produces for an
/// OpenTelemetry instrument: dots become underscores, the unit is appended
/// (`s` → `_seconds`, `By` → `_bytes`) and counters get `_total`.
///
/// ```
/// use kerniq_telemetry::semconv::{http, prometheus_name};
///
/// assert_eq!(
///     prometheus_name(http::SERVER_REQUEST_DURATION, "s", false),
///     "http_server_request_duration_seconds"
/// );
/// ```
pub fn prometheus_name(name: &str, unit: &str, counter: bool) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let suffix = match unit {
        "s" => "_seconds",
        "ms" => "_milliseconds",
        "By" => "_bytes",
        "1" => "_ratio",
        _ => "",
    };
    if !out.ends_with(suffix) {
        out.push_str(suffix);
    }
    if counter {
        out.push_str("_total");
    }
    out
}
