//! # Kerniq Server
//!
//! A lightweight, asynchronous web framework for Rust: the approachability
//! of Flask on top of Axum, Tower, Hyper and Tokio.
//!
//! ```no_run
//! use kerniq_server::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     App::new()
//!         .get("/", || async { "Hello from Kerniq" })
//!         .run("127.0.0.1:3000")
//!         .await
//! }
//! ```
//!
//! ## What Kerniq adds, and what it reuses
//!
//! Kerniq is a thin ergonomic layer. Routing, extraction and HTTP are
//! Axum's; middleware is Tower's; the protocol is Hyper's; execution is
//! Tokio's. Kerniq contributes:
//!
//! * the [`App`] builder: Flask-style route registration, typed state,
//!   configuration, lifecycle (graceful shutdown, background tasks,
//!   connection and concurrency limits) in one place,
//! * [`Router`](routing::Router) with prefixes, groups and mounting,
//! * middleware that applies to every route regardless of declaration order,
//!   with first-declared-is-outermost ordering,
//! * one JSON error envelope for application errors, malformed input,
//!   404/405, timeouts, rate limiting and authentication failures,
//! * production middleware (logging, request IDs, CORS, compression,
//!   timeouts, rate limiting, auth extension points, metrics),
//! * optional OpenAPI, validation and WebSocket support.
//!
//! Handlers are plain Axum handlers, and the app converts to an
//! [`axum::Router`] with [`App::into_axum_router`], so the Axum and Tower
//! ecosystems stay fully available.
//!
//! ## Crate features
//!
//! | Feature | Default | Enables |
//! |---|---|---|
//! | `json` | ✓ | `Json<T>`, `Created<T>` |
//! | `tracing` | ✓ | `Logger` middleware |
//! | `cors` | | `Cors` middleware |
//! | `compression` | | `Compression` middleware |
//! | `websocket` | | [`extract::ws`] |
//! | `openapi` | | `openapi` module (Utoipa + Swagger UI) |
//! | `validation` | | `ValidatedJson`, `ValidatedQuery` |
//! | `config-file` | | `ServerConfig::from_file` (TOML) |
//! | `macros` | | [`authorize`] attribute for role/permission checks |
//! | `full` | | all of the above |

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod builder;
pub mod prelude;

pub use builder::App;
pub use kerniq_core::{Error, Result};

#[cfg(feature = "macros")]
pub use kerniq_macros::authorize;

/// Re-export of Axum, for interoperability with Axum handlers, routers and
/// extractors at a guaranteed-compatible version.
pub use axum;
/// Re-export of `http`.
pub use http;
/// Re-export of Tower.
pub use tower;

/// The error model. See [`Error`].
pub mod error {
    pub use kerniq_core::error::*;
}

/// Server configuration. See [`ServerConfig`](config::ServerConfig).
pub mod config {
    pub use kerniq_core::config::*;
}

/// Request extractors.
pub mod extract {
    pub use kerniq_http::extractors::*;
    #[cfg(feature = "websocket")]
    pub use kerniq_http::ws;
    pub use kerniq_http::{
        Body, Bytes, ConnectInfo, Extension, HeaderMap, MatchedPath, OriginalUri, Request, State,
    };
    #[cfg(feature = "validation")]
    pub use kerniq_http::{ValidatedJson, ValidatedQuery, validation_error};
}

/// Response types.
pub mod response {
    pub use kerniq_http::response::*;
}

/// Routers, groups and method routers.
pub mod routing {
    pub use kerniq_routing::path;
    pub use kerniq_routing::{Group, RouteInfo, Router, methods};
}

/// Middleware: the composition traits and the built-in middleware.
pub mod middleware {
    pub use kerniq_middleware::*;
    pub use kerniq_routing::{Middleware, MiddlewareStack, TowerLayer};
}

/// Server lifecycle: shutdown handles, background tasks and runtime helpers.
pub mod runtime {
    pub use kerniq_runtime::runtime::in_runtime;
    pub use kerniq_runtime::{BackgroundTasks, ServeOptions, Shutdown, os_signal};
}

/// OpenAPI documents and Swagger UI (feature `openapi`).
#[cfg(feature = "openapi")]
pub mod openapi {
    pub use kerniq_openapi::*;
}

/// Compiles every Rust snippet in the README and the guides in `docs/`, so
/// documentation cannot drift from the API. Pages that need optional
/// features are only checked when those features are enabled.
#[cfg(doctest)]
mod documentation {
    macro_rules! doc_page {
        ($name:ident, $path:literal) => {
            #[doc = include_str!(concat!("../../../", $path))]
            struct $name;
        };
        ($name:ident, $path:literal, $($feature:literal),+) => {
            #[cfg(all($(feature = $feature),+))]
            #[doc = include_str!(concat!("../../../", $path))]
            struct $name;
        };
    }

    doc_page!(Readme, "README.md", "json", "tracing");
    doc_page!(GettingStarted, "docs/getting-started.md", "json");
    doc_page!(Routing, "docs/routing.md");
    doc_page!(Handlers, "docs/handlers.md", "json");
    doc_page!(Extractors, "docs/extractors.md", "json", "validation");
    doc_page!(StateManagement, "docs/state-management.md", "json");
    doc_page!(Middleware, "docs/middleware.md", "tracing", "cors");
    doc_page!(Authorization, "docs/authorization.md", "macros", "json");
    doc_page!(AxumInterop, "docs/axum-interop.md", "json");
    doc_page!(Runtime, "docs/runtime.md");
    doc_page!(Websockets, "docs/websockets.md", "websocket");
    doc_page!(Configuration, "docs/configuration.md", "config-file");
    doc_page!(Integrations, "docs/integrations.md");
    doc_page!(OpenApi, "docs/openapi.md", "openapi", "json");
    doc_page!(Performance, "docs/performance.md");
    doc_page!(Architecture, "docs/architecture.md");
}
