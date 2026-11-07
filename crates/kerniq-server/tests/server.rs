//! End-to-end tests over real TCP connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use kerniq_server::prelude::*;
use tokio::net::TcpListener;

async fn fetch(addr: SocketAddr, path: &str) -> std::result::Result<(u16, String), String> {
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<Bytes>>();
    let response = client
        .get(format!("http://{addr}{path}").parse().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

async fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

#[tokio::test]
async fn serves_over_tcp_and_shuts_down_gracefully() {
    let (listener, addr) = listener().await;
    let app = App::new()
        .get("/", || async { "Hello from Kerniq" })
        .get("/slow", || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            "done"
        });
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(app.serve(listener));

    assert_eq!(
        fetch(addr, "/").await.unwrap(),
        (200, "Hello from Kerniq".into())
    );

    let in_flight = tokio::spawn(fetch(addr, "/slow"));
    tokio::time::sleep(Duration::from_millis(80)).await;
    shutdown.trigger();
    assert_eq!(in_flight.await.unwrap().unwrap(), (200, "done".into()));

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server stops")
        .unwrap()
        .unwrap();
    assert!(
        fetch(addr, "/").await.is_err(),
        "no new connections after shutdown"
    );
}

#[tokio::test]
async fn custom_shutdown_signal_and_background_tasks() {
    let (listener, addr) = listener().await;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let ticks = Arc::new(AtomicU64::new(0));
    let cleaned = Arc::new(AtomicU64::new(0));

    let app = App::new()
        .get("/", || async { "ok" })
        .shutdown_signal(async {
            let _ = stop_rx.await;
        })
        .background_task("ticker", {
            let ticks = ticks.clone();
            let cleaned = cleaned.clone();
            move |shutdown: Shutdown| async move {
                let mut interval = tokio::time::interval(Duration::from_millis(5));
                loop {
                    tokio::select! {
                        _ = interval.tick() => { ticks.fetch_add(1, Ordering::SeqCst); }
                        () = shutdown.wait() => break,
                    }
                }
                cleaned.fetch_add(1, Ordering::SeqCst);
            }
        });
    let server = tokio::spawn(app.serve(listener));
    assert_eq!(fetch(addr, "/").await.unwrap().0, 200);
    tokio::time::sleep(Duration::from_millis(40)).await;

    stop_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(ticks.load(Ordering::SeqCst) > 0, "background task ran");
    assert_eq!(
        cleaned.load(Ordering::SeqCst),
        1,
        "background task finished cleanly"
    );
}

#[tokio::test]
async fn bind_errors_are_reported() {
    let (_held, addr) = listener().await;
    let err = App::new().run(addr.to_string()).await.unwrap_err();
    assert!(err.to_string().contains("failed to bind"), "{err}");
}

#[tokio::test]
async fn run_blocking_inside_runtime_is_an_error() {
    let err = App::new()
        .get("/", || async { "x" })
        .run_blocking("127.0.0.1:0")
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot be called from inside a Tokio runtime"),
        "{err}"
    );
}

#[test]
fn run_blocking_owns_its_runtime() {
    // Find a free port, release it, then let run_blocking bind it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let app = App::new()
        .config(ServerConfig::new().workers(2))
        .get("/", || async { "blocking ok" });
    let shutdown = app.shutdown_handle();
    let server = std::thread::spawn(move || app.run_blocking(format!("127.0.0.1:{port}")));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let body = rt.block_on(async {
        for _ in 0..100 {
            if let Ok(res) = fetch(addr, "/").await {
                return res;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("server did not start");
    });
    assert_eq!(body, (200, "blocking ok".into()));
    shutdown.trigger();
    server.join().unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_over_tcp() {
    #[derive(Clone)]
    struct Counter(Arc<AtomicU64>);
    let (listener, addr) = listener().await;
    let counter = Counter(Arc::new(AtomicU64::new(0)));
    let app = App::new()
        .config(
            ServerConfig::new()
                .max_concurrent_requests(16)
                .request_timeout(Duration::from_secs(5)),
        )
        .state(counter.clone())
        .get("/inc", |State(Counter(c)): State<Counter>| async move {
            let n = c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
            n.to_string()
        });
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(app.serve(listener));

    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<Bytes>>();
    // Bound client-side concurrency: bursts of hundreds of simultaneous
    // connects overflow the OS listen backlog (128 on macOS) and are reset
    // by the kernel before the server ever sees them.
    let in_flight = Arc::new(tokio::sync::Semaphore::new(64));
    let tasks: Vec<_> = (0..400)
        .map(|_| {
            let client = client.clone();
            let in_flight = in_flight.clone();
            tokio::spawn(async move {
                let _permit = in_flight.acquire_owned().await.unwrap();
                let res = client
                    .get(format!("http://{addr}/inc").parse().unwrap())
                    .await
                    .unwrap();
                assert_eq!(res.status(), 200);
                let body = res.into_body().collect().await.unwrap().to_bytes();
                String::from_utf8(body.to_vec())
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            })
        })
        .collect();
    let mut seen = Vec::new();
    for t in tasks {
        seen.push(t.await.unwrap());
    }
    seen.sort_unstable();
    // Every request got a distinct counter value: no lost updates.
    assert_eq!(seen, (0..400).collect::<Vec<_>>());
    assert_eq!(counter.0.load(Ordering::SeqCst), 400);

    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn request_timeout_from_config_cancels_handler() {
    let cancelled = Arc::new(AtomicU64::new(0));
    struct DropFlag(Arc<AtomicU64>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (listener, addr) = listener().await;
    let app = App::new()
        .config(ServerConfig::new().request_timeout(Duration::from_millis(100)))
        .get("/hang", {
            let cancelled = cancelled.clone();
            move || async move {
                let _flag = DropFlag(cancelled);
                tokio::time::sleep(Duration::from_secs(30)).await;
                "never"
            }
        });
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(app.serve(listener));

    let (status, body) = fetch(addr, "/hang").await.unwrap();
    assert_eq!(status, 408);
    assert!(body.contains("REQUEST_TIMEOUT"));
    assert_eq!(
        cancelled.load(Ordering::SeqCst),
        1,
        "handler future was dropped"
    );

    shutdown.trigger();
    server.await.unwrap().unwrap();
}
