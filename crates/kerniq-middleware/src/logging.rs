//! Structured request logging.

use std::time::Duration;

use axum::extract::MatchedPath;
use http::{Request, Response};
use kerniq_routing::Middleware;
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::trace::{MakeSpan, OnFailure, OnResponse, TraceLayer};
use tracing::{Level, Span};

/// Logs every request as a `tracing` span with one event per response.
///
/// Span `request` fields:
///
/// | Field | Source |
/// |---|---|
/// | `method` | request method |
/// | `path` | request path (no query string, which may contain secrets) |
/// | `route` | matched route template, e.g. `/users/{id}` |
/// | `request_id` | `x-request-id` header (see [`RequestId`](crate::RequestId)) |
/// | `trace_id` | W3C `traceparent` header, for correlating with distributed traces |
///
/// Response event fields: `status`, `latency_ms`. 5xx responses are logged at
/// `ERROR` level, everything else at the configured level (default `INFO`).
///
/// The span is a regular `tracing` span, so an OpenTelemetry subscriber
/// (`tracing-opentelemetry`) exports it without extra integration.
///
/// ```
/// use kerniq_middleware::Logger;
/// use tracing::Level;
///
/// let logger = Logger::new().level(Level::DEBUG);
/// ```
///
/// Requires the `tracing` feature.
#[derive(Debug, Clone, Copy)]
pub struct Logger {
    level: Level,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    /// Logs at `INFO`.
    pub fn new() -> Self {
        Self { level: Level::INFO }
    }

    /// Sets the level of the request span and response events.
    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }
}

impl Middleware for Logger {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(
            TraceLayer::new_for_http()
                .make_span_with(KerniqSpan { level: self.level })
                .on_request(())
                .on_response(KerniqOnResponse { level: self.level })
                .on_failure(KerniqOnFailure),
        )
    }
}

macro_rules! dyn_span {
    ($level:expr, $($rest:tt)*) => {
        match $level {
            Level::ERROR => tracing::error_span!($($rest)*),
            Level::WARN => tracing::warn_span!($($rest)*),
            Level::INFO => tracing::info_span!($($rest)*),
            Level::DEBUG => tracing::debug_span!($($rest)*),
            Level::TRACE => tracing::trace_span!($($rest)*),
        }
    };
}

macro_rules! dyn_event {
    ($level:expr, $($rest:tt)*) => {
        match $level {
            Level::ERROR => tracing::error!($($rest)*),
            Level::WARN => tracing::warn!($($rest)*),
            Level::INFO => tracing::info!($($rest)*),
            Level::DEBUG => tracing::debug!($($rest)*),
            Level::TRACE => tracing::trace!($($rest)*),
        }
    };
}

/// Creates the `request` span.
#[derive(Debug, Clone, Copy)]
pub struct KerniqSpan {
    level: Level,
}

impl<B> MakeSpan<B> for KerniqSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let headers = request.headers();
        let request_id = headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let trace_id = headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .and_then(trace_id_from_traceparent)
            .unwrap_or("");
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str)
            .unwrap_or("");
        dyn_span!(
            self.level,
            "request",
            method = %request.method(),
            path = request.uri().path(),
            route,
            request_id,
            trace_id,
        )
    }
}

/// Extracts the trace id from a W3C `traceparent` header
/// (`00-<32 hex trace id>-<16 hex parent id>-<2 hex flags>`).
fn trace_id_from_traceparent(value: &str) -> Option<&str> {
    let mut parts = value.split('-');
    let _version = parts.next()?;
    let trace_id = parts.next()?;
    (trace_id.len() == 32 && trace_id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(trace_id)
}

/// Emits the response event.
#[derive(Debug, Clone, Copy)]
pub struct KerniqOnResponse {
    level: Level,
}

impl<B> OnResponse<B> for KerniqOnResponse {
    fn on_response(self, response: &Response<B>, latency: Duration, span: &Span) {
        let status = response.status().as_u16();
        let latency_ms = latency.as_secs_f64() * 1000.0;
        let level = if response.status().is_server_error() {
            Level::ERROR
        } else {
            self.level
        };
        dyn_event!(level, parent: span, status, latency_ms, "response");
    }
}

/// Failures (5xx) are already logged by [`KerniqOnResponse`] at `ERROR`; this
/// only reports failures that happen while streaming the body.
#[derive(Debug, Clone, Copy)]
pub struct KerniqOnFailure;

impl OnFailure<ServerErrorsFailureClass> for KerniqOnFailure {
    fn on_failure(&mut self, class: ServerErrorsFailureClass, latency: Duration, span: &Span) {
        if let ServerErrorsFailureClass::Error(error) = class {
            tracing::error!(
                parent: span,
                error,
                latency_ms = latency.as_secs_f64() * 1000.0,
                "response body failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::trace_id_from_traceparent;

    #[test]
    fn parses_traceparent() {
        assert_eq!(
            trace_id_from_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("4bf92f3577b34da6a3ce929d0e0e4736")
        );
        assert_eq!(trace_id_from_traceparent("garbage"), None);
        assert_eq!(
            trace_id_from_traceparent("00-xyz-00f067aa0ba902b7-01"),
            None
        );
    }
}
