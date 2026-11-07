//! Request body and concurrency limits.

use axum::extract::DefaultBodyLimit;
use kerniq_routing::Middleware;
use tower::limit::GlobalConcurrencyLimitLayer;

/// Limits the request body size accepted by body extractors (`Json`,
/// `String`, `Bytes`, `Form`, ...). Larger bodies are rejected with
/// `413 PAYLOAD_TOO_LARGE` before they are fully read.
///
/// This is Axum's `DefaultBodyLimit`, which costs nothing per request: it
/// only records the limit for extractors to enforce. Handlers that consume
/// the raw `Body` stream are responsible for their own limits.
#[derive(Debug, Clone, Copy)]
pub struct BodyLimit {
    bytes: usize,
}

impl BodyLimit {
    /// Limit bodies to `bytes`.
    pub fn new(bytes: usize) -> Self {
        Self { bytes }
    }

    /// Limit bodies to `kib` KiB.
    pub fn kib(kib: usize) -> Self {
        Self::new(kib * 1024)
    }

    /// Limit bodies to `mib` MiB.
    pub fn mib(mib: usize) -> Self {
        Self::new(mib * 1024 * 1024)
    }

    /// The configured limit in bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Middleware for BodyLimit {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(DefaultBodyLimit::max(self.bytes))
    }
}

/// Limits the number of requests processed concurrently across *all*
/// routes the middleware is attached to. Requests over the limit wait for a
/// slot (combine with [`Timeout`](crate::Timeout) to bound the wait).
///
/// Uses Tower's `GlobalConcurrencyLimitLayer`, whose semaphore is shared by
/// every route (a plain `ConcurrencyLimitLayer` would create one semaphore
/// per route, because Axum applies layers route by route).
#[derive(Debug, Clone)]
pub struct ConcurrencyLimit {
    max: usize,
}

impl ConcurrencyLimit {
    /// Allow at most `max` concurrent requests.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero.
    pub fn new(max: usize) -> Self {
        assert!(max > 0, "ConcurrencyLimit must allow at least one request");
        Self { max }
    }
}

impl Middleware for ConcurrencyLimit {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(GlobalConcurrencyLimitLayer::new(self.max))
    }
}
