//! Scrape output and endpoint behavior.

use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use kerniq_observability_prometheus::{PrometheusConfig, PrometheusExporter};
use opentelemetry::KeyValue;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn setup(config: PrometheusConfig) -> (SdkMeterProvider, PrometheusExporter) {
    let (reader, exporter) = PrometheusExporter::build(config).unwrap();
    let provider = SdkMeterProvider::builder()
        .with_resource(Resource::builder().with_service_name("payment-api").build())
        .with_reader(reader)
        .build();
    (provider, exporter)
}

#[test]
fn renders_otel_instruments_in_prometheus_format() {
    let (provider, exporter) = setup(PrometheusConfig::default());
    let meter = provider.meter("payments");
    meter
        .u64_counter("payments.processed")
        .build()
        .add(3, &[KeyValue::new("payment.type", "transfer")]);
    meter
        .f64_histogram("http.server.request.duration")
        .with_unit("s")
        .build()
        .record(0.2, &[KeyValue::new("http.route", "/users/{id}")]);
    meter
        .i64_up_down_counter("http.server.active_requests")
        .build()
        .add(2, &[]);

    let text = exporter.render().unwrap();
    assert!(
        text.contains("# TYPE payments_processed_total counter"),
        "{text}"
    );
    assert!(
        text.contains(
            r#"payments_processed_total{payment_type="transfer",service_name="payment-api"} 3"#
        ),
        "{text}"
    );
    assert!(
        text.contains("# TYPE http_server_request_duration_seconds histogram"),
        "{text}"
    );
    assert!(text.contains(r#"http_server_request_duration_seconds_count{http_route="/users/{id}",service_name="payment-api"} 1"#), "{text}");
    assert!(
        text.contains(r#"http_server_active_requests{service_name="payment-api"} 2"#),
        "{text}"
    );
    assert!(
        text.contains(r#"target_info{service_name="payment-api","#),
        "{text}"
    );
    assert!(
        !text.contains("otel_scope_name"),
        "scope labels are off by default: {text}"
    );
}

#[tokio::test]
async fn router_serves_text_format_on_configured_path() {
    let (provider, exporter) = setup(PrometheusConfig::new().path("/internal/metrics"));
    provider.meter("t").u64_counter("hits").build().add(1, &[]);

    let res = exporter
        .router::<()>()
        .oneshot(
            Request::get("/internal/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(std::str::from_utf8(&body).unwrap().contains("hits_total{"));
}

#[tokio::test]
async fn serves_on_a_separate_listener_and_stops_on_shutdown() {
    let (provider, exporter) = setup(PrometheusConfig::default());
    provider.meter("t").u64_counter("hits").build().add(5, &[]);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn({
        let exporter = exporter.clone();
        async move {
            exporter
                .serve(listener, async {
                    let _ = stopped.await;
                })
                .await
        }
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("} 5"), "{response}");

    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .expect("listener stops")
        .unwrap()
        .unwrap();
}
