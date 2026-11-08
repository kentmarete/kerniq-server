//! Prometheus scraping for Kerniq Server.
//!
//! This crate is configuration over two existing libraries: the
//! [`opentelemetry-prometheus`](https://docs.rs/opentelemetry-prometheus)
//! exporter (an OpenTelemetry `MetricReader` that collects on demand) and the
//! [`prometheus`](https://docs.rs/prometheus) crate's text encoder. Kerniq
//! adds the HTTP endpoint and, optionally, a dedicated listener.
//!
//! ```
//! use kerniq_observability_prometheus::{PrometheusConfig, PrometheusExporter};
//! use opentelemetry::metrics::MeterProvider;
//! use opentelemetry_sdk::metrics::SdkMeterProvider;
//!
//! let (reader, exporter) = PrometheusExporter::build(PrometheusConfig::default()).unwrap();
//! let provider = SdkMeterProvider::builder().with_reader(reader).build();
//! provider.meter("demo").u64_counter("jobs").build().add(1, &[]);
//!
//! let text = exporter.render().unwrap();
//! assert!(text.contains("jobs_total"));
//!
//! // Serve it: merge into the application, or bind a separate listener.
//! let router: axum::Router = exporter.router();
//! ```
//!
//! # Cost model
//!
//! Nothing runs between scrapes: there is no background task and no copy of
//! the metrics. Each scrape asks the OpenTelemetry SDK to aggregate its
//! current state and encodes it on a blocking-pool thread, so encoding never
//! occupies an async worker that is serving requests.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::future::Future;
use std::net::SocketAddr;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use prometheus::{Encoder, TextEncoder};

pub use opentelemetry_prometheus::PrometheusExporter as PrometheusReader;

/// Errors from building or rendering the exporter.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The OpenTelemetry Prometheus exporter could not be built.
    #[error("failed to build the Prometheus exporter: {0}")]
    Build(String),
    /// Encoding the scrape failed.
    #[error("failed to encode metrics: {0}")]
    Encode(#[from] prometheus::Error),
    /// The metrics listener could not be bound or served.
    #[error("metrics listener error: {0}")]
    Io(#[from] std::io::Error),
}

/// Where and how metrics are exposed.
#[derive(Debug, Clone)]
#[must_use]
pub struct PrometheusConfig {
    path: String,
    listen: Option<SocketAddr>,
    scope_labels: bool,
    target_info: bool,
    namespace: Option<String>,
    resource_labels: Vec<String>,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            path: "/metrics".to_owned(),
            listen: None,
            scope_labels: false,
            target_info: true,
            namespace: None,
            resource_labels: vec!["service.name".to_owned()],
        }
    }
}

impl PrometheusConfig {
    /// `/metrics` on the application listener.
    pub fn new() -> Self {
        Self::default()
    }

    /// The scrape path. Default `/metrics`.
    ///
    /// # Panics
    ///
    /// Panics if `path` does not start with `/`.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        assert!(path.starts_with('/'), "metrics path must start with '/'");
        self.path = path;
        self
    }

    /// Serves metrics on a separate listener instead of the application's.
    /// Recommended in production: the port can be firewalled to the
    /// Prometheus servers, and scrapes never share a connection limit or
    /// middleware with user traffic.
    pub fn listen(mut self, address: SocketAddr) -> Self {
        self.listen = Some(address);
        self
    }

    /// Adds `otel_scope_name`/`otel_scope_version` labels to every series.
    /// Off by default: they add a label to every series without helping
    /// most dashboards.
    pub fn scope_labels(mut self, enabled: bool) -> Self {
        self.scope_labels = enabled;
        self
    }

    /// Emits the `target_info` series carrying resource attributes. Default
    /// on; join it in PromQL to filter by service or environment.
    pub fn target_info(mut self, enabled: bool) -> Self {
        self.target_info = enabled;
        self
    }

    /// Resource attributes copied onto every series as labels (dots become
    /// underscores). Default: `service.name`, so dashboards can filter by
    /// `service_name` both here and behind an OpenTelemetry Collector with
    /// `resource_to_telemetry_conversion`. Each entry is one constant label
    /// per process and does not increase cardinality.
    pub fn resource_labels<I, K>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        self.resource_labels = keys.into_iter().map(Into::into).collect();
        self
    }

    /// Prefixes every metric name with `namespace_`.
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// The configured scrape path.
    pub fn scrape_path(&self) -> &str {
        &self.path
    }

    /// The separate listener address, if any.
    pub fn listen_address(&self) -> Option<SocketAddr> {
        self.listen
    }
}

/// A handle that renders and serves the metrics collected by the paired
/// [`PrometheusReader`]. Cheap to clone.
#[derive(Debug, Clone)]
pub struct PrometheusExporter {
    registry: prometheus::Registry,
    config: PrometheusConfig,
}

impl PrometheusExporter {
    /// Builds the reader (register it with an `SdkMeterProvider`) and the
    /// handle that renders it.
    pub fn build(config: PrometheusConfig) -> Result<(PrometheusReader, Self), Error> {
        let registry = prometheus::Registry::new();
        let mut builder = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .scope_info_enabled(config.scope_labels);
        if !config.target_info {
            builder = builder.without_target_info();
        }
        if !config.resource_labels.is_empty() {
            let keys: std::collections::HashSet<opentelemetry::Key> = config
                .resource_labels
                .iter()
                .map(|k| opentelemetry::Key::new(k.clone()))
                .collect();
            builder = builder.with_resource_selector(keys);
        }
        if let Some(ns) = &config.namespace {
            builder = builder.with_namespace(ns.clone());
        }
        let reader = builder.build().map_err(|e| Error::Build(e.to_string()))?;
        Ok((reader, Self { registry, config }))
    }

    /// The configuration this exporter was built with.
    pub fn config(&self) -> &PrometheusConfig {
        &self.config
    }

    /// Collects and encodes the current metrics in the Prometheus text
    /// format (0.0.4). Synchronous and CPU-bound; prefer [`render_async`]
    /// from async code.
    ///
    /// [`render_async`]: Self::render_async
    pub fn render(&self) -> Result<String, Error> {
        let families = self.registry.gather();
        let mut buffer = Vec::with_capacity(16 * 1024);
        TextEncoder::new().encode(&families, &mut buffer)?;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }

    /// [`render`](Self::render) on Tokio's blocking pool.
    pub async fn render_async(&self) -> Result<String, Error> {
        let this = self.clone();
        match tokio::task::spawn_blocking(move || this.render()).await {
            Ok(result) => result,
            Err(join) => Err(Error::Io(std::io::Error::other(join))),
        }
    }

    /// A router answering `GET <path>`. It carries no Kerniq middleware, so
    /// merging it after the application's middleware has been applied keeps
    /// scrapes out of the HTTP metrics.
    pub fn router<S>(&self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        let exporter = self.clone();
        Router::new().route(
            &self.config.path,
            get(move || {
                let exporter = exporter.clone();
                async move { exporter.scrape().await }
            }),
        )
    }

    async fn scrape(&self) -> Response {
        match self.render_async().await {
            Ok(body) => (
                [(
                    header::CONTENT_TYPE,
                    TextEncoder::new().format_type().to_owned(),
                )],
                body,
            )
                .into_response(),
            Err(err) => {
                tracing::error!(error = %err, "failed to render Prometheus metrics");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }

    /// Serves the metrics endpoint on `listener` until `shutdown` completes.
    /// Used for [`PrometheusConfig::listen`]; usable directly for custom
    /// setups.
    pub async fn serve(
        &self,
        listener: tokio::net::TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), Error> {
        let address = listener.local_addr()?;
        tracing::info!(%address, path = %self.config.path, "Prometheus metrics listener started");
        axum::serve(listener, self.router::<()>())
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}
