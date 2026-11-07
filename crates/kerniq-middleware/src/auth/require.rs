//! Router-level authorization: [`Require`].

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use kerniq_core::Error;
use kerniq_routing::Middleware;
use pin_project_lite::pin_project;
use tower::{Layer, Service};

use super::{AuthIdentity, Authorities, Policy};

type Check<T> = Arc<dyn Fn(&T) -> Result<(), Error> + Send + Sync>;

const DEFAULT_MESSAGE: &str = "Insufficient permissions";

/// Middleware that requires the identity produced by [`Auth`](super::Auth)
/// to satisfy a rule, for every route of the router it is applied to.
///
/// It is the router-level counterpart of `#[authorize(..)]` and
/// [`Authorized<T, P>`](super::Authorized): put routes that share a
/// requirement in one router and apply `Require` to it. Requests without an
/// identity of type `T` are rejected with `401 UNAUTHORIZED`, identities
/// that fail the rule with `403 FORBIDDEN`. Apply it *after* `Auth` (or on
/// an inner router), so that authentication runs first.
///
/// ```
/// use kerniq_core::Error;
/// use kerniq_middleware::auth::{Auth, Authorities, Require};
/// use kerniq_routing::Router;
///
/// #[derive(Clone)]
/// struct User { roles: Vec<String> }
///
/// impl Authorities for User {
///     fn has_role(&self, role: &str) -> bool {
///         self.roles.iter().any(|r| r == role)
///     }
/// }
///
/// let auth = Auth::bearer(|_token: String| async move {
///     Ok::<_, Error>(User { roles: vec!["ADMIN".into()] })
/// });
///
/// let admin: Router = Router::new()
///     .middleware(auth)
///     .middleware(Require::<User>::role("ADMIN"))
///     .get("/stats", || async { "..." });
/// ```
pub struct Require<T> {
    check: Check<T>,
}

impl<T> Clone for Require<T> {
    fn clone(&self) -> Self {
        Self {
            check: Arc::clone(&self.check),
        }
    }
}

impl<T> fmt::Debug for Require<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Require")
            .field("identity", &std::any::type_name::<T>())
            .finish_non_exhaustive()
    }
}

fn names<I, N>(names: I) -> Arc<[String]>
where
    I: IntoIterator<Item = N>,
    N: Into<String>,
{
    names.into_iter().map(Into::into).collect()
}

impl<T: Send + Sync + 'static> Require<T> {
    /// Requires `rule` to return `true` for the identity.
    pub fn check<F>(rule: F) -> Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        Self::from_fn(move |identity| {
            if rule(identity) {
                Ok(())
            } else {
                Err(Error::forbidden(DEFAULT_MESSAGE))
            }
        })
    }

    /// Requires an identity of type `T`, with no further rule.
    pub fn authenticated() -> Self {
        Self::from_fn(|_| Ok(()))
    }

    /// Requires the [`Policy`] `P`, the same policy types used with
    /// [`Authorized<T, P>`](super::Authorized). Errors returned by the
    /// policy are sent as-is.
    pub fn policy<P: Policy<T>>() -> Self {
        Self::from_fn(P::check)
    }

    fn from_fn(check: impl Fn(&T) -> Result<(), Error> + Send + Sync + 'static) -> Self {
        Self {
            check: Arc::new(check),
        }
    }

    /// Requires both `self` and `other`. Fails with the first failing
    /// rule's error.
    pub fn and(self, other: Self) -> Self {
        Self::from_fn(move |identity| {
            (self.check)(identity)?;
            (other.check)(identity)
        })
    }

    /// Requires `self` or `other`. Fails with `other`'s error when both
    /// fail.
    pub fn or(self, other: Self) -> Self {
        Self::from_fn(move |identity| (self.check)(identity).or_else(|_| (other.check)(identity)))
    }

    /// Replaces the error of a failing rule with `403 FORBIDDEN` and
    /// `message` (default `"Insufficient permissions"`).
    pub fn message(self, message: impl Into<String>) -> Self {
        let message: Arc<str> = message.into().into();
        Self::from_fn(move |identity| {
            (self.check)(identity).map_err(|_| Error::forbidden(message.to_string()))
        })
    }

    /// Converts into a Tower layer, for `ServiceBuilder` or plain Axum.
    pub fn into_layer(self) -> RequireLayer<T> {
        RequireLayer { check: self.check }
    }
}

impl<T: Authorities + Send + Sync + 'static> Require<T> {
    /// Requires `role`.
    pub fn role(role: impl Into<String>) -> Self {
        let role = role.into();
        Self::check(move |identity: &T| identity.has_role(&role))
    }

    /// Requires at least one of `roles`.
    pub fn any_role<I, N>(roles: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: Into<String>,
    {
        let roles = names(roles);
        Self::check(move |identity: &T| roles.iter().any(|r| identity.has_role(r)))
    }

    /// Requires every one of `roles`.
    pub fn all_roles<I, N>(roles: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: Into<String>,
    {
        let roles = names(roles);
        Self::check(move |identity: &T| roles.iter().all(|r| identity.has_role(r)))
    }

    /// Requires `permission`.
    pub fn permission(permission: impl Into<String>) -> Self {
        let permission = permission.into();
        Self::check(move |identity: &T| identity.has_permission(&permission))
    }

    /// Requires at least one of `permissions`.
    pub fn any_permission<I, N>(permissions: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: Into<String>,
    {
        let permissions = names(permissions);
        Self::check(move |identity: &T| permissions.iter().any(|p| identity.has_permission(p)))
    }

    /// Requires every one of `permissions`.
    pub fn all_permissions<I, N>(permissions: I) -> Self
    where
        I: IntoIterator<Item = N>,
        N: Into<String>,
    {
        let permissions = names(permissions);
        Self::check(move |identity: &T| permissions.iter().all(|p| identity.has_permission(p)))
    }
}

impl<T: Send + Sync + 'static> Middleware for Require<T> {
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.into_layer())
    }
}

/// Tower layer for [`Require`].
pub struct RequireLayer<T> {
    check: Check<T>,
}

impl<T> Clone for RequireLayer<T> {
    fn clone(&self) -> Self {
        Self {
            check: Arc::clone(&self.check),
        }
    }
}

impl<T> fmt::Debug for RequireLayer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequireLayer").finish_non_exhaustive()
    }
}

impl<T, S> Layer<S> for RequireLayer<T> {
    type Service = RequireService<T, S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireService {
            check: Arc::clone(&self.check),
            inner,
        }
    }
}

/// Service produced by [`RequireLayer`].
pub struct RequireService<T, S> {
    check: Check<T>,
    inner: S,
}

impl<T, S: Clone> Clone for RequireService<T, S> {
    fn clone(&self) -> Self {
        Self {
            check: Arc::clone(&self.check),
            inner: self.inner.clone(),
        }
    }
}

impl<T, S> fmt::Debug for RequireService<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequireService").finish_non_exhaustive()
    }
}

impl<T, S> Service<Request> for RequireService<T, S>
where
    T: Send + Sync + 'static,
    S: Service<Request, Response = Response>,
{
    type Response = Response;
    type Error = S::Error;
    type Future = RequireFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let verdict = match request.extensions().get::<AuthIdentity<T>>() {
            Some(AuthIdentity(identity)) => (self.check)(identity),
            None => Err(Error::unauthorized("Authentication required")),
        };
        match verdict {
            Ok(()) => RequireFuture::Inner {
                future: self.inner.call(request),
            },
            Err(error) => RequireFuture::Rejected {
                response: Some(error.into_response()),
            },
        }
    }
}

pin_project! {
    /// Response future of [`RequireService`].
    #[project = RequireFutureProj]
    pub enum RequireFuture<F> {
        /// The rule passed; the inner service is running.
        Inner { #[pin] future: F },
        /// The rule failed; the rejection is returned immediately.
        Rejected { response: Option<Response> },
    }
}

impl<F> fmt::Debug for RequireFuture<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequireFuture").finish_non_exhaustive()
    }
}

impl<F, E> Future for RequireFuture<F>
where
    F: Future<Output = Result<Response, E>>,
{
    type Output = Result<Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            RequireFutureProj::Inner { future } => future.poll(cx),
            RequireFutureProj::Rejected { response } => Poll::Ready(Ok(response
                .take()
                .expect("RequireFuture polled after completion"))),
        }
    }
}
