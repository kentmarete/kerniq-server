//! Shutdown coordination.

use std::future::Future;

use tokio_util::sync::CancellationToken;

/// A cloneable handle used to request and observe server shutdown.
///
/// Every clone refers to the same underlying signal. The server stops
/// accepting connections once it is triggered, background tasks receive it,
/// and long-lived handlers (WebSockets, streaming responses) should watch it
/// to finish cleanly:
///
/// ```
/// use kerniq_runtime::Shutdown;
///
/// # async fn demo() {
/// let shutdown = Shutdown::new();
/// let watcher = shutdown.clone();
/// let task = tokio::spawn(async move {
///     watcher.wait().await;
///     "stopped"
/// });
/// shutdown.trigger();
/// assert_eq!(task.await.unwrap(), "stopped");
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct Shutdown {
    token: CancellationToken,
}

impl Shutdown {
    /// Creates a new, untriggered handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests shutdown. Idempotent.
    pub fn trigger(&self) {
        self.token.cancel();
    }

    /// Whether shutdown has been requested.
    pub fn is_triggered(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Completes once shutdown has been requested. Cancel-safe.
    pub async fn wait(&self) {
        self.token.cancelled().await;
    }

    /// An owned future that completes on shutdown (useful for `select!` in
    /// `'static` contexts).
    pub fn wait_owned(&self) -> impl Future<Output = ()> + Send + 'static {
        self.token.clone().cancelled_owned()
    }

    /// The underlying Tokio cancellation token, for interoperability.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl From<CancellationToken> for Shutdown {
    fn from(token: CancellationToken) -> Self {
        Self { token }
    }
}

/// Completes when the process receives Ctrl+C (SIGINT) or, on Unix, SIGTERM.
///
/// If a signal handler cannot be installed the failure is logged and that
/// signal is ignored (the future then waits on the remaining signals).
pub async fn os_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %err, "failed to listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received Ctrl+C, shutting down"),
        () = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}
