//! Background task management.

use std::future::Future;
use std::time::Duration;

use tokio_util::task::TaskTracker;

use crate::Shutdown;

/// Spawns and tracks background tasks tied to the server lifecycle.
///
/// Tasks receive a [`Shutdown`] handle and are expected to return soon after
/// it triggers. During graceful shutdown the server waits (up to the
/// shutdown timeout) for every tracked task to finish.
///
/// This is intentionally small: it is not a job queue, it has no retries and
/// no persistence.
///
/// ```
/// use std::time::Duration;
/// use kerniq_runtime::{BackgroundTasks, Shutdown};
///
/// # async fn demo() {
/// let shutdown = Shutdown::new();
/// let tasks = BackgroundTasks::new(shutdown.clone());
/// tasks.spawn("ticker", |shutdown| async move {
///     let mut interval = tokio::time::interval(Duration::from_millis(10));
///     loop {
///         tokio::select! {
///             _ = interval.tick() => { /* periodic work */ }
///             () = shutdown.wait() => break,
///         }
///     }
/// });
/// shutdown.trigger();
/// assert!(tasks.shutdown(Duration::from_secs(1)).await);
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct BackgroundTasks {
    tracker: TaskTracker,
    shutdown: Shutdown,
}

impl BackgroundTasks {
    /// Creates a task set bound to `shutdown`.
    pub fn new(shutdown: Shutdown) -> Self {
        Self {
            tracker: TaskTracker::new(),
            shutdown,
        }
    }

    /// Spawns a named task on the current Tokio runtime.
    ///
    /// # Panics
    ///
    /// Panics if called outside a Tokio runtime.
    pub fn spawn<F, Fut>(&self, name: &'static str, task: F)
    where
        F: FnOnce(Shutdown) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let future = task(self.shutdown.clone());
        tracing::debug!(task = name, "background task started");
        self.tracker.spawn(async move {
            future.await;
            tracing::debug!(task = name, "background task finished");
        });
    }

    /// Number of tasks still running.
    pub fn len(&self) -> usize {
        self.tracker.len()
    }

    /// Whether no tasks are running.
    pub fn is_empty(&self) -> bool {
        self.tracker.is_empty()
    }

    /// Triggers shutdown and waits up to `timeout` for all tasks to finish.
    /// Returns `false` if the timeout elapsed first.
    pub async fn shutdown(&self, timeout: Duration) -> bool {
        self.shutdown.trigger();
        self.tracker.close();
        match tokio::time::timeout(timeout, self.tracker.wait()).await {
            Ok(()) => true,
            Err(_) => {
                tracing::warn!(
                    remaining = self.tracker.len(),
                    "background tasks did not finish before the shutdown timeout"
                );
                false
            }
        }
    }
}
