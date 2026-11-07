//! Server lifecycle for Kerniq Server, built on Tokio and Hyper.
//!
//! * [`server`] — the accept loop with connection limits and bounded
//!   graceful drain,
//! * [`Shutdown`] / [`os_signal`] — shutdown coordination,
//! * [`BackgroundTasks`] — tasks tied to the server lifecycle,
//! * [`runtime`] — the rules for who owns the Tokio runtime.
//!
//! Kerniq does not implement an executor or an HTTP protocol: Tokio and
//! Hyper do that work.

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod runtime;
pub mod server;
mod shutdown;
mod tasks;

pub use server::{ServeOptions, bind, serve};
pub use shutdown::{Shutdown, os_signal};
pub use tasks::BackgroundTasks;
