//! Production middleware for Kerniq Server.
//!
//! Every item here is ordinary Tower middleware, mostly thin configuration
//! over `tower` and `tower-http`. Each implements Kerniq's
//! [`Middleware`](kerniq_routing::Middleware) trait so it can be passed to
//! `App::middleware` / `Router::middleware`, and most also expose
//! `into_layer()` for use with plain Axum or `tower::ServiceBuilder`.
//!
//! | Middleware | Built on | Feature |
//! |---|---|---|
//! | [`Logger`] | `tower_http::trace` | `tracing` |
//! | [`RequestId`] | `tower_http::request_id` | — |
//! | [`Timeout`] | `tower::timeout` + `HandleErrorLayer` | — |
//! | [`BodyLimit`] | `axum::extract::DefaultBodyLimit` | — |
//! | [`ConcurrencyLimit`] | `tower::limit::GlobalConcurrencyLimitLayer` | — |
//! | [`Cors`] | `tower_http::cors` | `cors` |
//! | [`Compression`] | `tower_http::compression` | `compression` |
//! | [`SecurityHeaders`] | Kerniq (single header-setting layer) | — |
//! | [`RateLimit`] | Kerniq (sharded token buckets) | — |
//! | [`Auth`](auth::Auth) | Kerniq (extension point, no provider) | — |
//! | [`Require`](auth::Require) | Kerniq (role/permission rules per router) | — |
//! | [`Metrics`] | Kerniq (Prometheus text format) | — |
//!
//! # Fusing middleware on hot paths
//!
//! Each `.middleware(..)` call is one Axum `Router::layer`, and every
//! `Router::layer` boxes the route once more (roughly two small allocations
//! per request). When that matters, fuse several middleware into a single
//! layer with Tower's `ServiceBuilder`; every built-in middleware exposes
//! `into_layer()` for this:
//!
//! ```
//! use kerniq_middleware::{RequestId, SecurityHeaders, Timeout};
//! use tower::ServiceBuilder;
//!
//! let fused = ServiceBuilder::new()
//!     .layer(RequestId::new().into_layer())      // outermost
//!     .layer(SecurityHeaders::new().into_layer())
//!     .layer(Timeout::seconds(30).into_layer());  // innermost
//! // app.layer(fused)
//! ```
//!
//! Errors produced by middleware (timeouts, rate limiting, failed
//! authentication) use the standard Kerniq JSON error envelope.

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod auth;
#[cfg(feature = "compression")]
mod compression;
#[cfg(feature = "cors")]
mod cors;
mod limit;
#[cfg(feature = "tracing")]
pub mod logging;
pub mod metrics;
pub mod rate_limit;
mod request_id;
pub mod security;
pub mod timeout;

#[cfg(feature = "compression")]
pub use compression::Compression;
#[cfg(feature = "cors")]
pub use cors::Cors;
pub use limit::{BodyLimit, ConcurrencyLimit};
#[cfg(feature = "tracing")]
pub use logging::Logger;
pub use metrics::{Metrics, MetricsRegistry};
pub use rate_limit::RateLimit;
pub use request_id::{RequestId, RequestIdLayer};
pub use security::SecurityHeaders;
pub use timeout::{Timeout, TimeoutLayer};
