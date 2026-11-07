//! The HTTP accept loop.
//!
//! Kerniq drives Hyper directly (through `hyper-util`'s HTTP/1 + HTTP/2
//! auto-detecting connection builder) instead of using `axum::serve`,
//! because it needs three things `axum::serve` does not offer:
//!
//! * a **connection limit** (`max_connections`),
//! * a **bounded drain**: in-flight connections get `shutdown_timeout` to
//!   finish, after which they are closed,
//! * control over the header read timeout.
//!
//! The per-request path is identical to `axum::serve`: each connection gets a
//! clone of the `axum::Router` (an `Arc` bump) and requests are dispatched
//! straight into it.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use http::Request;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use kerniq_core::{BoxError, ServerConfig, ServerError};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::Shutdown;

/// Connection-level options, usually derived from a [`ServerConfig`].
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Maximum number of open connections (`None` = unlimited).
    pub max_connections: Option<usize>,
    /// How long to wait for in-flight connections during shutdown.
    pub shutdown_timeout: Duration,
    /// Maximum time to receive request headers on HTTP/1 connections
    /// (`Duration::ZERO` disables the timeout).
    pub header_read_timeout: Duration,
    /// Set `TCP_NODELAY` on accepted sockets.
    pub tcp_nodelay: bool,
    /// Insert `ConnectInfo<SocketAddr>` into each request.
    pub connect_info: bool,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self::from(&ServerConfig::default())
    }
}

impl From<&ServerConfig> for ServeOptions {
    fn from(config: &ServerConfig) -> Self {
        Self {
            max_connections: config.max_connections,
            shutdown_timeout: config.shutdown_timeout,
            header_read_timeout: config.header_read_timeout,
            tcp_nodelay: config.tcp_nodelay,
            connect_info: config.connect_info,
        }
    }
}

/// Binds a TCP listener. Host names are resolved; the first address that
/// binds successfully is used.
pub async fn bind(address: &str) -> Result<TcpListener, ServerError> {
    if address.trim().is_empty() {
        return Err(ServerError::InvalidAddress {
            address: address.to_owned(),
            reason: "address is empty".to_owned(),
        });
    }
    TcpListener::bind(address)
        .await
        .map_err(|source| match source.kind() {
            io::ErrorKind::InvalidInput => ServerError::InvalidAddress {
                address: address.to_owned(),
                reason: source.to_string(),
            },
            _ => ServerError::Bind {
                address: address.to_owned(),
                source,
            },
        })
}

/// Serves `router` on `listener` until `shutdown` is triggered, then drains
/// in-flight connections for up to `options.shutdown_timeout`.
///
/// Shutdown sequence:
///
/// 1. stop accepting (the listener is dropped, so new connections are
///    refused by the OS),
/// 2. ask every open connection to finish: HTTP/1 connections close after
///    their in-flight response, HTTP/2 connections send `GOAWAY`,
/// 3. after `shutdown_timeout`, forcibly close whatever is left.
pub async fn serve(
    listener: TcpListener,
    router: Router,
    options: ServeOptions,
    shutdown: Shutdown,
) -> Result<(), ServerError> {
    let local_addr = listener.local_addr()?;
    let limiter = options.max_connections.map(|n| Arc::new(Semaphore::new(n)));
    let graceful = GracefulShutdown::new();
    let force_close = CancellationToken::new();

    let mut builder = Builder::new(TokioExecutor::new());
    // The header read timeout protects against slow-header (slowloris)
    // clients. It arms a timer for every request on a keep-alive connection,
    // which measurably costs throughput on trivial endpoints (see
    // docs/performance.md), so it can be disabled with a zero duration.
    if !options.header_read_timeout.is_zero() {
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(options.header_read_timeout);
        builder.http2().timer(TokioTimer::new());
    }

    tracing::info!(address = %local_addr, "kerniq server listening");

    loop {
        let permit = match &limiter {
            Some(semaphore) => tokio::select! {
                biased;
                () = shutdown.wait() => break,
                permit = semaphore.clone().acquire_owned() => match permit {
                    Ok(permit) => Some(permit),
                    Err(_) => break, // semaphore closed: cannot happen, but stop cleanly
                },
            },
            None => None,
        };

        let (stream, peer) = tokio::select! {
            biased;
            () = shutdown.wait() => break,
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(err) => {
                    handle_accept_error(err).await;
                    continue;
                }
            },
        };

        if options.tcp_nodelay
            && let Err(err) = stream.set_nodelay(true)
        {
            tracing::debug!(error = %err, "failed to set TCP_NODELAY");
        }

        let connection = serve_connection(
            &builder,
            &graceful,
            stream,
            peer,
            router.clone(),
            options.connect_info,
        );
        let force_close = force_close.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = connection => {
                    if let Err(err) = result {
                        tracing::trace!(peer = %peer, error = %err, "connection closed with error");
                    }
                }
                () = force_close.cancelled() => {
                    tracing::debug!(peer = %peer, "connection force-closed after shutdown timeout");
                }
            }
            drop(permit);
        });
    }

    drop(listener);
    let open = graceful.count();
    tracing::info!(
        open_connections = open,
        "stopped accepting connections, draining"
    );

    match tokio::time::timeout(options.shutdown_timeout, graceful.shutdown()).await {
        Ok(()) => tracing::info!("all connections drained"),
        Err(_) => {
            tracing::warn!(
                timeout = ?options.shutdown_timeout,
                "shutdown timeout elapsed, closing remaining connections"
            );
            force_close.cancel();
        }
    }
    Ok(())
}

fn serve_connection(
    builder: &Builder<TokioExecutor>,
    graceful: &GracefulShutdown,
    stream: TcpStream,
    peer: SocketAddr,
    router: Router,
    connect_info: bool,
) -> impl Future<Output = Result<(), BoxError>> + Send + 'static {
    let service = router.map_request(move |mut request: Request<Incoming>| {
        if connect_info {
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        request
    });
    let connection = builder
        .serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(service))
        .into_owned();
    graceful.watch(connection)
}

/// Errors on `accept` are either per-connection (the peer went away) or
/// resource exhaustion (EMFILE). The former are ignored; for the latter we
/// back off briefly so a full file-descriptor table does not spin the loop.
async fn handle_accept_error(err: io::Error) {
    if matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    ) {
        return;
    }
    tracing::error!(error = %err, "failed to accept connection");
    tokio::time::sleep(Duration::from_millis(50)).await;
}
