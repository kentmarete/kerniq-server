//! The Kerniq [`Router`] and route groups.

use std::fmt;

use axum::handler::Handler;
use axum::routing::{MethodFilter, MethodRouter};
use http::Method;

use crate::middleware::{Middleware, MiddlewareStack, TowerLayer};
use crate::path::{join_display, join_prefix, normalize, normalize_prefix};

/// A registered route, recorded for diagnostics (startup logs, tests,
/// tooling). Recording happens at build time only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInfo {
    /// Uppercase method name, `ANY` for method-agnostic routes, or `*` for
    /// opaque mounted Axum routers/services.
    pub method: String,
    /// The full path, including prefixes.
    pub path: String,
}

/// A composable set of routes with an optional prefix and router-scoped
/// middleware.
///
/// A `Router` wraps an [`axum::Router`] and adds:
///
/// * Flask-style registration methods (`get`, `post`, ...),
/// * a [`prefix`](Router::prefix) applied when the router is mounted,
/// * [`group`](Router::group) for registering several routes in a block,
/// * deferred, order-preserving middleware (see [`crate::middleware`]),
/// * `:id` path syntax in addition to Axum's `{id}`.
///
/// Routers are flattened into their parent when mounted (Axum's `nest` and
/// `merge` copy routes into a single route table), so composition has no
/// per-request cost.
///
/// `S` is the application state type the routes' handlers expect; it is
/// normally inferred from where the router is mounted.
///
/// ```
/// use kerniq_routing::Router;
///
/// async fn list() -> &'static str { "all users" }
/// async fn show() -> &'static str { "one user" }
///
/// let users: Router = Router::new()
///     .get("/", list)
///     .get("/:id", show);
/// ```
#[must_use = "routers do nothing unless mounted or converted"]
pub struct Router<S = ()> {
    prefix: String,
    inner: axum::Router<S>,
    middleware: MiddlewareStack<S>,
    routes: Vec<RouteInfo>,
}

impl<S> fmt::Debug for Router<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Router")
            .field("prefix", &self.prefix)
            .field("routes", &self.routes)
            .field("middleware", &self.middleware)
            .finish()
    }
}

impl<S> Default for Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Generates one by-value registration method per HTTP method.
macro_rules! method_fns {
    ($($(#[$doc:meta])* $name:ident => $filter:ident, $label:literal;)*) => {
        $(
            $(#[$doc])*
            pub fn $name<H, T>(mut self, path: &str, handler: H) -> Self
            where
                H: Handler<T, S>,
                T: 'static,
            {
                self.add(path, $label, axum::routing::on(MethodFilter::$filter, handler));
                self
            }
        )*
    };
}

/// Generates one `&mut` registration method per HTTP method for [`Group`].
macro_rules! group_method_fns {
    ($($(#[$doc:meta])* $name:ident => $filter:ident, $label:literal;)*) => {
        $(
            $(#[$doc])*
            pub fn $name<H, T>(&mut self, path: &str, handler: H) -> &mut Self
            where
                H: Handler<T, S>,
                T: 'static,
            {
                self.router.add(path, $label, axum::routing::on(MethodFilter::$filter, handler));
                self
            }
        )*
    };
}

impl<S> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Creates an empty router mounted at the root.
    pub fn new() -> Self {
        Self {
            prefix: String::new(),
            inner: axum::Router::new(),
            middleware: MiddlewareStack::default(),
            routes: Vec::new(),
        }
    }

    /// Sets the prefix under which this router's routes are served when it
    /// is mounted or converted. `:name` captures are allowed.
    ///
    /// The prefix applies to all routes regardless of whether they were
    /// registered before or after this call.
    pub fn prefix(mut self, prefix: &str) -> Self {
        let prefix = normalize_prefix(prefix);
        for route in &mut self.routes {
            let relative = route.path.strip_prefix(&self.prefix).unwrap_or(&route.path);
            route.path = join_display(&prefix, if relative.is_empty() { "/" } else { relative });
        }
        self.prefix = prefix;
        self
    }

    /// The normalized prefix (empty for the root).
    pub fn get_prefix(&self) -> &str {
        &self.prefix
    }

    /// The routes registered so far, with full paths.
    pub fn routes(&self) -> &[RouteInfo] {
        &self.routes
    }

    method_fns! {
        /// Registers a `GET` handler. Axum also answers `HEAD` for `GET` routes.
        get => GET, "GET";
        /// Registers a `POST` handler.
        post => POST, "POST";
        /// Registers a `PUT` handler.
        put => PUT, "PUT";
        /// Registers a `PATCH` handler.
        patch => PATCH, "PATCH";
        /// Registers a `DELETE` handler.
        delete => DELETE, "DELETE";
        /// Registers an `OPTIONS` handler.
        options => OPTIONS, "OPTIONS";
        /// Registers a `HEAD` handler (overrides the implicit `HEAD` of a `GET` route).
        head => HEAD, "HEAD";
        /// Registers a `TRACE` handler.
        trace => TRACE, "TRACE";
    }

    /// Registers a handler that answers every method.
    pub fn any<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.add(path, "ANY", axum::routing::any(handler));
        self
    }

    /// Registers a handler for an arbitrary standard method.
    ///
    /// # Panics
    ///
    /// Panics if `method` is an extension method (not one of the nine
    /// standard methods), which Axum's method router cannot match. Use
    /// [`any`](Router::any) and inspect the method in the handler instead.
    pub fn on<H, T>(mut self, method: Method, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let filter = method_filter(&method);
        self.add(path, method.as_str(), axum::routing::on(filter, handler));
        self
    }

    /// Registers an Axum [`MethodRouter`] — the escape hatch for advanced
    /// cases such as per-route layers or several methods built at once:
    ///
    /// ```
    /// use kerniq_routing::{Router, methods};
    ///
    /// async fn read() -> &'static str { "read" }
    /// async fn write() -> &'static str { "write" }
    ///
    /// let router: Router = Router::new().route("/items", methods::get(read).post(write));
    /// ```
    pub fn route(mut self, path: &str, method_router: MethodRouter<S>) -> Self {
        self.add(path, "ROUTE", method_router);
        self
    }

    /// Registers several routes in a block. The closure receives a
    /// [`Group`] whose methods take `&mut self`, so routes can be added as
    /// statements or chained:
    ///
    /// ```
    /// use kerniq_routing::Router;
    /// # async fn list() {} async fn create() {} async fn accounts() {}
    ///
    /// let api: Router = Router::new()
    ///     .prefix("/api/v1")
    ///     .group(|router| {
    ///         router.get("/users", list);
    ///         router.post("/users", create);
    ///         router.get("/accounts", accounts);
    ///     });
    /// ```
    pub fn group(mut self, f: impl FnOnce(&mut Group<'_, S>)) -> Self {
        f(&mut Group { router: &mut self });
        self
    }

    /// Mounts another Kerniq router under `path` (combined with that
    /// router's own prefix). The nested router's middleware is applied to its
    /// routes only.
    pub fn mount(mut self, path: &str, router: Router<S>) -> Self {
        self.add_router(path, router);
        self
    }

    /// Merges another Kerniq router at its own prefix.
    pub fn merge(self, router: Router<S>) -> Self {
        self.mount("/", router)
    }

    /// Mounts an existing [`axum::Router`] under `path`.
    pub fn mount_axum(mut self, path: &str, router: axum::Router<S>) -> Self {
        self.add_router(path, Router::from(router));
        self
    }

    /// Sets the handler for requests that match no route of this router.
    ///
    /// On a mounted router this applies to unmatched paths under its prefix
    /// (Axum's nested fallbacks).
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let inner = std::mem::take(&mut self.inner);
        self.inner = inner.fallback(handler);
        self
    }

    /// Adds router-scoped Kerniq middleware (applies only to this router's
    /// routes). See [`crate::middleware`] for ordering.
    pub fn middleware<M: Middleware>(mut self, middleware: M) -> Self {
        self.middleware.push(middleware);
        self
    }

    /// Adds a router-scoped Tower layer. Equivalent to
    /// `.middleware(TowerLayer(layer))`.
    pub fn layer<L>(self, layer: L) -> Self
    where
        TowerLayer<L>: Middleware,
    {
        self.middleware(TowerLayer(layer))
    }

    /// Converts into an [`axum::Router`], applying middleware and the prefix.
    pub fn into_axum(self) -> axum::Router<S> {
        let routes = self.middleware.apply(self.inner);
        if self.prefix.is_empty() {
            routes
        } else {
            axum::Router::new().nest(&self.prefix, routes)
        }
    }

    /// Provides the state, producing a router that needs state `S2`
    /// (usually `()`). Middleware is applied; prefix and route records are
    /// kept. Mirrors [`axum::Router::with_state`].
    pub fn with_state<S2>(self, state: S) -> Router<S2>
    where
        S2: Clone + Send + Sync + 'static,
    {
        Router {
            prefix: self.prefix,
            inner: self.middleware.apply(self.inner).with_state(state),
            middleware: MiddlewareStack::default(),
            routes: self.routes,
        }
    }

    /// Converts into an [`axum::Router`] *without* applying the prefix
    /// (middleware is applied). Used when mounting into a parent.
    fn into_parts(self) -> (String, axum::Router<S>, Vec<RouteInfo>) {
        let routes = self.middleware.apply(self.inner);
        (self.prefix, routes, self.routes)
    }

    fn add(&mut self, path: &str, label: &str, method_router: MethodRouter<S>) {
        let path = normalize(path);
        self.routes.push(RouteInfo {
            method: label.to_owned(),
            path: join_display(&self.prefix, &path),
        });
        let inner = std::mem::take(&mut self.inner);
        self.inner = inner.route(&path, method_router);
    }

    fn add_router(&mut self, path: &str, router: Router<S>) {
        let mount = normalize_prefix(path);
        let (child_prefix, child, child_routes) = router.into_parts();
        let full = join_prefix(&mount, &child_prefix);
        // Display paths for the child already include its own prefix.
        self.routes
            .extend(child_routes.into_iter().map(|r| RouteInfo {
                method: r.method,
                path: join_display(&self.prefix, &join_display(&mount, &r.path)),
            }));
        let inner = std::mem::take(&mut self.inner);
        self.inner = if full.is_empty() {
            inner.merge(child)
        } else {
            inner.nest(&full, child)
        };
    }

    pub(crate) fn record_opaque(&mut self, path: &str) {
        self.routes.push(RouteInfo {
            method: "*".to_owned(),
            path: join_display(&self.prefix, path),
        });
    }
}

impl<S> From<axum::Router<S>> for Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    /// Wraps an Axum router. Its routes are opaque to [`Router::routes`].
    fn from(router: axum::Router<S>) -> Self {
        let mut wrapped = Router::new();
        wrapped.inner = router;
        wrapped.record_opaque("/{*axum}");
        wrapped
    }
}

impl<S> From<Router<S>> for axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn from(router: Router<S>) -> Self {
        router.into_axum()
    }
}

/// A mutable view of a [`Router`] used inside [`Router::group`].
///
/// Methods take `&mut self` and return `&mut Self`, so they work both as
/// statements and chained.
pub struct Group<'a, S = ()> {
    router: &'a mut Router<S>,
}

impl<S> fmt::Debug for Group<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Group")
            .field("router", &self.router)
            .finish()
    }
}

impl<S> Group<'_, S>
where
    S: Clone + Send + Sync + 'static,
{
    group_method_fns! {
        /// Registers a `GET` handler.
        get => GET, "GET";
        /// Registers a `POST` handler.
        post => POST, "POST";
        /// Registers a `PUT` handler.
        put => PUT, "PUT";
        /// Registers a `PATCH` handler.
        patch => PATCH, "PATCH";
        /// Registers a `DELETE` handler.
        delete => DELETE, "DELETE";
        /// Registers an `OPTIONS` handler.
        options => OPTIONS, "OPTIONS";
        /// Registers a `HEAD` handler.
        head => HEAD, "HEAD";
        /// Registers a `TRACE` handler.
        trace => TRACE, "TRACE";
    }

    /// Registers a handler that answers every method.
    pub fn any<H, T>(&mut self, path: &str, handler: H) -> &mut Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.router.add(path, "ANY", axum::routing::any(handler));
        self
    }

    /// Registers a handler for an arbitrary standard method. See [`Router::on`].
    pub fn on<H, T>(&mut self, method: Method, path: &str, handler: H) -> &mut Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let filter = method_filter(&method);
        self.router
            .add(path, method.as_str(), axum::routing::on(filter, handler));
        self
    }

    /// Registers an Axum [`MethodRouter`]. See [`Router::route`].
    pub fn route(&mut self, path: &str, method_router: MethodRouter<S>) -> &mut Self {
        self.router.add(path, "ROUTE", method_router);
        self
    }

    /// Mounts a Kerniq router under `path`. See [`Router::mount`].
    pub fn mount(&mut self, path: &str, router: Router<S>) -> &mut Self {
        self.router.add_router(path, router);
        self
    }
}

fn method_filter(method: &Method) -> MethodFilter {
    MethodFilter::try_from(method.clone()).unwrap_or_else(|_| {
        panic!(
            "cannot route extension method `{method}`: only standard HTTP methods can be \
             matched. Register the route with `any(..)` and inspect the method in the handler"
        )
    })
}
