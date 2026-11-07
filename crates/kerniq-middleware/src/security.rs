//! Security-related response headers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use http::{HeaderName, HeaderValue, Request, Response};
use kerniq_routing::Middleware;
use pin_project_lite::pin_project;
use tower::{Layer, Service};

/// Adds common security headers to every response that does not already set
/// them.
///
/// Defaults:
///
/// | Header | Value |
/// |---|---|
/// | `X-Content-Type-Options` | `nosniff` |
/// | `X-Frame-Options` | `DENY` |
/// | `Referrer-Policy` | `no-referrer` |
/// | `Cross-Origin-Opener-Policy` | `same-origin` |
///
/// Optional: `Strict-Transport-Security` ([`hsts`](Self::hsts), only send it
/// when the service is reachable exclusively over HTTPS) and
/// `Content-Security-Policy` ([`content_security_policy`](Self::content_security_policy)).
///
/// All headers are set by a single layer that inserts pre-validated values,
/// so the per-response cost is a handful of map inserts.
#[derive(Debug, Clone)]
pub struct SecurityHeaders {
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl Default for SecurityHeaders {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityHeaders {
    /// The default header set.
    pub fn new() -> Self {
        Self {
            headers: vec![
                (
                    HeaderName::from_static("x-content-type-options"),
                    HeaderValue::from_static("nosniff"),
                ),
                (
                    HeaderName::from_static("x-frame-options"),
                    HeaderValue::from_static("DENY"),
                ),
                (
                    HeaderName::from_static("referrer-policy"),
                    HeaderValue::from_static("no-referrer"),
                ),
                (
                    HeaderName::from_static("cross-origin-opener-policy"),
                    HeaderValue::from_static("same-origin"),
                ),
            ],
        }
    }

    /// Adds `Strict-Transport-Security: max-age=<secs>; includeSubDomains`.
    pub fn hsts(self, max_age: Duration) -> Self {
        let value = format!("max-age={}; includeSubDomains", max_age.as_secs());
        self.header(
            "strict-transport-security",
            HeaderValue::from_str(&value).expect("valid header value"),
        )
    }

    /// Adds a `Content-Security-Policy`.
    ///
    /// # Panics
    ///
    /// Panics if `policy` is not a valid header value.
    pub fn content_security_policy(self, policy: &str) -> Self {
        self.header(
            "content-security-policy",
            HeaderValue::from_str(policy).expect("invalid Content-Security-Policy"),
        )
    }

    /// Adds or replaces an arbitrary header in the set.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid header name.
    pub fn header(mut self, name: &str, value: HeaderValue) -> Self {
        let name = HeaderName::try_from(name).expect("invalid header name");
        self.headers.retain(|(existing, _)| existing != name);
        self.headers.push((name, value));
        self
    }

    /// Converts into a Tower layer.
    pub fn into_layer(self) -> SetHeadersLayer {
        SetHeadersLayer {
            headers: self.headers.into(),
        }
    }
}

impl Middleware for SecurityHeaders {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

/// Layer that sets a fixed list of response headers if absent.
#[derive(Debug, Clone)]
pub struct SetHeadersLayer {
    headers: Arc<[(HeaderName, HeaderValue)]>,
}

impl<S> Layer<S> for SetHeadersLayer {
    type Service = SetHeaders<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SetHeaders {
            inner,
            headers: self.headers.clone(),
        }
    }
}

/// Service produced by [`SetHeadersLayer`].
#[derive(Debug, Clone)]
pub struct SetHeaders<S> {
    inner: S,
    headers: Arc<[(HeaderName, HeaderValue)]>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for SetHeaders<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = SetHeadersFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        SetHeadersFuture {
            inner: self.inner.call(request),
            headers: self.headers.clone(),
        }
    }
}

pin_project! {
    /// Response future for [`SetHeaders`].
    pub struct SetHeadersFuture<F> {
        #[pin]
        inner: F,
        headers: Arc<[(HeaderName, HeaderValue)]>,
    }
}

impl<F, B, E> Future for SetHeadersFuture<F>
where
    F: Future<Output = Result<Response<B>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let mut response = ready!(this.inner.poll(cx))?;
        let map = response.headers_mut();
        for (name, value) in this.headers.iter() {
            if !map.contains_key(name) {
                map.insert(name.clone(), value.clone());
            }
        }
        Poll::Ready(Ok(response))
    }
}
