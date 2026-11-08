//! Core types shared by every Kerniq Server crate.
//!
//! This crate sits at the bottom of the dependency graph and contains only
//! what every other crate needs:
//!
//! * [`Error`] / [`Result`] — the framework error model and its JSON
//!   envelope ([`error`] module).
//! * [`ServerConfig`] — programmatic, environment and file configuration
//!   ([`config`] module).
//! * [`ServerError`] — failures while starting or running the server.
//!
//! Application code normally uses these through `kerniq_server::prelude`.

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod config;
pub mod error;

pub use config::{ConfigError, ServerConfig};
pub use error::{BoxError, Error, OptionExt, Result, ResultExt};

/// Errors raised while starting or running the HTTP server (as opposed to
/// errors produced while handling a request).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerError {
    /// The listen address could not be parsed or resolved.
    #[error("invalid listen address `{address}`: {reason}")]
    InvalidAddress {
        /// The address as given.
        address: String,
        /// Why it was rejected.
        reason: String,
    },
    /// Binding the TCP listener failed (address in use, permission denied...).
    #[error("failed to bind to `{address}`: {source}")]
    Bind {
        /// The address that could not be bound.
        address: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `run_blocking` was called from inside an active Tokio runtime.
    #[error(
        "`run_blocking` cannot be called from inside a Tokio runtime (it would create a nested \
         runtime). Inside `#[tokio::main]` or any async context, use `app.run(addr).await` instead"
    )]
    NestedRuntime,
    /// The Tokio runtime could not be created.
    #[error("failed to build the Tokio runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// An I/O error occurred on the listening socket.
    #[error("server I/O error: {0}")]
    Io(#[from] std::io::Error),
}
