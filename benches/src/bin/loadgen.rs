//! A closed-loop HTTP/1.1 load generator.
//!
//! ```sh
//! loadgen <url> [connections=64] [seconds=10] [threads=4]
//! ```
//!
//! Each connection is a keep-alive HTTP/1.1 connection issuing requests
//! back-to-back. Reports requests/s, throughput and latency percentiles as a
//! single JSON line (for scripts) after a 1-second warm-up.

use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

struct ConnStats {
    latencies_us: Vec<u32>,
    bytes: u64,
    errors: u64,
}

async fn connection(
    host: String,
    path: String,
    warmup_until: Instant,
    deadline: Instant,
) -> ConnStats {
    let mut stats = ConnStats {
        latencies_us: Vec::with_capacity(200_000),
        bytes: 0,
        errors: 0,
    };
    let stream = TcpStream::connect(&host).await.expect("connect");
    stream.set_nodelay(true).ok();
    let (mut sender, conn) = http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(conn);
    loop {
        let start = Instant::now();
        if start >= deadline {
            break;
        }
        // Hyper's HTTP/1 client accepts the next request only once the
        // previous response has been fully processed.
        if sender.ready().await.is_err() {
            stats.errors += 1;
            report_error("connection closed");
            break;
        }
        let request = http::Request::get(&path)
            .header("host", &host)
            .body(Empty::<Bytes>::new())
            .expect("request");
        match sender.send_request(request).await {
            Ok(response) => {
                let ok = response.status().is_success();
                let body = response.into_body().collect().await.map(|b| b.to_bytes());
                if start >= warmup_until {
                    match (ok, body) {
                        (true, Ok(body)) => {
                            stats.bytes += body.len() as u64;
                            stats.latencies_us.push(start.elapsed().as_micros() as u32);
                        }
                        (ok, body) => {
                            stats.errors += 1;
                            report_error(&format!("status ok={ok}, body={:?}", body.err()));
                        }
                    }
                }
            }
            Err(err) => {
                stats.errors += 1;
                report_error(&format!("request failed: {err:?}"));
                break;
            }
        }
    }
    stats
}

/// Prints the first few error causes to stderr (stdout carries the result).
fn report_error(message: &str) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static REPORTED: AtomicU32 = AtomicU32::new(0);
    if REPORTED.fetch_add(1, Ordering::Relaxed) < 5 {
        eprintln!("loadgen error: {message}");
    }
}

fn percentile(sorted: &[u32], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    f64::from(sorted[rank]) / 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .expect("usage: loadgen <url> [connections] [seconds] [threads]");
    let connections: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(64);
    let seconds: u64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(10);
    let threads: usize = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(4);

    let rest = url
        .strip_prefix("http://")
        .expect("only http:// URLs are supported");
    let (host, path) = rest
        .split_once('/')
        .map_or((rest.to_owned(), "/".to_owned()), |(h, p)| {
            (h.to_owned(), format!("/{p}"))
        });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .expect("runtime");
    let results = runtime.block_on(async move {
        let warmup_until = Instant::now() + Duration::from_secs(1);
        let deadline = warmup_until + Duration::from_secs(seconds);
        let tasks: Vec<_> = (0..connections)
            .map(|_| {
                tokio::spawn(connection(
                    host.clone(),
                    path.clone(),
                    warmup_until,
                    deadline,
                ))
            })
            .collect();
        let mut all = Vec::with_capacity(tasks.len());
        for task in tasks {
            all.push(task.await.expect("connection task"));
        }
        all
    });

    let mut latencies: Vec<u32> = results
        .iter()
        .flat_map(|s| s.latencies_us.iter().copied())
        .collect();
    latencies.sort_unstable();
    let requests = latencies.len() as f64;
    let bytes: u64 = results.iter().map(|s| s.bytes).sum();
    let errors: u64 = results.iter().map(|s| s.errors).sum();
    let secs = seconds as f64;
    println!(
        "{{\"rps\":{:.0},\"body_mb_per_s\":{:.2},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"requests\":{},\"errors\":{}}}",
        requests / secs,
        bytes as f64 / secs / 1e6,
        percentile(&latencies, 50.0),
        percentile(&latencies, 95.0),
        percentile(&latencies, 99.0),
        latencies.last().map_or(0.0, |v| f64::from(*v) / 1000.0),
        latencies.len(),
        errors
    );
}
