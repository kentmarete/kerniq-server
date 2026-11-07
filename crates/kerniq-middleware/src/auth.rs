//! Authentication extension points.
//!
//! Kerniq does not ship an authentication *provider* (no JWT library, no
//! session store, no OAuth client). It provides the plumbing:
//!
//! * [`Authenticator`] — implement this for your scheme (JWT validation,
//!   API-key lookup, mTLS identity, ...),
//! * [`Auth`] — middleware that runs the authenticator and rejects
//!   unauthenticated requests with `401 UNAUTHORIZED`,
//! * [`Identity<T>`] — extractor that gives handlers the authenticated
//!   identity.
//!
//! ```
//! use kerniq_core::Error;
//! use kerniq_middleware::auth::{Auth, Identity};
//!
//! #[derive(Clone)]
//! struct User { id: u64 }
//!
//! // Bearer-token authentication from an async closure.
//! let auth = Auth::bearer(|token: String| async move {
//!     if token == "secret-token" {
//!         Ok(User { id: 1 })
//!     } else {
//!         Err(Error::unauthorized("Invalid token"))
//!     }
//! });
//!
//! async fn me(Identity(user): Identity<User>) -> String {
//!     format!("user {}", user.id)
//! }
//! ```
//!
//! Apply `Auth` to a router holding only protected routes (router-scoped
//! middleware), or to the whole app.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::extract::{FromRequestParts, OptionalFromRequestParts, Request};
use axum::response::{IntoResponse, Response};
use http::HeaderValue;
use http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use http::request::Parts;
use kerniq_core::Error;
use kerniq_routing::Middleware;
use tower::{Layer, Service};

/// Authenticates a request from its head (method, URI, headers,
/// extensions).
///
/// Return `Err` (usually [`Error::unauthorized`] or [`Error::forbidden`]) to
/// reject the request; the error is sent as the response.
pub trait Authenticator: Clone + Send + Sync + 'static {
    /// The identity made available to handlers via [`Identity<T>`].
    type Identity: Clone + Send + Sync + 'static;

    /// Authenticates the request.
    fn authenticate(
        &self,
        parts: &Parts,
    ) -> impl Future<Output = Result<Self::Identity, Error>> + Send;
}

/// Authentication middleware. See the [module documentation](self).
#[derive(Clone)]
pub struct Auth<A> {
    authenticator: A,
}

impl<A> fmt::Debug for Auth<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("authenticator", &std::any::type_name::<A>())
            .finish()
    }
}

impl<A: Authenticator> Auth<A> {
    /// Uses a custom [`Authenticator`].
    pub fn new(authenticator: A) -> Self {
        Self { authenticator }
    }
}

impl<F, Fut, I> Auth<BearerAuth<F>>
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<I, Error>> + Send,
    I: Clone + Send + Sync + 'static,
{
    /// Authenticates `Authorization: Bearer <token>` with an async function
    /// from the token to an identity. Missing or malformed headers are
    /// rejected with `401` and `WWW-Authenticate: Bearer` before `validate`
    /// is called.
    pub fn bearer(validate: F) -> Self {
        Self::new(BearerAuth { validate })
    }
}

/// Bearer-token [`Authenticator`] created by [`Auth::bearer`].
#[derive(Clone)]
pub struct BearerAuth<F> {
    validate: F,
}

impl<F> fmt::Debug for BearerAuth<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BearerAuth").finish_non_exhaustive()
    }
}

fn bearer_challenge(message: &'static str) -> Error {
    Error::unauthorized(message).with_header(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))
}

impl<F, Fut, I> Authenticator for BearerAuth<F>
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<I, Error>> + Send,
    I: Clone + Send + Sync + 'static,
{
    type Identity = I;

    fn authenticate(&self, parts: &Parts) -> impl Future<Output = Result<I, Error>> + Send {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        let validate = self.validate.clone();
        async move {
            match token {
                Some(token) => validate(token).await,
                None => Err(bearer_challenge("Missing or malformed bearer token")),
            }
        }
    }
}

impl<A: Authenticator> Middleware for Auth<A> {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(AuthLayer {
            authenticator: self.authenticator,
        })
    }
}

/// Tower layer for [`Auth`].
#[derive(Clone)]
pub struct AuthLayer<A> {
    authenticator: A,
}

impl<A> fmt::Debug for AuthLayer<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthLayer").finish_non_exhaustive()
    }
}

impl<A: Clone, S> Layer<S> for AuthLayer<A> {
    type Service = AuthService<A, S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            authenticator: self.authenticator.clone(),
            inner,
        }
    }
}

/// Service produced by [`AuthLayer`].
#[derive(Clone)]
pub struct AuthService<A, S> {
    authenticator: A,
    inner: S,
}

impl<A, S> fmt::Debug for AuthService<A, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthService").finish_non_exhaustive()
    }
}

impl<A, S> Service<Request> for AuthService<A, S>
where
    A: Authenticator,
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        // Use the service that was driven to readiness; leave a fresh clone
        // behind for the next call (standard Tower pattern).
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let authenticator = self.authenticator.clone();
        Box::pin(async move {
            let (mut parts, body) = request.into_parts();
            match authenticator.authenticate(&parts).await {
                Ok(identity) => {
                    parts.extensions.insert(AuthIdentity(identity));
                    inner.call(Request::from_parts(parts, body)).await
                }
                Err(error) => Ok(error.into_response()),
            }
        })
    }
}

/// Request extension wrapper so identities cannot collide with other
/// extensions of the same type.
#[derive(Clone)]
struct AuthIdentity<T>(T);

/// Extracts the identity produced by [`Auth`].
///
/// Rejects with `401 UNAUTHORIZED` if no identity of type `T` is present
/// (for example when the route is not covered by `Auth`), so a
/// misconfiguration fails closed. Use `Option<Identity<T>>` for routes where
/// authentication is optional.
#[derive(Debug, Clone)]
pub struct Identity<T>(pub T);

impl<T, S> FromRequestParts<S> for Identity<T>
where
    T: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthIdentity<T>>()
            .map(|AuthIdentity(identity)| Identity(identity.clone()))
            .ok_or_else(|| Error::unauthorized("Authentication required"))
    }
}

impl<T, S> OptionalFromRequestParts<S> for Identity<T>
where
    T: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<AuthIdentity<T>>()
            .map(|AuthIdentity(identity)| Identity(identity.clone())))
    }
}
