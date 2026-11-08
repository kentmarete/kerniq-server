//! The structured JSON log format.
//!
//! One JSON object per line:
//!
//! ```json
//! {"timestamp":"2026-11-08T10:15:30.123456Z","level":"INFO","target":"payment_api",
//!  "message":"payment accepted","amount_minor":1250,
//!  "trace_id":"4bf92f3577b34da6a3ce929d0e0e4736","span_id":"00f067aa0ba902b7",
//!  "spans":[{"name":"HTTP request","http.request.method":"POST","http.route":"/payments",
//!            "kerniq.request_id":"7f0c..."}]}
//! ```
//!
//! Event fields are flattened into the object. `trace_id`/`span_id` come
//! from the OpenTelemetry context of the current span (present only when
//! tracing is enabled), so log lines can be joined with traces in any
//! backend. Field values are whatever the application logs: never log
//! passwords, tokens, authorization headers or payloads.

use std::fmt;
use std::sync::{Arc, OnceLock};

use tracing::dispatcher::WeakDispatch;
use tracing::field::{Field, Visit};
use tracing::{Dispatch, Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields, MakeWriter};
use tracing_subscriber::registry::LookupSpan;

/// JSON event formatter with trace correlation.
///
/// Use [`json_layer`], which pairs it with [`SpanFields`] (span fields
/// stored as JSON members) and with the hook that lets it read OpenTelemetry trace
/// ids. A bare `JsonFormat` still produces valid JSON, without `trace_id`.
#[derive(Debug, Clone, Default)]
pub struct JsonFormat {
    // The subscriber this format is registered with. Inside an event
    // callback `Span::current()` is unavailable (tracing forbids re-entrant
    // dispatch), so trace ids are looked up through this handle instead.
    dispatch: Arc<OnceLock<WeakDispatch>>,
}

impl JsonFormat {
    /// Creates the formatter.
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "tracing")]
    fn otel_context<S>(
        &self,
        span: &tracing_subscriber::registry::SpanRef<'_, S>,
    ) -> Option<opentelemetry::Context>
    where
        S: for<'a> LookupSpan<'a>,
    {
        let dispatch = self.dispatch.get()?.upgrade()?;
        tracing_opentelemetry::get_otel_context(&span.id(), &dispatch)
    }
}

/// Records the subscriber's `Dispatch` for [`JsonFormat`].
struct DispatchCapture(Arc<OnceLock<WeakDispatch>>);

impl<S: Subscriber> Layer<S> for DispatchCapture {
    fn on_register_dispatch(&self, subscriber: &Dispatch) {
        let _ = self.0.set(subscriber.downgrade());
    }
}

/// A complete JSON console layer writing to `writer`.
///
/// ```
/// use tracing_subscriber::layer::SubscriberExt;
///
/// let subscriber = tracing_subscriber::registry()
///     .with(kerniq_observability::logging::json_layer(std::io::stdout));
/// ```
pub fn json_layer<S, W>(writer: W) -> Box<dyn Layer<S> + Send + Sync>
where
    S: Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let format = JsonFormat::new();
    let capture = DispatchCapture(format.dispatch.clone());
    capture
        .and_then(
            tracing_subscriber::fmt::layer()
                .event_format(format)
                .fmt_fields(SpanFields)
                .with_writer(writer),
        )
        .boxed()
}

/// Writes event fields as `,"key":value` pairs straight into the output.
/// Values are escaped in place; nothing is collected into a map first.
struct FieldVisitor<'a, 'w> {
    writer: &'a mut Writer<'w>,
    result: fmt::Result,
    skip_otel: bool,
}

impl FieldVisitor<'_, '_> {
    fn key(&mut self, field: &Field) -> fmt::Result {
        self.writer.write_char(',')?;
        write_json_str(self.writer, field.name())?;
        self.writer.write_char(':')
    }

    fn write(&mut self, f: impl FnOnce(&mut Writer<'_>) -> fmt::Result, field: &Field) {
        if self.skip_otel && field.name().starts_with("otel.") {
            return;
        }
        if self.result.is_ok() {
            self.result = self.key(field).and_then(|()| f(self.writer));
        }
    }
}

impl Visit for FieldVisitor<'_, '_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.write(|w| write_json_str(w, value), field);
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.write(|w| write!(w, "{value}"), field);
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.write(|w| write!(w, "{value}"), field);
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        if value.is_finite() {
            self.write(|w| write!(w, "{value}"), field);
        } else {
            self.write(|w| write!(w, "\"{value}\""), field);
        }
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.write(|w| write!(w, "{value}"), field);
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.write(|w| write_json_display(w, value), field);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(|w| write_json_display(w, &DebugAsDisplay(value)), field);
    }
}

struct DebugAsDisplay<'a>(&'a dyn fmt::Debug);

impl fmt::Display for DebugAsDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

/// Escapes `s` as JSON string contents.
fn escape_into(w: &mut dyn fmt::Write, s: &str) -> fmt::Result {
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        let escaped = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0..=0x1f => {
                w.write_str(&s[start..i])?;
                write!(w, "\\u{b:04x}")?;
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        w.write_str(&s[start..i])?;
        w.write_str(escaped)?;
        start = i + 1;
    }
    w.write_str(&s[start..])
}

fn write_json_str(writer: &mut Writer<'_>, value: &str) -> fmt::Result {
    writer.write_char('"')?;
    escape_into(writer, value)?;
    writer.write_char('"')
}

/// Writes a `Display` value as a JSON string, escaping as it is formatted.
fn write_json_display(writer: &mut Writer<'_>, value: &dyn fmt::Display) -> fmt::Result {
    struct Escaper<'a, 'w>(&'a mut Writer<'w>);
    impl fmt::Write for Escaper<'_, '_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            escape_into(self.0, s)
        }
    }
    writer.write_char('"')?;
    fmt::Write::write_fmt(&mut Escaper(writer), format_args!("{value}"))?;
    writer.write_char('"')
}

impl<S, N> FormatEvent<S, N> for JsonFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();

        let mut timestamp = String::with_capacity(32);
        SystemTime.format_time(&mut Writer::new(&mut timestamp))?;

        writer.write_str("{\"timestamp\":")?;
        write_json_str(&mut writer, &timestamp)?;
        writer.write_str(",\"level\":\"")?;
        writer.write_str(meta.level().as_str())?;
        writer.write_str("\",\"target\":")?;
        write_json_str(&mut writer, meta.target())?;

        // `message` is an ordinary field in `tracing`; it is written in
        // field order like the others.
        let mut visitor = FieldVisitor {
            writer: &mut writer,
            result: Ok(()),
            skip_otel: false,
        };
        event.record(&mut visitor);
        visitor.result?;

        #[cfg(feature = "tracing")]
        {
            use opentelemetry::trace::TraceContextExt;
            // The innermost span of the event (contextual or explicit parent).
            let leaf = ctx.event_scope().and_then(|mut scope| scope.next());
            if let Some(cx) = leaf.as_ref().and_then(|span| self.otel_context(span)) {
                let span_ref = cx.span();
                let sc = span_ref.span_context();
                if sc.is_valid() {
                    write!(
                        writer,
                        ",\"trace_id\":\"{}\",\"span_id\":\"{}\",\"trace_sampled\":{}",
                        sc.trace_id(),
                        sc.span_id(),
                        sc.is_sampled()
                    )?;
                }
            }
        }

        if let Some(scope) = ctx.event_scope() {
            writer.write_str(",\"spans\":[")?;
            let mut first = true;
            for span in scope.from_root() {
                if !first {
                    writer.write_char(',')?;
                }
                first = false;
                writer.write_str("{\"name\":")?;
                write_json_str(&mut writer, span.name())?;
                let ext = span.extensions();
                if let Some(stored) = ext.get::<FormattedFields<N>>() {
                    // `SpanFields` stores `,"k":v` members, ready to splice.
                    let text = stored.fields.as_str();
                    if text.starts_with(',') {
                        writer.write_str(text)?;
                    }
                }
                writer.write_char('}')?;
            }
            writer.write_char(']')?;
        }

        writer.write_str("}\n")
    }
}

/// Span-field storage for [`JsonFormat`]: each field is kept as an escaped
/// `,"key":value` member.
///
/// `tracing-subscriber`'s `JsonFields` re-parses and re-serializes a span's
/// stored JSON on every `Span::record`; this formatter appends instead, so
/// recording a field is proportional to that field only. Fields whose name
/// starts with `otel.` (instructions to the OpenTelemetry bridge, such as
/// `otel.kind`) are not printed.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpanFields;

impl<'w> FormatFields<'w> for SpanFields {
    fn format_fields<R: RecordFields>(&self, mut writer: Writer<'w>, fields: R) -> fmt::Result {
        let mut visitor = FieldVisitor {
            writer: &mut writer,
            result: Ok(()),
            skip_otel: true,
        };
        fields.record(&mut visitor);
        visitor.result
    }

    fn add_fields(
        &self,
        current: &'w mut FormattedFields<Self>,
        fields: &tracing::span::Record<'_>,
    ) -> fmt::Result {
        let mut writer = current.as_writer();
        let mut visitor = FieldVisitor {
            writer: &mut writer,
            result: Ok(()),
            skip_otel: true,
        };
        fields.record(&mut visitor);
        visitor.result
    }
}
