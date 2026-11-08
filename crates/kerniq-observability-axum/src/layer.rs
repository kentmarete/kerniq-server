//! The Tower layer.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, ready};
use std::time::Instant;

use axum::extract::MatchedPath;
use bytes::Buf;
use http::{Method, Request, Response, StatusCode};
use kerniq_routing::Middleware;
use kerniq_telemetry::attributes::{http_method, protocol_version};
use opentelemetry::{StringValue, Value};
use pin_project_lite::pin_project;
use tower::{Layer, Service};

#[cfg(feature = "metrics")]
use crate::HttpMetrics;
use crate::body::{Completion, ObservedBody, Outcome};

#[cfg(feature = "tracing")]
type Propagator = Arc<dyn opentelemetry::propagation::TextMapPropagator + Send + Sync>;

/// Configuration for HTTP instrumentation. Convert it with
/// [`into_layer`](Self::into_layer), or pass it to `App::middleware`.
#[derive(Clone)]
#[must_use]
pub struct HttpInstrumentation {
    #[cfg(feature = "metrics")]
    metrics: Option<HttpMetrics>,
    #[cfg(feature = "tracing")]
    tracing: bool,
    #[cfg(feature = "tracing")]
    propagator: Option<Propagator>,
    exclude: Vec<String>,
    scheme: &'static str,
    request_id_header: http::HeaderName,
}

impl fmt::Debug for HttpInstrumentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("HttpInstrumentation");
        #[cfg(feature = "metrics")]
        d.field("metrics", &self.metrics.is_some());
        #[cfg(feature = "tracing")]
        d.field("tracing", &self.tracing)
            .field("propagator", &self.propagator.is_some());
        d.field("exclude", &self.exclude)
            .field("scheme", &self.scheme)
            .finish()
    }
}

impl Default for HttpInstrumentation {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpInstrumentation {
    /// Tracing on (when the `tracing` feature is enabled), metrics off until
    /// a meter is supplied, no propagator, no exclusions.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "metrics")]
            metrics: None,
            #[cfg(feature = "tracing")]
            tracing: true,
            #[cfg(feature = "tracing")]
            propagator: None,
            exclude: Vec::new(),
            scheme: "http",
            request_id_header: http::HeaderName::from_static("x-request-id"),
        }
    }

    /// Records HTTP metrics with instruments created on `meter`.
    #[cfg(feature = "metrics")]
    pub fn meter(self, meter: &opentelemetry::metrics::Meter) -> Self {
        self.metrics(HttpMetrics::new(meter))
    }

    /// Records HTTP metrics with existing instruments.
    #[cfg(feature = "metrics")]
    pub fn metrics(mut self, metrics: HttpMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Enables or disables the per-request span.
    #[cfg(feature = "tracing")]
    pub fn tracing(mut self, enabled: bool) -> Self {
        self.tracing = enabled;
        self
    }

    /// Extracts the parent context from request headers with `propagator`
    /// (typically W3C TraceContext + Baggage).
    #[cfg(feature = "tracing")]
    pub fn propagator<P>(mut self, propagator: P) -> Self
    where
        P: opentelemetry::propagation::TextMapPropagator + Send + Sync + 'static,
    {
        self.propagator = Some(Arc::new(propagator));
        self
    }

    /// Like [`propagator`](Self::propagator), from a shared instance.
    #[cfg(feature = "tracing")]
    pub fn shared_propagator(mut self, propagator: Propagator) -> Self {
        self.propagator = Some(propagator);
        self
    }

    /// Excludes a route template (`/health`, `/users/{id}`) — or, for
    /// requests that match no route, an exact path — from metrics and
    /// tracing.
    pub fn exclude(mut self, route: impl Into<String>) -> Self {
        self.exclude.push(route.into());
        self
    }

    /// The `url.scheme` reported when the request URI has none (HTTP/1).
    /// Set to `"https"` when TLS terminates in front of the application.
    /// Default `"http"`.
    pub fn scheme(mut self, scheme: &'static str) -> Self {
        self.scheme = scheme;
        self
    }

    /// The header whose value is recorded as `kerniq.request_id` on the
    /// request span. Default `x-request-id` (set by `RequestId`).
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid header name.
    pub fn request_id_header(mut self, name: &str) -> Self {
        self.request_id_header =
            http::HeaderName::try_from(name).expect("invalid request id header name");
        self
    }

    /// Converts into a Tower layer.
    pub fn into_layer(self) -> HttpLayer {
        HttpLayer {
            config: Arc::new(Config {
                #[cfg(feature = "metrics")]
                metrics: self.metrics.map(Arc::new),
                #[cfg(feature = "tracing")]
                tracing: self.tracing,
                #[cfg(feature = "tracing")]
                propagator: self.propagator,
                exclude: self.exclude.into_boxed_slice(),
                scheme: self.scheme,
                request_id_header: self.request_id_header,
            }),
        }
    }
}

impl Middleware for HttpInstrumentation {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

struct Config {
    #[cfg(feature = "metrics")]
    metrics: Option<Arc<HttpMetrics>>,
    #[cfg(feature = "tracing")]
    tracing: bool,
    #[cfg(feature = "tracing")]
    propagator: Option<Propagator>,
    exclude: Box<[String]>,
    scheme: &'static str,
    request_id_header: http::HeaderName,
}

impl Config {
    fn excluded(&self, route: &str) -> bool {
        self.exclude.iter().any(|e| e == route)
    }
}

/// Tower layer produced by [`HttpInstrumentation::into_layer`].
#[derive(Clone)]
pub struct HttpLayer {
    config: Arc<Config>,
}

impl fmt::Debug for HttpLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpLayer").finish_non_exhaustive()
    }
}

impl<S> Layer<S> for HttpLayer {
    type Service = HttpService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        HttpService {
            inner,
            config: self.config.clone(),
            route: Arc::new(OnceLock::new()),
        }
    }
}

/// The route template of the route this service instance wraps, converted
/// once into an OpenTelemetry value.
struct RouteEntry {
    template: Arc<str>,
    value: Value,
    excluded: bool,
}

/// Service produced by [`HttpLayer`].
///
/// Axum's `Router::layer` wraps each route separately, so each instance
/// usually sees a single route template. The template is converted into an
/// attribute value on the first request and reused afterwards; requests
/// with a different template (e.g. the shared 404 fallback) fall back to a
/// per-request conversion.
#[derive(Clone)]
pub struct HttpService<S> {
    inner: S,
    config: Arc<Config>,
    route: Arc<OnceLock<RouteEntry>>,
}

impl<S> fmt::Debug for HttpService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpService").finish_non_exhaustive()
    }
}

impl<S> HttpService<S> {
    /// Returns `(route value, excluded)`.
    fn route<B>(&self, request: &Request<B>) -> (Option<Value>, bool) {
        let Some(matched) = request.extensions().get::<MatchedPath>() else {
            return (None, self.config.excluded(request.uri().path()));
        };
        let template = matched.as_str();
        if let Some(entry) = self.route.get()
            && &*entry.template == template
        {
            return (Some(entry.value.clone()), entry.excluded);
        }
        let shared: Arc<str> = Arc::from(template);
        let entry = RouteEntry {
            value: Value::String(StringValue::from(shared.clone())),
            template: shared,
            excluded: self.config.excluded(template),
        };
        let result = (Some(entry.value.clone()), entry.excluded);
        let _ = self.route.set(entry);
        result
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for HttpService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: http_body::Body,
    ResBody::Data: Buf,
{
    type Response = Response<ObservedBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let start = Instant::now();
        let (route, excluded) = self.route(&request);
        if excluded {
            return ResponseFuture {
                inner: self.inner.call(request),
                completion: None,
                head: false,
                request_id_header: None,
            };
        }

        let config = &*self.config;
        let method = http_method(request.method());
        let scheme = request.uri().scheme_str().map_or(config.scheme, |s| {
            if s == "https" { "https" } else { "http" }
        });
        let protocol = protocol_version(request.version());
        let request_size = request
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());

        #[cfg(feature = "metrics")]
        if let Some(metrics) = &config.metrics {
            use kerniq_telemetry::semconv::http as conv;
            use opentelemetry::KeyValue;
            metrics.active.add(
                1,
                &[
                    KeyValue::new(conv::REQUEST_METHOD, method),
                    KeyValue::new(conv::URL_SCHEME, scheme),
                ],
            );
        }

        #[cfg(feature = "tracing")]
        let span = if config.tracing {
            make_span(config, &request, method, scheme, protocol, route.as_ref())
        } else {
            tracing::Span::none()
        };

        let completion = Completion {
            #[cfg(feature = "metrics")]
            metrics: config.metrics.clone(),
            method,
            scheme,
            protocol,
            route,
            request_size,
            status: None,
            start,
            first_byte: false,
            bytes: 0,
            done: false,
            #[cfg(feature = "tracing")]
            span,
        };

        let head = request.method() == Method::HEAD;
        let request_id_header = (!request.headers().contains_key(&config.request_id_header))
            .then(|| config.request_id_header.clone());
        #[cfg(feature = "tracing")]
        let inner = {
            let _entered = completion.span().enter();
            self.inner.call(request)
        };
        #[cfg(not(feature = "tracing"))]
        let inner = self.inner.call(request);

        ResponseFuture {
            inner,
            completion: Some(completion),
            head,
            request_id_header,
        }
    }
}

#[cfg(feature = "tracing")]
fn make_span<B>(
    config: &Config,
    request: &Request<B>,
    method: &'static str,
    scheme: &'static str,
    protocol: Option<&'static str>,
    route: Option<&Value>,
) -> tracing::Span {
    use tracing::field::Empty;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let span = tracing::info_span!(
        target: "kerniq::http",
        "HTTP request",
        otel.name = Empty,
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = method,
        http.route = Empty,
        http.response.status_code = Empty,
        http.response.body.size = Empty,
        url.path = request.uri().path(),
        url.scheme = scheme,
        network.protocol.version = Empty,
        error.type = Empty,
        user_agent.original = Empty,
        kerniq.request_id = Empty,
    );
    if span.is_disabled() {
        return span;
    }
    match route {
        Some(route) => {
            let route = route.as_str();
            span.record("http.route", &*route);
            span.record("otel.name", format!("{method} {route}"));
        }
        None => {
            span.record("otel.name", method);
        }
    }
    if let Some(protocol) = protocol {
        span.record("network.protocol.version", protocol);
    }
    let headers = request.headers();
    if let Some(ua) = headers
        .get(http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
    {
        span.record("user_agent.original", ua);
    }
    if let Some(id) = headers
        .get(&config.request_id_header)
        .and_then(|v| v.to_str().ok())
    {
        span.record("kerniq.request_id", id);
    }
    if let Some(propagator) = &config.propagator {
        let cx = crate::propagation::extract_context(propagator.as_ref(), headers);
        use opentelemetry::trace::TraceContextExt;
        if cx.span().span_context().is_valid() {
            let _ = span.set_parent(cx);
        }
    }
    span
}

pin_project! {
    /// Response future for [`HttpService`].
    ///
    /// If it is dropped before completing (the client disconnected while the
    /// handler was running, or a timeout cancelled it), the request is
    /// recorded with `error.type = kerniq.aborted`.
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        completion: Option<Completion>,
        head: bool,
        // Set when the request carried no id: read it from the response.
        request_id_header: Option<http::HeaderName>,
    }
}

impl<F> fmt::Debug for ResponseFuture<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseFuture").finish_non_exhaustive()
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    ResBody: http_body::Body,
    ResBody::Data: Buf,
{
    type Output = Result<Response<ObservedBody<ResBody>>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let result = {
            #[cfg(feature = "tracing")]
            let _entered = this.completion.as_ref().map(|c| c.span().enter());
            ready!(this.inner.poll(cx))
        };
        let Some(mut completion) = this.completion.take() else {
            return Poll::Ready(result.map(|r| r.map(|b| ObservedBody::new(b, None))));
        };
        match result {
            Ok(response) => {
                let status = response.status();
                completion.status = Some(status.as_u16());
                #[cfg(feature = "tracing")]
                {
                    let span = completion.span();
                    span.record("http.response.status_code", status.as_u16());
                    // An inner `RequestId` middleware generates ids after this
                    // layer saw the request; it echoes them on the response.
                    if !span.is_disabled()
                        && let Some(id) = this
                            .request_id_header
                            .as_ref()
                            .and_then(|h| response.headers().get(h))
                            .and_then(|v| v.to_str().ok())
                    {
                        span.record("kerniq.request_id", id);
                    }
                }
                // Hyper never polls these bodies, so they end here.
                if *this.head
                    || status == StatusCode::NO_CONTENT
                    || status == StatusCode::NOT_MODIFIED
                    || status.is_informational()
                    || response.body().is_end_stream()
                {
                    completion.finish(Outcome::Finished);
                }
                Poll::Ready(Ok(
                    response.map(|body| ObservedBody::new(body, Some(completion)))
                ))
            }
            Err(err) => {
                completion.finish(Outcome::ServiceError);
                Poll::Ready(Err(err))
            }
        }
    }
}
