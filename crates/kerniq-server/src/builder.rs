//! The [`App`] builder.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use axum::handler::Handler;
use axum::routing::MethodRouter;
use http::Method;
use kerniq_core::config::DEFAULT_MAX_BODY_SIZE;
use kerniq_core::{Error, Result, ServerConfig};
use kerniq_middleware::{BodyLimit, ConcurrencyLimit, Timeout};
use kerniq_routing::{Group, Middleware, MiddlewareStack, RouteInfo, Router, TowerLayer};
use kerniq_runtime::{BackgroundTasks, ServeOptions, Shutdown};
use tokio::net::TcpListener;

type Signal = Pin<Box<dyn Future<Output = ()> + Send>>;
type TaskStarter = Box<dyn FnOnce(&BackgroundTasks) + Send>;

/// A Kerniq application: routes, state, middleware, configuration and
/// lifecycle in one builder.
///
/// ```no_run
/// use kerniq_server::prelude::*;
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     App::new()
///         .get("/", || async { "Hello from Kerniq" })
///         .run("127.0.0.1:3000")
///         .await
/// }
/// ```
///
/// # State
///
/// `App<S>` is parameterized by its state type. [`App::new`] creates an
/// `App<()>`; [`App::state`] switches to `App<T>`, after which handlers can
/// extract `State<T>`. Routes registered before `state` (which cannot use
/// `State<T>`) are kept.
///
/// # Finalization
///
/// Nothing is assembled until the app is run or converted with
/// [`App::into_axum_router`]. At that point:
///
/// 1. routes are combined into a single `axum::Router` (no runtime nesting),
/// 2. the JSON 404/405 fallbacks are installed,
/// 3. the state is provided,
/// 4. middleware is applied in declaration order (first = outermost),
///    followed by the middleware implied by [`ServerConfig`]
///    (`request_timeout` → `max_concurrent_requests` → `max_body_size`,
///    outermost to innermost).
#[must_use = "an App does nothing until it is run"]
pub struct App<S = ()> {
    router: Router<S>,
    state: S,
    middleware: MiddlewareStack<()>,
    config: ServerConfig,
    custom_fallback: bool,
    shutdown: Shutdown,
    signal: Option<Signal>,
    tasks: Vec<TaskStarter>,
    #[cfg(feature = "observability")]
    observability: Option<kerniq_observability::Observability>,
}

impl<S> fmt::Debug for App<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("App");
        let d = d
            .field("router", &self.router)
            .field("middleware", &self.middleware)
            .field("config", &self.config)
            .field("background_tasks", &self.tasks.len());
        #[cfg(feature = "observability")]
        let d = d.field("observability", &self.observability);
        d.finish_non_exhaustive()
    }
}

impl Default for App<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl App<()> {
    /// Creates an application with no routes, no state and default
    /// configuration.
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            state: (),
            middleware: MiddlewareStack::default(),
            config: ServerConfig::default(),
            custom_fallback: false,
            shutdown: Shutdown::new(),
            signal: None,
            tasks: Vec::new(),
            #[cfg(feature = "observability")]
            observability: None,
        }
    }

    /// Sets the application state, available to handlers via `State<T>`.
    ///
    /// The state is cloned for each request that extracts it, so it should
    /// be cheap to clone: wrap expensive resources in `Arc` (connection
    /// pools usually already are). Sub-states can be extracted with Axum's
    /// `FromRef`.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use kerniq_server::prelude::*;
    ///
    /// #[derive(Clone)]
    /// struct AppState {
    ///     service_name: Arc<str>,
    /// }
    ///
    /// async fn name(State(state): State<AppState>) -> String {
    ///     state.service_name.to_string()
    /// }
    ///
    /// let app = App::new()
    ///     .state(AppState { service_name: "customer-api".into() })
    ///     .get("/name", name);
    /// ```
    pub fn state<T>(self, state: T) -> App<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        App {
            router: self.router.with_state(()),
            state,
            middleware: self.middleware,
            config: self.config,
            custom_fallback: self.custom_fallback,
            shutdown: self.shutdown,
            signal: self.signal,
            tasks: self.tasks,
            #[cfg(feature = "observability")]
            observability: self.observability,
        }
    }
}

macro_rules! delegate_methods {
    ($($(#[$doc:meta])* $name:ident;)*) => {
        $(
            $(#[$doc])*
            pub fn $name<H, T>(mut self, path: &str, handler: H) -> Self
            where
                H: Handler<T, S>,
                T: 'static,
            {
                self.router = self.router.$name(path, handler);
                self
            }
        )*
    };
}

impl<S> App<S>
where
    S: Clone + Send + Sync + 'static,
{
    delegate_methods! {
        /// Registers a `GET` handler (also answers `HEAD`).
        ///
        /// Paths use `{name}` captures; `:name` is accepted and converted.
        get;
        /// Registers a `POST` handler.
        post;
        /// Registers a `PUT` handler.
        put;
        /// Registers a `PATCH` handler.
        patch;
        /// Registers a `DELETE` handler.
        delete;
        /// Registers an `OPTIONS` handler.
        options;
        /// Registers a `HEAD` handler.
        head;
        /// Registers a `TRACE` handler.
        trace;
        /// Registers a handler for every method.
        any;
    }

    /// Registers a handler for an arbitrary standard method.
    /// See [`Router::on`].
    pub fn on<H, T>(mut self, method: Method, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.router = self.router.on(method, path, handler);
        self
    }

    /// Registers an Axum `MethodRouter` (advanced use: per-route layers,
    /// several methods at once). See [`Router::route`].
    pub fn route(mut self, path: &str, method_router: MethodRouter<S>) -> Self {
        self.router = self.router.route(path, method_router);
        self
    }

    /// Registers several routes in a block. See [`Router::group`].
    pub fn group(mut self, f: impl FnOnce(&mut Group<'_, S>)) -> Self {
        self.router = self.router.group(f);
        self
    }

    /// Mounts a Kerniq [`Router`] under `path` (combined with the router's
    /// own prefix).
    ///
    /// ```
    /// use kerniq_server::prelude::*;
    /// # async fn list() {} async fn create() {} async fn accounts() {}
    ///
    /// let users = Router::new().get("/", list).post("/", create);
    /// let app = App::new()
    ///     .mount("/users", users)
    ///     .mount("/accounts", Router::new().get("/", accounts));
    /// ```
    pub fn mount(mut self, path: &str, router: Router<S>) -> Self {
        self.router = self.router.mount(path, router);
        self
    }

    /// Merges a Kerniq [`Router`] at its own [prefix](Router::prefix).
    ///
    /// ```
    /// use kerniq_server::prelude::*;
    /// # async fn users() {}
    ///
    /// let api = Router::new()
    ///     .prefix("/api/v1")
    ///     .group(|r| { r.get("/users", users); });
    /// let app = App::new().merge(api);
    /// ```
    pub fn merge(mut self, router: Router<S>) -> Self {
        self.router = self.router.merge(router);
        self
    }

    /// Mounts an existing [`axum::Router`] under `path` (`"/"` merges it at
    /// the root). Its routes, layers and handlers are used as-is.
    pub fn mount_axum(mut self, path: &str, router: axum::Router<S>) -> Self {
        self.router = self.router.mount_axum(path, router);
        self
    }

    /// Adds application-wide middleware. The first middleware added is the
    /// outermost. Applies to every route and to the 404 fallback, regardless
    /// of whether routes are registered before or after this call.
    pub fn middleware<M: Middleware>(mut self, middleware: M) -> Self {
        self.middleware.push(middleware);
        self
    }

    /// Adds an application-wide Tower layer. Shares the ordered stack with
    /// [`middleware`](Self::middleware).
    pub fn layer<L>(self, layer: L) -> Self
    where
        TowerLayer<L>: Middleware,
    {
        self.middleware(TowerLayer(layer))
    }

    /// Replaces the default JSON `404 NOT_FOUND` fallback.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.router = self.router.fallback(handler);
        self.custom_fallback = true;
        self
    }

    /// Enables observability (feature `observability`).
    ///
    /// ```no_run
    /// use kerniq_server::prelude::*;
    /// use kerniq_server::observability::prelude::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let observability = Observability::builder()
    ///         .service_name("payment-api")
    ///         .metrics(MetricsConfig::new())
    ///         .build()
    ///         .await?;
    ///
    ///     App::new()
    ///         .observability(observability)
    ///         .get("/health", || async { "OK" })
    ///         .run("0.0.0.0:3000")
    ///         .await
    /// }
    /// ```
    ///
    /// At finalization:
    ///
    /// * HTTP instrumentation (metrics, a span per request, W3C context
    ///   extraction) becomes the **outermost** middleware, so it measures
    ///   everything inside it, including timeouts, rate limiting and the
    ///   404 fallback;
    /// * a Prometheus endpoint configured on the application listener is
    ///   added *outside* all middleware, so scrapes are never instrumented,
    ///   rate-limited or timed out; one configured with its own listener is
    ///   served by a background task bound when the app starts.
    ///
    /// Under [`run`](Self::run)/[`serve`](Self::serve), telemetry is flushed
    /// and shut down after connections and background tasks have drained,
    /// bounded by the observability shutdown timeout. With
    /// [`into_axum_router`](Self::into_axum_router) the application owns the
    /// handle and calls `shutdown` itself.
    #[cfg(feature = "observability")]
    pub fn observability(mut self, observability: kerniq_observability::Observability) -> Self {
        self.observability = Some(observability);
        self
    }

    /// Sets the server configuration.
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// The current server configuration.
    pub fn server_config(&self) -> &ServerConfig {
        &self.config
    }

    /// Registers a background task started when the server starts. It
    /// receives a [`Shutdown`] handle and should return promptly once it
    /// triggers; the server waits up to `shutdown_timeout` for it.
    ///
    /// ```
    /// use std::time::Duration;
    /// use kerniq_server::prelude::*;
    ///
    /// let app = App::new().background_task("heartbeat", |shutdown: Shutdown| async move {
    ///     let mut tick = tokio::time::interval(Duration::from_secs(30));
    ///     loop {
    ///         tokio::select! {
    ///             _ = tick.tick() => tracing::info!("still alive"),
    ///             () = shutdown.wait() => break,
    ///         }
    ///     }
    /// });
    /// ```
    pub fn background_task<F, Fut>(mut self, name: &'static str, task: F) -> Self
    where
        F: FnOnce(Shutdown) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.tasks.push(Box::new(move |tasks: &BackgroundTasks| {
            tasks.spawn(name, task)
        }));
        self
    }

    /// Replaces the default shutdown trigger (Ctrl+C / SIGTERM) with
    /// `signal`. Shutdown can also be triggered at any time through
    /// [`shutdown_handle`](Self::shutdown_handle).
    pub fn shutdown_signal<F>(mut self, signal: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.signal = Some(Box::pin(signal));
        self
    }

    /// A handle that triggers or observes this app's shutdown. Store it in
    /// state so long-lived handlers (WebSockets, streams) can finish when the
    /// server stops.
    pub fn shutdown_handle(&self) -> Shutdown {
        self.shutdown.clone()
    }

    /// The registered routes (for diagnostics; mounted Axum routers appear
    /// as a single opaque entry).
    pub fn routes(&self) -> &[RouteInfo] {
        self.router.routes()
    }

    /// Finalizes the application into a plain [`axum::Router`], with
    /// fallbacks, state and all middleware applied.
    ///
    /// Use this to run Kerniq under `axum::serve`, embed it into a larger
    /// Axum application, or test it with `tower::ServiceExt::oneshot`.
    /// Lifecycle features (background tasks, shutdown signal, connection
    /// limits) belong to [`App::run`] and are not part of the router.
    pub fn into_axum_router(self) -> axum::Router {
        self.into_parts().0
    }

    fn into_parts(self) -> (axum::Router, Lifecycle) {
        let App {
            router,
            state,
            middleware,
            config,
            custom_fallback,
            shutdown,
            signal,
            tasks,
            #[cfg(feature = "observability")]
            observability,
        } = self;

        let router = router.into_axum();
        let router = if custom_fallback {
            router
        } else {
            router.fallback(not_found)
        };
        let router = router
            .method_not_allowed_fallback(method_not_allowed)
            .with_state(state);

        let mut middleware = middleware;
        if let Some(timeout) = config.request_timeout {
            middleware.push(Timeout::new(timeout));
        }
        if let Some(max) = config.max_concurrent_requests {
            middleware.push(ConcurrencyLimit::new(max));
        }
        // Axum already enforces a 2 MiB default; adding an identical layer
        // would only cost an extra service boxing per request.
        if config.max_body_size != DEFAULT_MAX_BODY_SIZE {
            middleware.push(BodyLimit::new(config.max_body_size));
        }
        let router = middleware.apply(router);

        #[cfg(feature = "observability")]
        let router = match &observability {
            Some(observability) => attach_observability(router, observability),
            None => router,
        };

        (
            router,
            Lifecycle {
                config,
                shutdown,
                signal,
                tasks,
                #[cfg(feature = "observability")]
                observability,
            },
        )
    }

    /// Binds `address` and serves until shutdown. Requires a Tokio runtime
    /// (for example `#[tokio::main]`).
    ///
    /// `address` is anything `tokio::net::TcpListener::bind` accepts as a
    /// string: `"127.0.0.1:3000"`, `"0.0.0.0:8080"`, `"localhost:3000"`,
    /// `"[::1]:3000"`. To use the configured address, pass
    /// `config.address()`.
    pub async fn run(self, address: impl AsRef<str>) -> Result<()> {
        let listener = kerniq_runtime::bind(address.as_ref()).await?;
        self.serve_inner(listener, false).await
    }

    /// Serves on an already-bound listener (useful for tests with port 0,
    /// socket activation, or binding before dropping privileges).
    pub async fn serve(self, listener: TcpListener) -> Result<()> {
        self.serve_inner(listener, false).await
    }

    /// Like [`run`](Self::run), but creates and owns a multi-threaded Tokio
    /// runtime (with `workers` threads, if configured).
    ///
    /// ```no_run
    /// use kerniq_server::prelude::*;
    ///
    /// fn main() -> Result<()> {
    ///     App::new()
    ///         .get("/", || async { "OK" })
    ///         .run_blocking("0.0.0.0:3000")
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error (and does not start) if called from inside a Tokio
    /// runtime: nested runtimes are not allowed. Use `.run(addr).await` there.
    pub fn run_blocking(self, address: impl AsRef<str>) -> Result<()> {
        let workers = self.config.workers;
        let timeout = self.config.shutdown_timeout;
        let address = address.as_ref().to_owned();
        kerniq_runtime::runtime::block_on(workers, timeout, async move {
            let listener = kerniq_runtime::bind(&address).await?;
            self.serve_inner(listener, true).await
        })?
    }

    async fn serve_inner(self, listener: TcpListener, owns_runtime: bool) -> Result<()> {
        if !owns_runtime && self.config.workers.is_some() {
            tracing::warn!(
                "`ServerConfig::workers` is ignored by `run`/`serve` because the application owns \
                 the Tokio runtime; configure `#[tokio::main(worker_threads = N)]` or use `run_blocking`"
            );
        }
        for route in self.router.routes() {
            tracing::debug!(method = %route.method, path = %route.path, "route registered");
        }

        let (router, lifecycle) = self.into_parts();
        let Lifecycle {
            config,
            shutdown,
            signal,
            tasks,
            #[cfg(feature = "observability")]
            observability,
        } = lifecycle;

        let background = BackgroundTasks::new(shutdown.clone());
        #[cfg(feature = "observability")]
        if let Some(observability) = &observability {
            start_metrics_listener(observability, &background).await?;
        }
        for start in tasks {
            start(&background);
        }

        let signal = signal.unwrap_or_else(|| Box::pin(kerniq_runtime::os_signal()));
        let signal_task = tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                tokio::select! {
                    () = signal => shutdown.trigger(),
                    () = shutdown.wait() => {}
                }
            }
        });

        let result = kerniq_runtime::serve(
            listener,
            router,
            ServeOptions::from(&config),
            shutdown.clone(),
        )
        .await;

        shutdown.trigger();
        signal_task.abort();
        if !background.is_empty() {
            background.shutdown(config.shutdown_timeout).await;
        }
        tracing::info!("kerniq server stopped");
        #[cfg(feature = "observability")]
        if let Some(observability) = observability {
            let report = observability.shutdown().await;
            if report.timed_out || !report.errors.is_empty() {
                tracing::warn!(
                    duration = ?report.duration,
                    errors = ?report.errors,
                    "telemetry shutdown incomplete"
                );
            }
        }
        result.map_err(Error::from)
    }
}

struct Lifecycle {
    config: ServerConfig,
    shutdown: Shutdown,
    signal: Option<Signal>,
    tasks: Vec<TaskStarter>,
    #[cfg(feature = "observability")]
    observability: Option<kerniq_observability::Observability>,
}

#[cfg(feature = "observability")]
fn attach_observability(
    router: axum::Router,
    observability: &kerniq_observability::Observability,
) -> axum::Router {
    // Applied last, so it is the outermost layer on every route.
    let router = observability.http_instrumentation().apply(router);
    #[cfg(feature = "prometheus")]
    if let Some(prometheus) = observability.prometheus()
        && prometheus.config().listen_address().is_none()
    {
        // Merged after every layer: scrapes bypass all middleware.
        return router.merge(prometheus.router());
    }
    router
}

#[cfg(feature = "observability")]
#[allow(clippy::unused_async)]
async fn start_metrics_listener(
    observability: &kerniq_observability::Observability,
    background: &BackgroundTasks,
) -> Result<()> {
    #[cfg(feature = "prometheus")]
    if let Some(prometheus) = observability.prometheus()
        && let Some(address) = prometheus.config().listen_address()
    {
        let listener = kerniq_runtime::bind(&address.to_string()).await?;
        let prometheus = prometheus.clone();
        background.spawn("prometheus-metrics", move |shutdown: Shutdown| async move {
            if let Err(err) = prometheus.serve(listener, shutdown.wait_owned()).await {
                tracing::error!(error = %err, "Prometheus metrics listener failed");
            }
        });
    }
    let _ = (observability, background);
    Ok(())
}

async fn not_found() -> Error {
    Error::not_found("No route matches the requested path")
}

async fn method_not_allowed() -> Error {
    Error::method_not_allowed("The route does not support this HTTP method")
}

impl<S> From<App<S>> for axum::Router
where
    S: Clone + Send + Sync + 'static,
{
    fn from(app: App<S>) -> Self {
        app.into_axum_router()
    }
}
