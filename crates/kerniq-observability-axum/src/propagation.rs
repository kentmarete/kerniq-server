//! W3C trace-context propagation over `http::HeaderMap`.
//!
//! Incoming context is extracted by [`HttpInstrumentation`](crate::HttpInstrumentation).
//! For outgoing calls, inject the current span's context:
//!
//! ```
//! use kerniq_observability_axum::propagation::inject_current_context;
//! use opentelemetry::propagation::TextMapPropagator;
//! # use opentelemetry_sdk::propagation::TraceContextPropagator;
//!
//! # let propagator = TraceContextPropagator::new();
//! let mut headers = http::HeaderMap::new();
//! inject_current_context(&propagator, &mut headers);
//! // send `headers` with the outbound request
//! ```

use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Reads propagation fields from request headers.
#[derive(Debug)]
pub struct HeaderExtractor<'a>(pub &'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

/// Writes propagation fields into request headers. Invalid names or values
/// are skipped.
#[derive(Debug)]
pub struct HeaderInjector<'a>(pub &'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Injects the current `tracing` span's OpenTelemetry context into
/// `headers` (for outbound HTTP calls).
pub fn inject_current_context(propagator: &dyn TextMapPropagator, headers: &mut HeaderMap) {
    let cx = tracing::Span::current().context();
    propagator.inject_context(&cx, &mut HeaderInjector(headers));
}

/// Extracts a remote context from `headers`.
pub fn extract_context(
    propagator: &dyn TextMapPropagator,
    headers: &HeaderMap,
) -> opentelemetry::Context {
    propagator.extract(&HeaderExtractor(headers))
}
