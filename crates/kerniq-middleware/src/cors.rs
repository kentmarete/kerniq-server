//! Cross-origin resource sharing.

use std::time::Duration;

use http::{HeaderName, HeaderValue, Method};
use kerniq_routing::Middleware;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// CORS headers and preflight handling (tower-http's `CorsLayer`).
///
/// * [`Cors::default()`] — any origin, the standard REST methods, any
///   request header, no credentials, preflight cached for one hour. Suitable
///   for public APIs and development.
/// * [`Cors::new()`] — allows nothing until configured; use for
///   credentialed or browser-facing production APIs.
/// * [`Cors::from`] a hand-built `CorsLayer` for anything else.
///
/// ```
/// use kerniq_middleware::Cors;
///
/// let public = Cors::default();
///
/// let dashboard = Cors::new()
///     .allow_origins(["https://dashboard.example.com"])
///     .allow_methods([http::Method::GET, http::Method::POST])
///     .allow_headers(["content-type", "authorization"])
///     .allow_credentials(true);
/// ```
///
/// Requires the `cors` feature.
#[derive(Debug, Clone)]
pub struct Cors {
    layer: CorsLayer,
}

impl Default for Cors {
    fn default() -> Self {
        Self {
            layer: CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::HEAD,
                    Method::OPTIONS,
                ])
                .allow_headers(Any)
                .max_age(Duration::from_secs(3600)),
        }
    }
}

impl Cors {
    /// A restrictive policy that allows no cross-origin requests until
    /// configured.
    pub fn new() -> Self {
        Self {
            layer: CorsLayer::new(),
        }
    }

    /// tower-http's `CorsLayer::permissive()`: any origin, method and header.
    pub fn permissive() -> Self {
        Self {
            layer: CorsLayer::permissive(),
        }
    }

    /// Allows the listed origins (exact match).
    ///
    /// # Panics
    ///
    /// Panics if an origin is not a valid header value.
    pub fn allow_origins<I, T>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let origins: Vec<HeaderValue> = origins
            .into_iter()
            .map(|o| HeaderValue::from_str(o.as_ref()).expect("invalid CORS origin"))
            .collect();
        self.layer = self.layer.allow_origin(AllowOrigin::list(origins));
        self
    }

    /// Allows the listed methods.
    pub fn allow_methods<I: IntoIterator<Item = Method>>(mut self, methods: I) -> Self {
        self.layer = self
            .layer
            .allow_methods(methods.into_iter().collect::<Vec<_>>());
        self
    }

    /// Allows the listed request headers.
    ///
    /// # Panics
    ///
    /// Panics if a name is not a valid header name.
    pub fn allow_headers<I, T>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let headers: Vec<HeaderName> = headers
            .into_iter()
            .map(|h| HeaderName::try_from(h.as_ref()).expect("invalid CORS header name"))
            .collect();
        self.layer = self.layer.allow_headers(headers);
        self
    }

    /// Exposes the listed response headers to scripts.
    ///
    /// # Panics
    ///
    /// Panics if a name is not a valid header name.
    pub fn expose_headers<I, T>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let headers: Vec<HeaderName> = headers
            .into_iter()
            .map(|h| HeaderName::try_from(h.as_ref()).expect("invalid CORS header name"))
            .collect();
        self.layer = self.layer.expose_headers(headers);
        self
    }

    /// Allows credentials (cookies, `Authorization`). Cannot be combined
    /// with wildcard origins, methods or headers (tower-http panics when the
    /// layer is built in that case).
    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.layer = self.layer.allow_credentials(allow);
        self
    }

    /// How long browsers may cache preflight responses.
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.layer = self.layer.max_age(max_age);
        self
    }

    /// The underlying `CorsLayer`.
    pub fn into_layer(self) -> CorsLayer {
        self.layer
    }
}

impl From<CorsLayer> for Cors {
    fn from(layer: CorsLayer) -> Self {
        Self { layer }
    }
}

impl Middleware for Cors {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.layer)
    }
}
