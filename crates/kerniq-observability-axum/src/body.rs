//! The response body wrapper and per-request completion record.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Buf;
use http_body::{Body, Frame, SizeHint};
use kerniq_telemetry::attributes::status_error_type;
use kerniq_telemetry::semconv::{http as conv, outcome};
use opentelemetry::{KeyValue, Value};
use pin_project_lite::pin_project;

#[cfg(feature = "metrics")]
use crate::HttpMetrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Finished,
    Aborted,
    BodyError,
    ServiceError,
}

/// Everything needed to record one request, carried from the response
/// future into the response body. Recording happens exactly once: when the
/// body ends, fails, or — if neither happened — when this value is dropped
/// (client disconnect or cancellation).
#[derive(Debug)]
pub(crate) struct Completion {
    #[cfg(feature = "metrics")]
    pub(crate) metrics: Option<std::sync::Arc<HttpMetrics>>,
    pub(crate) method: &'static str,
    pub(crate) scheme: &'static str,
    pub(crate) protocol: Option<&'static str>,
    pub(crate) route: Option<Value>,
    pub(crate) request_size: Option<u64>,
    pub(crate) status: Option<u16>,
    pub(crate) start: Instant,
    pub(crate) first_byte: bool,
    pub(crate) bytes: u64,
    pub(crate) done: bool,
    #[cfg(feature = "tracing")]
    pub(crate) span: tracing::Span,
}

/// Placeholder used to fill the fixed-size attribute buffer; never recorded.
fn blank() -> KeyValue {
    KeyValue::new("", "")
}

impl Completion {
    #[cfg(feature = "tracing")]
    pub(crate) fn span(&self) -> &tracing::Span {
        &self.span
    }

    /// Writes the request attributes into `buf` and returns how many were
    /// written. A stack buffer keeps attribute construction allocation-free.
    #[cfg(feature = "metrics")]
    fn attributes(&self, buf: &mut [KeyValue; 6], error_type: Option<&'static str>) -> usize {
        let mut n = 0;
        let mut push = |kv: KeyValue| {
            buf[n] = kv;
            n += 1;
        };
        push(KeyValue::new(conv::REQUEST_METHOD, self.method));
        push(KeyValue::new(conv::URL_SCHEME, self.scheme));
        if let Some(protocol) = self.protocol {
            push(KeyValue::new(conv::NETWORK_PROTOCOL_VERSION, protocol));
        }
        if let Some(route) = &self.route {
            push(KeyValue::new(conv::ROUTE, route.clone()));
        }
        if let Some(status) = self.status {
            push(KeyValue::new(conv::RESPONSE_STATUS_CODE, i64::from(status)));
        }
        if let Some(error_type) = error_type {
            push(KeyValue::new(conv::ERROR_TYPE, error_type));
        }
        n
    }

    pub(crate) fn on_first_byte(&mut self) {
        if self.first_byte {
            return;
        }
        self.first_byte = true;
        #[cfg(feature = "metrics")]
        if let Some(metrics) = &self.metrics {
            let mut buf: [KeyValue; 6] = std::array::from_fn(|_| blank());
            let n = self.attributes(&mut buf, None);
            metrics
                .time_to_first_byte
                .record(self.start.elapsed().as_secs_f64(), &buf[..n]);
        }
    }

    pub(crate) fn finish(&mut self, how: Outcome) {
        if self.done {
            return;
        }
        self.done = true;
        let error_type = match how {
            Outcome::Finished => self.status.and_then(status_error_type),
            Outcome::Aborted => Some(outcome::ABORTED),
            Outcome::BodyError => Some(outcome::BODY_ERROR),
            Outcome::ServiceError => Some(outcome::SERVICE_ERROR),
        };

        #[cfg(feature = "metrics")]
        if let Some(metrics) = &self.metrics {
            let elapsed = self.start.elapsed().as_secs_f64();
            let mut buf: [KeyValue; 6] = std::array::from_fn(|_| blank());
            let n = self.attributes(&mut buf, error_type);
            let attrs = &buf[..n];
            metrics.duration.record(elapsed, attrs);
            if self.status.is_some() {
                metrics.response_size.record(self.bytes, attrs);
            }
            if let Some(size) = self.request_size {
                metrics.request_size.record(size, attrs);
            }
            metrics.active.add(
                -1,
                &[
                    KeyValue::new(conv::REQUEST_METHOD, self.method),
                    KeyValue::new(conv::URL_SCHEME, self.scheme),
                ],
            );
        }

        #[cfg(feature = "tracing")]
        if !self.span.is_disabled() {
            if let Some(error_type) = error_type {
                self.span.record(conv::ERROR_TYPE, error_type);
                // Server spans: 5xx and failures are errors, 4xx are not.
                self.span.record("otel.status_code", "error");
            }
            self.span.record("http.response.body.size", self.bytes);
        }
        #[cfg(not(any(feature = "metrics", feature = "tracing")))]
        let _ = error_type;
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.finish(Outcome::Aborted);
    }
}

pin_project! {
    /// A response body that reports its progress to the instrumentation.
    ///
    /// Frames are forwarded unchanged and immediately; nothing is buffered.
    /// The only per-frame work is adding the frame length to a counter (and,
    /// for the first data frame, recording time to first byte).
    #[derive(Debug)]
    pub struct ObservedBody<B> {
        #[pin]
        inner: B,
        completion: Option<Completion>,
    }
}

impl<B: Body> ObservedBody<B> {
    pub(crate) fn new(inner: B, completion: Option<Completion>) -> Self {
        Self { inner, completion }
    }

    /// The wrapped body.
    pub fn into_inner(self) -> B {
        self.inner
    }
}

impl<B> Body for ObservedBody<B>
where
    B: Body,
    B::Data: Buf,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();
        let Some(completion) = this.completion.as_mut() else {
            return this.inner.poll_frame(cx);
        };
        // Code that produces the stream (e.g. a token generator) runs inside
        // the request span. The span is moved out for the duration of the
        // poll (a pointer move; cloning would touch the span registry on
        // every frame) and restored before any recording.
        #[cfg(feature = "tracing")]
        let result = {
            let span = std::mem::replace(&mut completion.span, tracing::Span::none());
            let result = {
                let _entered = span.enter();
                this.inner.as_mut().poll_frame(cx)
            };
            completion.span = span;
            result
        };
        #[cfg(not(feature = "tracing"))]
        let result = this.inner.as_mut().poll_frame(cx);
        match &result {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let len = data.remaining();
                    if len > 0 {
                        completion.on_first_byte();
                        completion.bytes += len as u64;
                    }
                }
                if this.inner.is_end_stream() {
                    completion.finish(Outcome::Finished);
                }
            }
            Poll::Ready(Some(Err(_))) => completion.finish(Outcome::BodyError),
            Poll::Ready(None) => completion.finish(Outcome::Finished),
            Poll::Pending => {}
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
