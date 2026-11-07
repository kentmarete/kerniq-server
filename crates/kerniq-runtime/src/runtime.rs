//! Tokio runtime ownership.
//!
//! Kerniq never creates a runtime behind the user's back. There are exactly
//! two modes:
//!
//! * **User-managed** (`app.run(addr).await`): the application owns the
//!   runtime, typically through `#[tokio::main]`. Kerniq only spawns tasks
//!   on it.
//! * **Framework-managed** (`app.run_blocking(addr)`): Kerniq builds a
//!   multi-threaded runtime, runs the server to completion and shuts the
//!   runtime down. This is refused with [`ServerError::NestedRuntime`] when
//!   called from inside an existing runtime, because Tokio cannot nest
//!   runtimes (it would panic).

use std::future::Future;
use std::time::Duration;

use kerniq_core::ServerError;

/// Whether the current thread is inside a Tokio runtime context.
pub fn in_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok()
}

/// Builds a multi-threaded runtime, drives `future` to completion and shuts
/// the runtime down, waiting at most `shutdown_timeout` for stray tasks.
///
/// `workers` sets the number of worker threads (default: one per core).
///
/// # Errors
///
/// [`ServerError::NestedRuntime`] if called from inside a runtime, or
/// [`ServerError::Runtime`] if the runtime cannot be built.
pub fn block_on<F: Future>(
    workers: Option<usize>,
    shutdown_timeout: Duration,
    future: F,
) -> Result<F::Output, ServerError> {
    if in_runtime() {
        return Err(ServerError::NestedRuntime);
    }
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all().thread_name("kerniq-worker");
    if let Some(workers) = workers {
        builder.worker_threads(workers);
    }
    let runtime = builder.build().map_err(ServerError::Runtime)?;
    let output = runtime.block_on(future);
    runtime.shutdown_timeout(shutdown_timeout);
    Ok(output)
}
