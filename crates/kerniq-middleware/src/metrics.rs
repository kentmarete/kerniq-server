//! Prometheus-compatible HTTP metrics.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt::{self, Write as _};
use std::future::Future;
use std::hash::BuildHasher;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll, ready};
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::response::Response;
use http::Method;
use kerniq_routing::Middleware;
use pin_project_lite::pin_project;
use tower::{Layer, Service};

/// Upper bounds (seconds) of the latency histogram buckets.
const BUCKETS: [f64; 12] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];
const SHARDS: usize = 16;

/// Records request counts, latencies and in-flight requests, and renders
/// them in the Prometheus text exposition format.
///
/// | Metric | Type | Labels |
/// |---|---|---|
/// | `http_requests_total` | counter | `method`, `route`, `status` |
/// | `http_request_duration_seconds` | histogram | `method`, `route` |
/// | `http_requests_in_flight` | gauge | — |
///
/// `route` is the matched route *template* (`/users/{id}`, not
/// `/users/42`), so label cardinality is bounded by the number of routes.
/// Unmatched requests use `route="<unmatched>"`.
///
/// ```
/// use kerniq_middleware::Metrics;
///
/// let metrics = Metrics::new();
/// let registry = metrics.registry();
/// // app.middleware(metrics)
/// //    .get("/metrics", move || async move { registry.render() })
/// ```
///
/// The hot path takes a sharded read lock and increments atomics; the write
/// lock is only taken the first time a (method, route, status) combination
/// is seen. Exporting to other systems (OpenTelemetry, StatsD) can be built
/// the same way as a custom Tower layer.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    registry: MetricsRegistry,
}

impl Metrics {
    /// Creates the middleware with a fresh registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle to the registry (cheap to clone) for rendering.
    pub fn registry(&self) -> MetricsRegistry {
        self.registry.clone()
    }

    /// Converts into a Tower layer.
    pub fn into_layer(self) -> MetricsLayer {
        MetricsLayer {
            registry: self.registry,
        }
    }
}

impl Middleware for Metrics {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

#[derive(Debug, Default)]
struct Series {
    /// Responses by status code.
    statuses: RwLock<HashMap<u16, AtomicU64>>,
    buckets: [AtomicU64; BUCKETS.len()],
    count: AtomicU64,
    sum_nanos: AtomicU64,
}

type RouteMap = HashMap<Arc<str>, Vec<(Method, Arc<Series>)>>;

#[derive(Debug, Default)]
struct Inner {
    hasher: RandomState,
    /// route template -> per-method series, sharded by route.
    shards: [RwLock<RouteMap>; SHARDS],
    in_flight: AtomicI64,
}

/// Shared metric storage. Clone it to render metrics from a handler.
#[derive(Clone, Default)]
pub struct MetricsRegistry {
    inner: Arc<Inner>,
}

impl fmt::Debug for MetricsRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricsRegistry").finish_non_exhaustive()
    }
}

impl MetricsRegistry {
    fn series(&self, method: &Method, route: &str) -> Arc<Series> {
        let shard = &self.inner.shards[(self.inner.hasher.hash_one(route) as usize) % SHARDS];
        {
            let map = shard.read().unwrap_or_else(|p| p.into_inner());
            if let Some(series) = map
                .get(route)
                .and_then(|methods| methods.iter().find(|(m, _)| m == method))
            {
                return series.1.clone();
            }
        }
        let mut map = shard.write().unwrap_or_else(|p| p.into_inner());
        let methods = map.entry(Arc::from(route)).or_default();
        if let Some((_, series)) = methods.iter().find(|(m, _)| m == method) {
            return series.clone();
        }
        let series = Arc::new(Series::default());
        methods.push((method.clone(), series.clone()));
        series
    }

    fn record(&self, method: &Method, route: &str, status: u16, start: Instant) {
        let elapsed = start.elapsed();
        let series = self.series(method, route);
        let secs = elapsed.as_secs_f64();
        for (bucket, bound) in series.buckets.iter().zip(BUCKETS) {
            if secs <= bound {
                bucket.fetch_add(1, Ordering::Relaxed);
            }
        }
        series.count.fetch_add(1, Ordering::Relaxed);
        series.sum_nanos.fetch_add(
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        {
            let statuses = series.statuses.read().unwrap_or_else(|p| p.into_inner());
            if let Some(counter) = statuses.get(&status) {
                counter.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        series
            .statuses
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .entry(status)
            .or_default()
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Renders all metrics in the Prometheus text exposition format
    /// (version 0.0.4).
    pub fn render(&self) -> String {
        let mut all: Vec<(Arc<str>, Method, Arc<Series>)> = Vec::new();
        for shard in &self.inner.shards {
            let map = shard.read().unwrap_or_else(|p| p.into_inner());
            for (route, methods) in map.iter() {
                for (method, series) in methods {
                    all.push((route.clone(), method.clone(), series.clone()));
                }
            }
        }
        all.sort_by(|a, b| (&a.0, a.1.as_str()).cmp(&(&b.0, b.1.as_str())));

        let mut out = String::with_capacity(256 + all.len() * 512);
        out.push_str(
            "# HELP http_requests_total Total HTTP requests by method, route and status.\n",
        );
        out.push_str("# TYPE http_requests_total counter\n");
        for (route, method, series) in &all {
            let statuses = series.statuses.read().unwrap_or_else(|p| p.into_inner());
            let mut codes: Vec<_> = statuses.iter().collect();
            codes.sort_by_key(|(code, _)| **code);
            for (code, count) in codes {
                let _ = writeln!(
                    out,
                    "http_requests_total{{method=\"{}\",route=\"{}\",status=\"{}\"}} {}",
                    method,
                    escape(route),
                    code,
                    count.load(Ordering::Relaxed)
                );
            }
        }
        out.push_str("# HELP http_request_duration_seconds HTTP request latency.\n");
        out.push_str("# TYPE http_request_duration_seconds histogram\n");
        for (route, method, series) in &all {
            let labels = format!("method=\"{method}\",route=\"{}\"", escape(route));
            for (bucket, bound) in series.buckets.iter().zip(BUCKETS) {
                let _ = writeln!(
                    out,
                    "http_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {}",
                    bucket.load(Ordering::Relaxed)
                );
            }
            let count = series.count.load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {count}"
            );
            let _ = writeln!(
                out,
                "http_request_duration_seconds_sum{{{labels}}} {}",
                series.sum_nanos.load(Ordering::Relaxed) as f64 / 1e9
            );
            let _ = writeln!(
                out,
                "http_request_duration_seconds_count{{{labels}}} {count}"
            );
        }
        out.push_str("# HELP http_requests_in_flight HTTP requests currently being processed.\n");
        out.push_str("# TYPE http_requests_in_flight gauge\n");
        let _ = writeln!(
            out,
            "http_requests_in_flight {}",
            self.inner.in_flight.load(Ordering::Relaxed)
        );
        out
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Tower layer for [`Metrics`].
#[derive(Debug, Clone)]
pub struct MetricsLayer {
    registry: MetricsRegistry,
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            registry: self.registry.clone(),
        }
    }
}

/// Service produced by [`MetricsLayer`].
#[derive(Debug, Clone)]
pub struct MetricsService<S> {
    inner: S,
    registry: MetricsRegistry,
}

impl<S> Service<Request> for MetricsService<S>
where
    S: Service<Request, Response = Response>,
{
    type Response = Response;
    type Error = S::Error;
    type Future = MetricsFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let method = request.method().clone();
        // `MatchedPath` wraps an `Arc<str>`: cloning it does not allocate.
        let route = request.extensions().get::<MatchedPath>().cloned();
        self.registry
            .inner
            .in_flight
            .fetch_add(1, Ordering::Relaxed);
        MetricsFuture {
            inner: self.inner.call(request),
            guard: InFlight {
                registry: self.registry.clone(),
                method,
                route,
                start: Instant::now(),
            },
        }
    }
}

/// Decrements the in-flight gauge even if the future is dropped
/// (client disconnect, timeout).
struct InFlight {
    registry: MetricsRegistry,
    method: Method,
    route: Option<MatchedPath>,
    start: Instant,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.registry
            .inner
            .in_flight
            .fetch_sub(1, Ordering::Relaxed);
    }
}

pin_project! {
    /// Response future for [`MetricsService`].
    pub struct MetricsFuture<F> {
        #[pin]
        inner: F,
        guard: InFlight,
    }
}

impl<F, E> Future for MetricsFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = Result<Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let result = ready!(this.inner.poll(cx));
        if let Ok(response) = &result {
            let guard = &*this.guard;
            guard.registry.record(
                &guard.method,
                guard
                    .route
                    .as_ref()
                    .map_or("<unmatched>", MatchedPath::as_str),
                response.status().as_u16(),
                guard.start,
            );
        }
        Poll::Ready(result)
    }
}

impl fmt::Debug for InFlight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InFlight").finish_non_exhaustive()
    }
}
