//! Routing for Kerniq Server.
//!
//! Kerniq does not have its own routing engine: every route ends up in an
//! [`axum::Router`], and requests are matched by Axum's `matchit`-based
//! router. This crate adds the developer-facing layer on top:
//!
//! * [`Router`] — Flask-style registration, prefixes, groups and mounting,
//! * [`Group`] — block-style route registration,
//! * [`middleware`] — deferred, order-preserving middleware composition,
//! * [`path`] — normalization of `:id` / `{id}` path syntax,
//! * [`methods`] — Axum's method-router constructors for advanced routes.

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod middleware;
pub mod path;
mod router;

pub use middleware::{Middleware, MiddlewareStack, TowerLayer};
pub use router::{Group, RouteInfo, Router};

/// Axum's method-router constructors, for use with [`Router::route`].
///
/// ```
/// use kerniq_routing::{Router, methods};
/// # async fn list() {} async fn create() {}
/// let router: Router = Router::new().route("/users", methods::get(list).post(create));
/// ```
pub mod methods {
    pub use axum::routing::{
        MethodFilter, MethodRouter, any, delete, get, head, on, options, patch, post, put, trace,
    };
}
