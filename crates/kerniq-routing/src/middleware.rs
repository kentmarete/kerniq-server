//! Middleware composition.
//!
//! # Why Kerniq defers middleware
//!
//! Axum's `Router::layer` wraps only the routes registered *before* the call.
//! Code like the following therefore silently skips logging for `/users`:
//!
//! ```ignore
//! axum::Router::new()
//!     .layer(TraceLayer::new_for_http())   // wraps nothing yet
//!     .route("/users", get(list_users));
//! ```
//!
//! Kerniq's `.middleware(..)` / `.layer(..)` record middleware in a
//! [`MiddlewareStack`] and apply it when the router is finalized, so
//! declaration order relative to routes does not matter.
//!
//! # Execution order
//!
//! The **first** middleware declared is the **outermost**: it sees the
//! request first and the response last.
//!
//! ```text
//! App::new().middleware(A).middleware(B).get("/", handler)
//!
//! request  ──▶ A ──▶ B ──▶ handler
//! response ◀── A ◀── B ◀──┘
//! ```
//!
//! # Scope
//!
//! Middleware is attached with Axum's `Router::layer`, i.e. per route and
//! after routing. Consequences:
//!
//! * `MatchedPath` and path parameters are available to middleware.
//! * App-level middleware also wraps the 404 fallback.
//! * A `Layer` is invoked once per route, so any state that must be shared
//!   across routes (counters, semaphores, rate-limit buckets) must live
//!   behind an `Arc` inside the layer. All Kerniq middleware does this.
//! * Requests that match no route of a *router-scoped* stack never reach it.

use std::convert::Infallible;
use std::fmt;

use axum::extract::Request;
use axum::response::IntoResponse;
use axum::routing::Route;
use tower::{Layer, Service};

/// A unit of middleware that can be attached to a Kerniq `App` or `Router`.
///
/// Most middleware is a single Tower [`Layer`]; wrap such layers with
/// [`TowerLayer`] or simply pass them to `.layer(..)`. Implement
/// `Middleware` directly when one logical feature needs several layers or
/// router-level configuration (for example, a timeout plus the error
/// handler that renders it).
pub trait Middleware: Send + 'static {
    /// Applies this middleware to `router`.
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static;
}

/// Adapts any Tower [`Layer`] that Axum accepts into a [`Middleware`].
#[derive(Debug, Clone)]
pub struct TowerLayer<L>(pub L);

impl<L> Middleware for TowerLayer<L>
where
    L: Layer<Route> + Clone + Send + Sync + 'static,
    L::Service: Service<Request> + Clone + Send + Sync + 'static,
    <L::Service as Service<Request>>::Response: IntoResponse + 'static,
    <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
    <L::Service as Service<Request>>::Future: Send + 'static,
{
    fn apply<S>(self, router: axum::Router<S>) -> axum::Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        router.layer(self.0)
    }
}

type ApplyFn<S> = Box<dyn FnOnce(axum::Router<S>) -> axum::Router<S> + Send>;

/// An ordered list of middleware, applied when a router is finalized.
pub struct MiddlewareStack<S> {
    entries: Vec<(&'static str, ApplyFn<S>)>,
}

impl<S> Default for MiddlewareStack<S> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<S> fmt::Debug for MiddlewareStack<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|(name, _)| name))
            .finish()
    }
}

impl<S> MiddlewareStack<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Appends middleware (it becomes the innermost so far).
    pub fn push<M: Middleware>(&mut self, middleware: M) {
        self.entries.push((
            std::any::type_name::<M>(),
            Box::new(move |router| middleware.apply(router)),
        ));
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Wraps `router` so that the first pushed middleware is outermost.
    pub fn apply(self, router: axum::Router<S>) -> axum::Router<S> {
        // `Router::layer` makes the most recent layer outermost, so apply in
        // reverse declaration order.
        self.entries
            .into_iter()
            .rev()
            .fold(router, |router, (_, apply)| apply(router))
    }
}
