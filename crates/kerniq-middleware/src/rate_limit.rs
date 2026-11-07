//! In-process rate limiting.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::future::Future;
use std::hash::{BuildHasher, Hash, Hasher};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::extract::ConnectInfo;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use http::HeaderValue;
use http::header::RETRY_AFTER;
use kerniq_core::Error;
use kerniq_routing::Middleware;
use pin_project_lite::pin_project;
use tower::{Layer, Service};

const SHARDS: usize = 16;
/// Buckets are swept when a shard grows past this many keys.
const SWEEP_THRESHOLD: usize = 4096;

type KeyFn = Arc<dyn Fn(&Request) -> Option<String> + Send + Sync>;

#[derive(Clone)]
enum KeyBy {
    Global,
    PeerIp,
    Header(http::HeaderName),
    Custom(KeyFn),
}

/// Token-bucket rate limiting, keyed globally, per client IP, per header
/// value or by a custom function.
///
/// Requests over the limit receive `429 TOO_MANY_REQUESTS` with a
/// `Retry-After` header. State is kept in memory in 16 independently locked
/// shards (no global lock); idle buckets are swept as shards grow. For limits
/// shared across several server instances, use a gateway or a store-backed
/// middleware instead.
///
/// ```
/// use std::time::Duration;
/// use kerniq_middleware::RateLimit;
///
/// // 100 requests per second per client IP, bursts of up to 200.
/// let per_ip = RateLimit::per_second(100).burst(200).by_ip();
///
/// // 1000 requests per minute per API key.
/// let per_key = RateLimit::per_minute(1000).by_header("x-api-key");
/// ```
///
/// `by_ip` uses the TCP peer address (`ConnectInfo`, enabled by default in
/// `ServerConfig`). Behind a reverse proxy, every client shares the proxy's
/// address: key on a header your proxy sets with [`by_key`](Self::by_key)
/// instead. Requests without a key share one bucket.
#[derive(Clone)]
pub struct RateLimit {
    rate: f64,
    burst: f64,
    key: KeyBy,
}

impl fmt::Debug for RateLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = match &self.key {
            KeyBy::Global => "global".to_owned(),
            KeyBy::PeerIp => "peer-ip".to_owned(),
            KeyBy::Header(h) => format!("header:{h}"),
            KeyBy::Custom(_) => "custom".to_owned(),
        };
        f.debug_struct("RateLimit")
            .field("rate_per_sec", &self.rate)
            .field("burst", &self.burst)
            .field("key", &key)
            .finish()
    }
}

impl RateLimit {
    /// Allows `requests` per `period`, with a burst equal to `requests`.
    ///
    /// # Panics
    ///
    /// Panics if `requests` is zero or `period` is zero.
    pub fn new(requests: u32, period: Duration) -> Self {
        assert!(requests > 0, "rate limit must allow at least one request");
        assert!(!period.is_zero(), "rate limit period must be non-zero");
        Self {
            rate: f64::from(requests) / period.as_secs_f64(),
            burst: f64::from(requests),
            key: KeyBy::Global,
        }
    }

    /// `requests` per second.
    pub fn per_second(requests: u32) -> Self {
        Self::new(requests, Duration::from_secs(1))
    }

    /// `requests` per minute.
    pub fn per_minute(requests: u32) -> Self {
        Self::new(requests, Duration::from_secs(60))
    }

    /// Sets the bucket capacity (maximum burst).
    ///
    /// # Panics
    ///
    /// Panics if `burst` is zero.
    pub fn burst(mut self, burst: u32) -> Self {
        assert!(burst > 0, "burst must be at least one");
        self.burst = f64::from(burst);
        self
    }

    /// One bucket for all requests (the default).
    pub fn global(mut self) -> Self {
        self.key = KeyBy::Global;
        self
    }

    /// One bucket per TCP peer IP address.
    pub fn by_ip(mut self) -> Self {
        self.key = KeyBy::PeerIp;
        self
    }

    /// One bucket per value of `header`.
    ///
    /// # Panics
    ///
    /// Panics if `header` is not a valid header name.
    pub fn by_header(mut self, header: &str) -> Self {
        self.key = KeyBy::Header(http::HeaderName::try_from(header).expect("invalid header name"));
        self
    }

    /// One bucket per key returned by `f` (`None` = shared bucket).
    ///
    /// ```
    /// use kerniq_middleware::RateLimit;
    ///
    /// // Key on the client address set by a trusted reverse proxy.
    /// let limit = RateLimit::per_second(50).by_key(|req| {
    ///     req.headers()
    ///         .get("x-real-ip")
    ///         .and_then(|v| v.to_str().ok())
    ///         .map(str::to_owned)
    /// });
    /// ```
    pub fn by_key<F>(mut self, f: F) -> Self
    where
        F: Fn(&Request) -> Option<String> + Send + Sync + 'static,
    {
        self.key = KeyBy::Custom(Arc::new(f));
        self
    }

    /// Converts into a Tower layer. All services created from the layer
    /// share the same buckets.
    pub fn into_layer(self) -> RateLimitLayer {
        RateLimitLayer {
            state: Arc::new(State {
                rate: self.rate,
                burst: self.burst,
                key: self.key,
                hasher: RandomState::new(),
                shards: std::array::from_fn(|_| Mutex::new(HashMap::new())),
            }),
        }
    }
}

impl Middleware for RateLimit {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

struct State {
    rate: f64,
    burst: f64,
    key: KeyBy,
    hasher: RandomState,
    shards: [Mutex<HashMap<u64, Bucket>>; SHARDS],
}

impl State {
    fn key_hash(&self, request: &Request) -> u64 {
        let mut hasher = self.hasher.build_hasher();
        match &self.key {
            KeyBy::Global => return 0,
            KeyBy::PeerIp => request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip())
                .hash(&mut hasher),
            KeyBy::Header(name) => request
                .headers()
                .get(name)
                .map(HeaderValue::as_bytes)
                .hash(&mut hasher),
            KeyBy::Custom(f) => f(request).hash(&mut hasher),
        }
        hasher.finish()
    }

    /// Takes a token; on failure returns how long until one is available.
    fn acquire(&self, key: u64, now: Instant) -> Result<(), Duration> {
        let shard = &self.shards[(key as usize) % SHARDS];
        let mut buckets = shard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if buckets.len() > SWEEP_THRESHOLD {
            // A bucket idle long enough to refill completely is equivalent
            // to a fresh one, so it can be dropped.
            let full_after = Duration::from_secs_f64(self.burst / self.rate);
            buckets.retain(|_, b| now.duration_since(b.updated) < full_after);
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: self.burst,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rate).min(self.burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / self.rate))
        }
    }
}

/// Tower layer for [`RateLimit`].
#[derive(Clone)]
pub struct RateLimitLayer {
    state: Arc<State>,
}

impl fmt::Debug for RateLimitLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimitLayer").finish_non_exhaustive()
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            state: self.state.clone(),
        }
    }
}

/// Service produced by [`RateLimitLayer`].
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    state: Arc<State>,
}

impl<S> fmt::Debug for RateLimitService<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimitService").finish_non_exhaustive()
    }
}

impl<S> Service<Request> for RateLimitService<S>
where
    S: Service<Request, Response = Response>,
{
    type Response = Response;
    type Error = S::Error;
    type Future = RateLimitFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let key = self.state.key_hash(&request);
        match self.state.acquire(key, Instant::now()) {
            Ok(()) => RateLimitFuture::Inner {
                future: self.inner.call(request),
            },
            Err(wait) => {
                let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
                let response = Error::too_many_requests("Rate limit exceeded, retry later")
                    .with_header(RETRY_AFTER, HeaderValue::from(secs))
                    .into_response();
                RateLimitFuture::Limited {
                    response: Some(response),
                }
            }
        }
    }
}

pin_project! {
    /// Response future for [`RateLimitService`].
    #[project = RateLimitProj]
    pub enum RateLimitFuture<F> {
        /// Forwarded to the inner service.
        Inner { #[pin] future: F },
        /// Rejected.
        Limited { response: Option<Response> },
    }
}

impl<F, E> Future for RateLimitFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = Result<Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            RateLimitProj::Inner { future } => future.poll(cx),
            RateLimitProj::Limited { response } => {
                Poll::Ready(Ok(response.take().expect("polled after completion")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(rate: u32, burst: u32) -> Arc<State> {
        RateLimit::per_second(rate).burst(burst).into_layer().state
    }

    #[test]
    fn bucket_allows_burst_then_limits() {
        let state = state(10, 3);
        let now = Instant::now();
        assert!(state.acquire(1, now).is_ok());
        assert!(state.acquire(1, now).is_ok());
        assert!(state.acquire(1, now).is_ok());
        let wait = state.acquire(1, now).unwrap_err();
        assert!(wait <= Duration::from_millis(100));
        // Other keys are independent.
        assert!(state.acquire(2, now).is_ok());
        // Tokens refill over time.
        assert!(state.acquire(1, now + Duration::from_millis(150)).is_ok());
    }

    #[test]
    fn idle_buckets_are_swept() {
        let state = state(1000, 1);
        let start = Instant::now();
        for key in 0..(SWEEP_THRESHOLD as u64 * SHARDS as u64 + SHARDS as u64 * 2) {
            let _ = state.acquire(key, start);
        }
        let later = start + Duration::from_secs(1);
        for shard in 0..SHARDS as u64 {
            let _ = state.acquire(shard, later);
        }
        let total: usize = state.shards.iter().map(|s| s.lock().unwrap().len()).sum();
        assert!(
            total <= SHARDS * 2,
            "expected idle buckets to be swept, found {total}"
        );
    }
}
