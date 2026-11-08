//! The JSON console format: valid JSON lines, structured fields, span
//! context and trace correlation.

use std::io::Write;
use std::sync::{Arc, Mutex};

use kerniq_observability::logging::json_layer;
use kerniq_observability::prelude::*;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SimpleSpanProcessor};
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
    type Writer = Buffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tracing::instrument(skip(password))]
fn authenticate(username: &str, password: &str) {
    let _ = password;
    tracing::info!(
        username,
        attempts = 1_u64,
        ok = true,
        "Authentication request received"
    );
}

#[tokio::test]
async fn json_lines_carry_fields_spans_and_trace_ids() {
    let spans = InMemorySpanExporter::default();
    let observability = Observability::builder()
        .traces(TracesConfig::new().processor(SimpleSpanProcessor::new(spans.clone())))
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::registry()
        .with(observability.tracing_layer())
        .with(json_layer(buffer.clone()));

    tracing::subscriber::with_default(subscriber, || {
        tracing::info_span!("HTTP request", http.route = "/login").in_scope(|| {
            authenticate("alice", "hunter2");
        });
        tracing::warn!(
            reason = "quote \" and newline \n",
            raw = "ctl\u{1}\ttab back\\slash é 🚀",
            debug = ?vec!["a\"b"],
            "outside any span"
        );
    });

    let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<serde_json::Value> = output
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("invalid JSON line {l}: {e}")))
        .collect();
    assert_eq!(lines.len(), 2);

    let auth = &lines[0];
    assert_eq!(auth["level"], "INFO");
    assert_eq!(auth["message"], "Authentication request received");
    assert_eq!(auth["username"], "alice");
    assert_eq!(auth["attempts"], 1);
    assert_eq!(auth["ok"], true);
    assert!(auth["timestamp"].as_str().unwrap().ends_with('Z'));
    assert!(
        !output.contains("hunter2"),
        "skipped fields are never logged"
    );

    let span_names: Vec<&str> = auth["spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(span_names, ["HTTP request", "authenticate"]);
    assert_eq!(auth["spans"][0]["http.route"], "/login");

    let finished = spans.get_finished_spans().unwrap();
    let inner = finished.iter().find(|s| s.name == "authenticate").unwrap();
    assert_eq!(auth["trace_id"], inner.span_context.trace_id().to_string());
    assert_eq!(auth["span_id"], inner.span_context.span_id().to_string());

    let outside = &lines[1];
    assert_eq!(outside["level"], "WARN");
    assert_eq!(outside["reason"], "quote \" and newline \n");
    assert_eq!(outside["raw"], "ctl\u{1}\ttab back\\slash é 🚀");
    assert_eq!(outside["debug"], "[\"a\\\"b\"]");
    assert_eq!(outside["message"], "outside any span");
    assert!(outside.get("trace_id").is_none());
    assert!(outside.get("spans").is_none());
}

#[tokio::test]
async fn disabled_levels_do_not_format() {
    // A `Debug` impl that panics if it is ever formatted.
    struct Expensive;
    impl std::fmt::Debug for Expensive {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("formatted a disabled event");
        }
    }

    let observability = Observability::builder()
        .logs(LogsConfig::json().filter("info").non_blocking(None))
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    let subscriber = tracing_subscriber::registry().with(observability.tracing_layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(value = ?Expensive, "never built");
    });
}
