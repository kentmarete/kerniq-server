//! End-to-end behavior of the HTTP instrumentation layer.

#[path = "../../kerniq-observability-testing/src/snapshot.rs"]
#[allow(dead_code)]
mod snapshot;

use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::routing::get;
use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use kerniq_observability_axum::HttpInstrumentation;
use kerniq_telemetry::semconv::{http as conv, kerniq, outcome};
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use snapshot::MetricSnapshot;
use tokio::sync::mpsc;
use tower::ServiceExt;

struct Metrics {
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
}

impl Metrics {
    fn new() -> Self {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        Self { provider, exporter }
    }

    fn instrumentation(&self) -> HttpInstrumentation {
        HttpInstrumentation::new().meter(&self.provider.meter("test"))
    }

    fn snapshot(&self) -> MetricSnapshot {
        self.provider.force_flush().unwrap();
        let snap = MetricSnapshot::latest(&self.exporter.get_finished_metrics().unwrap());
        self.exporter.reset();
        snap
    }
}

fn streaming_body(rx: mpsc::Receiver<&'static str>) -> Body {
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| {
            (
                Ok::<_, Infallible>(Bytes::from_static(chunk.as_bytes())),
                rx,
            )
        })
    });
    Body::from_stream(stream)
}

async fn send(app: &Router, method: &str, uri: &str) -> http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn records_route_template_not_raw_path() {
    let m = Metrics::new();
    let app = Router::new()
        .route("/users/{id}", get(|| async { "user" }))
        .layer(m.instrumentation().into_layer());

    for id in 0..5 {
        let res = send(&app, "GET", &format!("/users/{id}")).await;
        assert_eq!(res.status(), StatusCode::OK);
        res.into_body().collect().await.unwrap();
    }

    let snap = m.snapshot();
    assert_eq!(
        snap.attribute_values(conv::SERVER_REQUEST_DURATION, conv::ROUTE),
        ["/users/{id}"]
    );
    let point = snap
        .find(
            conv::SERVER_REQUEST_DURATION,
            &[
                (conv::ROUTE, "/users/{id}"),
                (conv::REQUEST_METHOD, "GET"),
                (conv::RESPONSE_STATUS_CODE, "200"),
                (conv::URL_SCHEME, "http"),
                (conv::NETWORK_PROTOCOL_VERSION, "1.1"),
            ],
        )
        .expect("duration point");
    assert_eq!(point.number(), 5.0);
    assert!(point.attr(conv::ERROR_TYPE).is_none());
    assert_eq!(snap.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    assert_eq!(
        snap.find(conv::SERVER_RESPONSE_BODY_SIZE, &[])
            .unwrap()
            .sum(),
        20.0 // 5 × "user"
    );
}

#[tokio::test]
async fn unmatched_requests_have_no_route_and_methods_are_bounded() {
    let m = Metrics::new();
    let app = Router::new()
        .route("/", get(|| async { "ok" }))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(m.instrumentation().into_layer());

    send(&app, "GET", "/no/such/path/123").await;
    send(&app, "PURGE", "/").await;

    let snap = m.snapshot();
    let not_found = snap
        .find(
            conv::SERVER_REQUEST_DURATION,
            &[(conv::RESPONSE_STATUS_CODE, "404")],
        )
        .unwrap();
    assert_eq!(not_found.attr(conv::ROUTE), None);
    assert_eq!(
        snap.attribute_values(conv::SERVER_REQUEST_DURATION, conv::REQUEST_METHOD),
        ["GET", "_OTHER"]
    );
}

#[tokio::test]
async fn excluded_routes_are_not_recorded() {
    let m = Metrics::new();
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/work", get(|| async { "done" }))
        .layer(m.instrumentation().exclude("/health").into_layer());

    send(&app, "GET", "/health").await;
    send(&app, "GET", "/work").await;

    let snap = m.snapshot();
    assert_eq!(
        snap.attribute_values(conv::SERVER_REQUEST_DURATION, conv::ROUTE),
        ["/work"]
    );
}

#[tokio::test]
async fn server_errors_carry_error_type() {
    let m = Metrics::new();
    let app = Router::new()
        .route("/boom", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
        .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
        .layer(m.instrumentation().into_layer());

    send(&app, "GET", "/boom").await;
    send(&app, "GET", "/missing").await;

    let snap = m.snapshot();
    let boom = snap
        .find(conv::SERVER_REQUEST_DURATION, &[(conv::ROUTE, "/boom")])
        .unwrap();
    assert_eq!(boom.attr(conv::ERROR_TYPE), Some("503"));
    let missing = snap
        .find(conv::SERVER_REQUEST_DURATION, &[(conv::ROUTE, "/missing")])
        .unwrap();
    assert_eq!(missing.attr(conv::ERROR_TYPE), None, "4xx is not an error");
}

#[tokio::test]
async fn request_body_size_from_content_length() {
    let m = Metrics::new();
    let app = Router::new()
        .route(
            "/upload",
            axum::routing::post(|body: Bytes| async move { body.len().to_string() }),
        )
        .layer(m.instrumentation().into_layer());

    let res = app
        .clone()
        .oneshot(
            Request::post("/upload")
                .header("content-length", "11")
                .body(Body::from("hello world"))
                .unwrap(),
        )
        .await
        .unwrap();
    res.into_body().collect().await.unwrap();

    let snap = m.snapshot();
    assert_eq!(
        snap.find(conv::SERVER_REQUEST_BODY_SIZE, &[])
            .unwrap()
            .sum(),
        11.0
    );
}

/// The core streaming guarantees: frames are forwarded as they are produced
/// (no buffering), the request stays active for the whole stream, and the
/// duration covers the stream lifetime.
#[tokio::test]
async fn streaming_response_is_not_buffered_and_measured_to_the_end() {
    let m = Metrics::new();
    let (tx, rx) = mpsc::channel(4);
    let rx = std::sync::Arc::new(std::sync::Mutex::new(Some(rx)));
    let app = Router::new()
        .route(
            "/stream",
            get(move || {
                let rx = rx.lock().unwrap().take().unwrap();
                async move { streaming_body(rx) }
            }),
        )
        .layer(m.instrumentation().into_layer());

    let res = send(&app, "GET", "/stream").await;
    let mut body = res.into_body();

    tx.send("first").await.unwrap();
    // The first frame arrives while the producer is still running.
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .expect("first frame must not wait for the end of the stream")
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap(), "first");

    let mid = m.snapshot();
    assert_eq!(
        mid.total(conv::SERVER_ACTIVE_REQUESTS, &[]),
        1.0,
        "still active"
    );
    assert!(
        !mid.contains(conv::SERVER_REQUEST_DURATION),
        "not finished yet"
    );
    assert_eq!(mid.total(kerniq::SERVER_TIME_TO_FIRST_BYTE, &[]), 1.0);

    tokio::time::sleep(Duration::from_millis(150)).await;
    tx.send("second").await.unwrap();
    drop(tx);
    let rest = body.collect().await.unwrap().to_bytes();
    assert_eq!(rest, "second");

    let done = m.snapshot();
    assert_eq!(done.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    let duration = done.find(conv::SERVER_REQUEST_DURATION, &[]).unwrap();
    assert_eq!(duration.attr(conv::ERROR_TYPE), None);
    assert!(
        duration.sum() >= 0.15,
        "duration covers the stream: {}",
        duration.sum()
    );
    let ttfb = done.find(kerniq::SERVER_TIME_TO_FIRST_BYTE, &[]).unwrap();
    assert!(ttfb.sum() < duration.sum());
    assert_eq!(
        done.find(conv::SERVER_RESPONSE_BODY_SIZE, &[])
            .unwrap()
            .sum(),
        11.0
    );
}

#[tokio::test]
async fn client_disconnect_mid_stream_is_recorded_as_aborted() {
    let m = Metrics::new();
    let (tx, rx) = mpsc::channel(4);
    let rx = std::sync::Arc::new(std::sync::Mutex::new(Some(rx)));
    let app = Router::new()
        .route(
            "/stream",
            get(move || {
                let rx = rx.lock().unwrap().take().unwrap();
                async move { streaming_body(rx) }
            }),
        )
        .layer(m.instrumentation().into_layer());

    let res = send(&app, "GET", "/stream").await;
    let mut body = res.into_body();
    tx.send("partial").await.unwrap();
    body.frame().await.unwrap().unwrap();
    drop(body); // what Hyper does when the client goes away

    let snap = m.snapshot();
    assert_eq!(snap.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    let point = snap.find(conv::SERVER_REQUEST_DURATION, &[]).unwrap();
    assert_eq!(point.attr(conv::ERROR_TYPE), Some(outcome::ABORTED));
    assert_eq!(point.attr(conv::RESPONSE_STATUS_CODE), Some("200"));
}

#[tokio::test]
async fn cancelled_handler_is_recorded_as_aborted() {
    let m = Metrics::new();
    let app = Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                "late"
            }),
        )
        .layer(m.instrumentation().into_layer());

    let pending = send(&app, "GET", "/slow");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), pending)
            .await
            .is_err()
    );

    let snap = m.snapshot();
    assert_eq!(snap.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    let point = snap.find(conv::SERVER_REQUEST_DURATION, &[]).unwrap();
    assert_eq!(point.attr(conv::ERROR_TYPE), Some(outcome::ABORTED));
    assert_eq!(point.attr(conv::RESPONSE_STATUS_CODE), None);
}

#[tokio::test]
async fn head_and_no_content_responses_complete_without_body_polling() {
    let m = Metrics::new();
    let app = Router::new()
        .route("/empty", get(|| async { StatusCode::NO_CONTENT }))
        .route("/doc", get(|| async { "body" }))
        .layer(m.instrumentation().into_layer());

    // Responses are dropped without polling their bodies, as Hyper does.
    drop(send(&app, "GET", "/empty").await);
    drop(send(&app, "HEAD", "/doc").await);

    let snap = m.snapshot();
    assert_eq!(snap.total(conv::SERVER_ACTIVE_REQUESTS, &[]), 0.0);
    for point in snap.metric(conv::SERVER_REQUEST_DURATION) {
        assert_eq!(point.attr(conv::ERROR_TYPE), None, "{point:?}");
    }
}

mod tracing_tests {
    use super::*;
    use opentelemetry::trace::{SpanKind, Status, TracerProvider as _};
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt;

    fn tracer() -> (SdkTracerProvider, InMemorySpanExporter) {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        (provider, exporter)
    }

    #[tokio::test]
    async fn continues_incoming_w3c_trace() {
        let (provider, exporter) = tracer();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = Router::new()
            .route(
                "/users/{id}",
                get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .layer(
                HttpInstrumentation::new()
                    .propagator(TraceContextPropagator::new())
                    .into_layer(),
            );

        let res = app
            .clone()
            .oneshot(
                Request::get("/users/7?secret=1")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                    )
                    .header("x-request-id", "req-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        res.into_body().collect().await.unwrap();
        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.name, "GET /users/{id}");
        assert_eq!(span.span_kind, SpanKind::Server);
        assert_eq!(
            span.span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(span.parent_span_id.to_string(), "00f067aa0ba902b7");
        assert!(matches!(span.status, Status::Error { .. }));
        let attr = |k: &str| {
            span.attributes
                .iter()
                .find(|kv| kv.key.as_str() == k)
                .map(|kv| kv.value.as_str().into_owned())
        };
        assert_eq!(attr("http.route").as_deref(), Some("/users/{id}"));
        assert_eq!(attr("url.path").as_deref(), Some("/users/7"));
        assert_eq!(attr("http.response.status_code").as_deref(), Some("500"));
        assert_eq!(attr("error.type").as_deref(), Some("500"));
        assert_eq!(attr("kerniq.request_id").as_deref(), Some("req-123"));
        // The query string is never captured.
        assert!(
            span.attributes
                .iter()
                .all(|kv| !kv.value.as_str().contains("secret"))
        );
    }

    #[tokio::test]
    async fn span_lasts_for_the_whole_stream_and_is_parent_of_handler_spans() {
        let (provider, exporter) = tracer();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = Router::new()
            .route(
                "/stream",
                get(|| async {
                    tracing::info_span!("load_model").in_scope(|| {});
                    let stream = futures_util::stream::iter(["a", "b"]).then(|c| async move {
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        Ok::<_, Infallible>(Bytes::from_static(c.as_bytes()))
                    });
                    Body::from_stream(stream)
                }),
            )
            .layer(HttpInstrumentation::new().into_layer());

        let res = send(&app, "GET", "/stream").await;
        res.into_body().collect().await.unwrap();
        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        let request = spans.iter().find(|s| s.name == "GET /stream").unwrap();
        let child = spans.iter().find(|s| s.name == "load_model").unwrap();
        assert_eq!(child.parent_span_id, request.span_context.span_id());
        let lifetime = request.end_time.duration_since(request.start_time).unwrap();
        assert!(lifetime >= Duration::from_millis(60), "{lifetime:?}");
    }

    use futures_util::StreamExt;
}
