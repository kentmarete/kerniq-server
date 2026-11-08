//! `App::observability`: HTTP instrumentation, Prometheus endpoints, log
//! correlation and lifecycle, through the public API.

use std::net::SocketAddr;
use std::time::Duration;

use http_body_util::BodyExt;
use kerniq_observability_testing::TestTelemetry;
use kerniq_server::observability::prelude::*;
use kerniq_server::observability::telemetry::semconv::{http as conv, outcome};
use kerniq_server::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = router
        .clone()
        .oneshot(
            http::Request::get(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Sends a raw HTTP/1.1 request and returns the full response text.
async fn raw_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    out
}

#[tokio::test]
async fn http_metrics_use_route_templates_and_cover_fallbacks() {
    let telemetry = TestTelemetry::new().await;
    let router = App::new()
        .observability(telemetry.observability().clone())
        .get("/users/{id}", |Path(id): Path<u64>| async move {
            format!("user {id}")
        })
        .into_axum_router();

    for id in [1, 2, 3] {
        assert_eq!(
            get(&router, &format!("/users/{id}")).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(get(&router, "/nope").await.0, StatusCode::NOT_FOUND);

    let metrics = telemetry.metrics();
    assert_eq!(
        metrics.total(
            conv::SERVER_REQUEST_DURATION,
            &[
                (conv::ROUTE, "/users/{id}"),
                (conv::RESPONSE_STATUS_CODE, "200")
            ]
        ),
        3.0
    );
    let not_found = metrics
        .find(
            conv::SERVER_REQUEST_DURATION,
            &[(conv::RESPONSE_STATUS_CODE, "404")],
        )
        .expect("the 404 fallback is instrumented");
    assert_eq!(not_found.attr(conv::ROUTE), None);
    assert_eq!(metrics.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    telemetry.assert_bounded_metric_labels();
}

#[tokio::test]
async fn instrumentation_is_outermost_and_sees_timeouts() {
    let telemetry = TestTelemetry::new().await;
    let router = App::new()
        .observability(telemetry.observability().clone())
        .config(ServerConfig::new().request_timeout(Duration::from_millis(20)))
        .get("/slow", || async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "late"
        })
        .into_axum_router();

    let (status, _) = get(&router, "/slow").await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    let metrics = telemetry.metrics();
    let point = metrics
        .find(conv::SERVER_REQUEST_DURATION, &[(conv::ROUTE, "/slow")])
        .unwrap();
    assert_eq!(point.attr(conv::RESPONSE_STATUS_CODE), Some("408"));
}

#[tokio::test]
async fn logs_are_correlated_with_the_request_trace() {
    let telemetry = TestTelemetry::new().await;
    let router = App::new()
        .observability(telemetry.observability().clone())
        .middleware(RequestId::new())
        .post("/payments", || async {
            tracing::info!(amount_minor = 1250, "payment accepted");
            StatusCode::CREATED
        })
        .into_axum_router();

    let res = router
        .clone()
        .oneshot(
            http::Request::post("/payments")
                .header(
                    "traceparent",
                    "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.into_body().collect().await.unwrap();

    let span = telemetry.span("POST /payments");
    assert_eq!(
        span.span_context.trace_id().to_string(),
        "0af7651916cd43dd8448eb211c80319c"
    );
    assert_eq!(span.parent_span_id.to_string(), "b7ad6b7169203331");
    let request_id = span
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "kerniq.request_id")
        .map(|kv| kv.value.to_string())
        .expect("request id recorded on the span");
    assert!(!request_id.is_empty());

    let logs = telemetry.logs();
    let log = logs
        .iter()
        .find(|l| l.body == "payment accepted")
        .expect("log bridged");
    assert_eq!(
        log.trace_id.as_deref(),
        Some("0af7651916cd43dd8448eb211c80319c")
    );
    assert_eq!(log.span_id, Some(span.span_context.span_id().to_string()));
    assert_eq!(log.attr("amount_minor"), Some("1250"));
}

#[tokio::test]
async fn prometheus_on_the_app_listener_is_not_instrumented() {
    let observability = Observability::builder()
        .service_name("scrape-test")
        .metrics(MetricsConfig::prometheus())
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let router = App::new()
        .observability(observability.clone())
        .middleware(RateLimit::per_second(1).burst(1))
        .get("/users/{id}", || async { "user" })
        .into_axum_router();

    get(&router, "/users/42").await;
    // Scrape twice: neither scrape is rate limited nor recorded.
    let _ = get(&router, "/metrics").await;
    let (status, body) = get(&router, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"http_server_request_duration_seconds_count{http_request_method="GET",http_response_status_code="200",http_route="/users/{id}""#),
        "{body}"
    );
    assert!(
        !body.contains(r#"http_route="/metrics""#),
        "scrapes must not be recorded: {body}"
    );
    assert!(
        !body.contains("/users/42"),
        "raw paths must never be labels: {body}"
    );
    assert!(body.contains(r#"service_name="scrape-test""#), "{body}");
}

#[tokio::test]
async fn prometheus_on_a_separate_listener() {
    let metrics_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let observability = Observability::builder()
        .metrics(MetricsConfig::prometheus().prometheus_listener(metrics_port))
        .install_subscriber(false)
        .build()
        .await
        .unwrap();

    let app = App::new()
        .observability(observability.clone())
        .get("/", || async { "home" });
    let shutdown = app.shutdown_handle();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = listener.local_addr().unwrap();
    let server = tokio::spawn(app.serve(listener));

    // Wait for the metrics listener.
    for _ in 0..100 {
        if TcpStream::connect(metrics_port).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(raw_get(app_addr, "/").await.contains("home"));
    let scrape = raw_get(metrics_port, "/metrics").await;
    assert!(scrape.starts_with("HTTP/1.1 200"), "{scrape}");
    assert!(
        scrape.contains("http_server_request_duration_seconds"),
        "{scrape}"
    );
    assert!(
        raw_get(app_addr, "/metrics")
            .await
            .starts_with("HTTP/1.1 404")
    );

    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("server stops")
        .unwrap()
        .unwrap();
    // The app shut telemetry down; a second shutdown is a no-op.
    assert_eq!(observability.shutdown().await.duration, Duration::ZERO);
    assert!(
        TcpStream::connect(metrics_port).await.is_err(),
        "metrics listener closed"
    );
}

/// A long-lived streaming response over a real connection, abandoned by the
/// client: recorded as aborted, active requests return to zero.
#[tokio::test]
async fn client_disconnect_during_a_real_stream() {
    let telemetry = TestTelemetry::new().await;
    let app = App::new()
        .observability(telemetry.observability().clone())
        .get("/events", || async {
            let stream = futures_util::stream::unfold(0u32, |n| async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Some((
                    Ok::<_, std::convert::Infallible>(Bytes::from(format!("data: {n}\n\n"))),
                    n + 1,
                ))
            });
            (
                [("content-type", "text/event-stream")],
                Body::from_stream(stream),
            )
        });
    let shutdown = app.shutdown_handle();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(app.serve(listener));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /events HTTP/1.1\r\nhost: test\r\n\r\n")
        .await
        .unwrap();
    let mut buf = vec![0u8; 512];
    let n = stream.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).contains("200 OK"));

    // While streaming, the request is active.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        telemetry.metrics().total(conv::SERVER_ACTIVE_REQUESTS, &[]),
        1.0
    );

    drop(stream);
    let mut aborted = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let metrics = telemetry.metrics();
        if let Some(p) = metrics.find(conv::SERVER_REQUEST_DURATION, &[(conv::ROUTE, "/events")]) {
            aborted = Some((p.attr(conv::ERROR_TYPE).map(str::to_owned), p.sum()));
            assert_eq!(metrics.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
            break;
        }
    }
    let (error_type, duration) = aborted.expect("aborted stream recorded");
    assert_eq!(error_type.as_deref(), Some(outcome::ABORTED));
    assert!(
        duration >= 0.05,
        "duration covers the stream lifetime: {duration}"
    );

    shutdown.trigger();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn disabled_observability_is_inert() {
    let observability = Observability::disabled();
    let counter = observability.meter("noop").u64_counter("x").build();
    counter.add(1, &[]);
    let router = App::new()
        .observability(observability.clone())
        .get("/", || async { "ok" })
        .into_axum_router();
    assert_eq!(get(&router, "/").await.0, StatusCode::OK);
    let report = observability.shutdown().await;
    assert!(report.errors.is_empty());
    assert!(!report.timed_out);
}
