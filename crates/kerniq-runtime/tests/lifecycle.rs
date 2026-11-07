use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ConnectInfo;
use axum::routing::get;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use kerniq_core::ServerError;
use kerniq_runtime::{BackgroundTasks, ServeOptions, Shutdown, bind, runtime, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn router() -> Router {
    Router::new()
        .route("/", get(|| async { "ok" }))
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                "slow ok"
            }),
        )
        .route(
            "/hang",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                "never"
            }),
        )
        .route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        )
}

async fn start(options: ServeOptions) -> (SocketAddr, Shutdown, tokio::task::JoinHandle<()>) {
    let listener = bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = Shutdown::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { serve(listener, router(), options, shutdown).await.unwrap() }
    });
    (addr, shutdown, handle)
}

async fn get_text(addr: SocketAddr, path: &str) -> Result<(u16, String), String> {
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<Bytes>>();
    let uri = format!("http://{addr}{path}").parse().unwrap();
    let response = client.get(uri).await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

#[tokio::test]
async fn serves_requests_and_connect_info() {
    let (addr, shutdown, handle) = start(ServeOptions::default()).await;
    assert_eq!(get_text(addr, "/").await.unwrap(), (200, "ok".into()));
    assert_eq!(
        get_text(addr, "/peer").await.unwrap(),
        (200, "127.0.0.1".into())
    );
    shutdown.trigger();
    handle.await.unwrap();
}

#[tokio::test]
async fn graceful_shutdown_lets_in_flight_requests_finish() {
    let (addr, shutdown, handle) = start(ServeOptions::default()).await;

    let in_flight = tokio::spawn(async move { get_text(addr, "/slow").await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    shutdown.trigger();

    let result = in_flight.await.unwrap();
    assert_eq!(result.unwrap(), (200, "slow ok".into()));
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("server should stop after draining")
        .unwrap();

    // The listener is closed: new connections are refused.
    assert!(TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn shutdown_timeout_force_closes_stuck_connections() {
    let options = ServeOptions {
        shutdown_timeout: Duration::from_millis(150),
        ..ServeOptions::default()
    };
    let (addr, shutdown, handle) = start(options).await;

    let stuck = tokio::spawn(async move { get_text(addr, "/hang").await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    let started = Instant::now();
    shutdown.trigger();

    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("server should stop after the shutdown timeout")
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    let result = tokio::time::timeout(Duration::from_secs(5), stuck)
        .await
        .expect("client should observe the closed connection")
        .unwrap();
    assert!(
        result.is_err(),
        "stuck request must not succeed: {result:?}"
    );
}

#[tokio::test]
async fn max_connections_limits_open_connections() {
    let options = ServeOptions {
        max_connections: Some(1),
        ..ServeOptions::default()
    };
    let (addr, shutdown, handle) = start(options).await;

    // Occupy the only slot with an idle keep-alive connection.
    let mut first = TcpStream::connect(addr).await.unwrap();
    first
        .write_all(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 256];
    let n = first.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));

    // A second connection is not served while the first is open.
    let blocked = tokio::time::timeout(Duration::from_millis(300), get_text(addr, "/")).await;
    assert!(blocked.is_err(), "second connection should wait for a slot");

    // Closing the first connection frees the slot.
    drop(first);
    let served = tokio::time::timeout(Duration::from_secs(5), get_text(addr, "/"))
        .await
        .expect("second connection should be served once a slot frees up");
    assert_eq!(served.unwrap(), (200, "ok".into()));

    shutdown.trigger();
    handle.await.unwrap();
}

#[tokio::test]
async fn bind_reports_address_in_use() {
    let listener = bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let err = bind(&addr).await.unwrap_err();
    assert!(matches!(err, ServerError::Bind { .. }), "{err}");
    let err = bind("not an address").await.unwrap_err();
    assert!(
        matches!(
            err,
            ServerError::InvalidAddress { .. } | ServerError::Bind { .. }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn block_on_refuses_nested_runtime() {
    let err = runtime::block_on(None, Duration::from_secs(1), async {}).unwrap_err();
    assert!(matches!(err, ServerError::NestedRuntime));
    assert!(err.to_string().contains(".run(addr).await"));
}

#[test]
fn block_on_creates_runtime_outside_tokio() {
    let value = runtime::block_on(Some(2), Duration::from_secs(1), async {
        tokio::task::spawn(async { 21 * 2 }).await.unwrap()
    })
    .unwrap();
    assert_eq!(value, 42);
}

#[tokio::test]
async fn background_tasks_stop_on_shutdown() {
    let shutdown = Shutdown::new();
    let tasks = BackgroundTasks::new(shutdown.clone());
    let (tx, rx) = tokio::sync::oneshot::channel();
    tasks.spawn("waiter", |shutdown| async move {
        shutdown.wait().await;
        tx.send("cleaned up").unwrap();
    });
    assert_eq!(tasks.len(), 1);
    assert!(tasks.shutdown(Duration::from_secs(1)).await);
    assert_eq!(rx.await.unwrap(), "cleaned up");
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn background_task_shutdown_times_out() {
    let tasks = BackgroundTasks::new(Shutdown::new());
    tasks.spawn("stubborn", |_shutdown| async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    assert!(!tasks.shutdown(Duration::from_millis(50)).await);
}

#[tokio::test]
async fn header_read_timeout_closes_slow_clients_and_can_be_disabled() {
    async fn idle_connection_closed(options: ServeOptions) -> bool {
        let (addr, shutdown, handle) = start(options).await;
        let mut stream = TcpStream::connect(addr).await.unwrap();
        // Send part of a request line and then stall.
        stream.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
        let mut buf = [0u8; 512];
        let closed = matches!(
            tokio::time::timeout(Duration::from_millis(1500), stream.read(&mut buf)).await,
            Ok(Ok(_)) | Ok(Err(_))
        );
        shutdown.trigger();
        drop(stream);
        handle.await.unwrap();
        closed
    }
    let protected = ServeOptions {
        header_read_timeout: Duration::from_millis(200),
        shutdown_timeout: Duration::from_millis(100),
        ..ServeOptions::default()
    };
    assert!(
        idle_connection_closed(protected).await,
        "slow client should be cut off"
    );
    let unprotected = ServeOptions {
        header_read_timeout: Duration::ZERO,
        shutdown_timeout: Duration::from_millis(100),
        ..ServeOptions::default()
    };
    assert!(
        !idle_connection_closed(unprotected).await,
        "timeout disabled"
    );
}
