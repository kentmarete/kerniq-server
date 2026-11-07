//! Request timeouts.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::response::{IntoResponse, Response};
use http::Request;
use kerniq_core::Error;
use kerniq_routing::Middleware;
use pin_project_lite::pin_project;
use tokio::time::Sleep;
use tower::{Layer, Service};

/// Fails requests that take longer than a fixed duration with
/// `408 REQUEST_TIMEOUT` (Kerniq JSON envelope).
///
/// The timer covers everything *inside* this middleware: inner middleware,
/// extractors (including reading the body) and the handler. When it fires,
/// the handler's future is dropped, which cancels the work at its next
/// `.await`.
///
/// ```
/// use kerniq_middleware::Timeout;
///
/// let timeout = Timeout::seconds(30);
/// ```
///
/// Implemented as a single Tower service whose response future holds the
/// timer inline (no extra boxing), rendering the timeout as a Kerniq error.
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    duration: Duration,
}

impl Timeout {
    /// A timeout of `duration`.
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }

    /// A timeout of `secs` seconds.
    pub fn seconds(secs: u64) -> Self {
        Self::new(Duration::from_secs(secs))
    }

    /// A timeout of `millis` milliseconds.
    pub fn millis(millis: u64) -> Self {
        Self::new(Duration::from_millis(millis))
    }

    /// The configured duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Converts into a Tower layer.
    pub fn into_layer(self) -> TimeoutLayer {
        TimeoutLayer {
            duration: self.duration,
        }
    }
}

impl Middleware for Timeout {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

/// Tower layer for [`Timeout`].
#[derive(Debug, Clone, Copy)]
pub struct TimeoutLayer {
    duration: Duration,
}

impl<S> Layer<S> for TimeoutLayer {
    type Service = TimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TimeoutService {
            inner,
            duration: self.duration,
        }
    }
}

/// Service produced by [`TimeoutLayer`].
#[derive(Debug, Clone)]
pub struct TimeoutService<S> {
    inner: S,
    duration: Duration,
}

impl<S, B> Service<Request<B>> for TimeoutService<S>
where
    S: Service<Request<B>, Response = Response>,
{
    type Response = Response;
    type Error = S::Error;
    type Future = TimeoutFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        TimeoutFuture {
            inner: self.inner.call(request),
            sleep: tokio::time::sleep(self.duration),
        }
    }
}

pin_project! {
    /// Response future for [`TimeoutService`].
    pub struct TimeoutFuture<F> {
        #[pin]
        inner: F,
        #[pin]
        sleep: Sleep,
    }
}

impl<F, E> Future for TimeoutFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = Result<Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(result) = this.inner.poll(cx) {
            return Poll::Ready(result);
        }
        match this.sleep.poll(cx) {
            Poll::Ready(()) => Poll::Ready(Ok(Error::timeout(
                "The request did not complete within the configured time limit",
            )
            .into_response())),
            Poll::Pending => Poll::Pending,
        }
    }
}
