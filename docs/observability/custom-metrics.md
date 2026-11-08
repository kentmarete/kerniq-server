# Custom metrics

Application metrics use the OpenTelemetry `Meter` API directly. Kerniq does
not wrap it; `Observability::meter` hands you one.

```rust
use kerniq_server::observability::prelude::*;

# let observability = kerniq_server::observability::Observability::disabled();
let meter = observability.meter("payment-service");

let payment_counter = meter
    .u64_counter("payments.processed")
    .with_unit("{payment}")
    .with_description("Payments processed, by type and outcome.")
    .build();

payment_counter.add(1, &[
    KeyValue::new("payment.type", "transfer"),
    KeyValue::new("outcome", "accepted"),
]);
```

When metrics are disabled, `meter` returns a no-op meter, so instrumentation
code never has to check.

## Instruments

| Instrument | Builder | Use for |
|---|---|---|
| Counter | `u64_counter`, `f64_counter` | things that only increase (requests, payments, tokens) |
| UpDownCounter | `i64_up_down_counter` | things that go up and down (in-flight work) |
| Histogram | `f64_histogram`, `u64_histogram` | distributions (latency, amounts, sizes) |
| Gauge | `u64_gauge`, `f64_gauge` | a current value you set |
| Observable counter / gauge | `u64_observable_gauge(..).with_callback(..)` | values read only when collected (queue depth, pool size) |

## Rules

1. **Create instruments once** (at startup, in your state) and clone them.
   Creating an instrument per request is slow and defeats aggregation.
2. **Keep attributes bounded.** Every distinct attribute combination is a
   separate series kept in memory. Allowed: route templates, enumerations
   (payment type, outcome, currency, model family). Never: user, customer
   or account ids, transaction or request ids, e-mail addresses, raw URLs,
   error messages, prompts, free-form model names. Put those on spans and
   log events instead.
3. **Use `BoundedLabel`** to map untrusted input onto an allow-list:

   ```rust
   use kerniq_server::observability::BoundedLabel;

   static CURRENCY: BoundedLabel = BoundedLabel::new(&["USD", "EUR", "GBP", "KES"], "other");
   assert_eq!(CURRENCY.get("XYZ"), "other");
   ```

4. **Use `&'static str` values** where possible (`KeyValue::new("outcome",
   "accepted")`); they do not allocate.
5. **Record amounts in histograms, not labels.**

As a safety net the SDK caps each instrument at 2 000 attribute sets
(`MetricsConfig::cardinality_limit` to change it); excess measurements are
folded into one `otel.metric.overflow` series. In tests,
`TestTelemetry::assert_bounded_metric_labels` fails on forbidden keys.

## Examples by domain

All of these are implemented in `examples/observability-demo/src/business.rs`.

| Domain | Metric | Attributes |
|---|---|---|
| Banking | `bank.transactions` (counter) | `transaction.type`, `currency`, `outcome` |
| Banking | `bank.transaction.amount` (histogram, minor units) | `transaction.type`, `currency` |
| Loans | `loan.applications` (counter) | `loan.product`, `loan.decision` |
| Loans | `loan.decision.duration` (histogram, s) | `loan.product`, `loan.decision` |
| Loans | `loan.applications.in_review` (up-down counter) | `loan.product` |
| Database | `db.client.operation.duration` (histogram, s) | `db.system.name`, `db.operation.name` |
| Authentication | `auth.attempts` (counter) | `auth.method`, `outcome` — never the username |
| External calls | `http.client.request.duration` (histogram, s) | `http.request.method`, `server.address` (a fixed service name) |
| Queues | `queue.depth` (observable gauge) | `queue.name` |
| Queues | `queue.jobs.processed` (counter), `queue.job.wait.duration` (histogram) | `queue.name`, `job.kind`, `outcome` |
| AI inference | see below | `inference.model` (bounded), `inference.outcome` |

Prefer OpenTelemetry semantic-convention names (`db.client.*`,
`http.client.*`, `messaging.*`) where one exists.

## AI inference

`kerniq_telemetry::inference` provides the instruments and a per-request
guard for long-running generation:

```rust
use kerniq_server::observability::telemetry::inference::InferenceMetrics;
use kerniq_server::observability::{BoundedLabel, KeyValue};

static MODEL: BoundedLabel = BoundedLabel::new(&["chat-small", "chat-large"], "other");
# let observability = kerniq_server::observability::Observability::disabled();
let metrics = InferenceMetrics::new(&observability.meter("inference"));

let mut request = metrics.start([KeyValue::new("inference.model", MODEL.get("chat-small"))]);
request.input_tokens(512);
request.execution_started();          // left the queue → inference.queue.duration
for _token in 0..128 {
    request.tokens_generated(1);      // first call → inference.time_to_first_token
}
request.finish();                     // or .fail("timeout"); dropping it = cancelled
```

| Metric | |
|---|---|
| `inference.requests` | by `inference.outcome` (`success`, `error`, `cancelled`) |
| `inference.errors` | by `error.type` (a static, bounded value) |
| `inference.queue.duration` | arrival → execution start |
| `inference.time_to_first_token` | arrival → first token |
| `inference.duration` | arrival → end |
| `inference.tokens.generated`, `inference.tokens.input` | aggregated per request |
| `inference.token_throughput` | output tokens / s of generation, per request |
| `inference.active_streams` | currently generating |

Per-token cost is an integer add: tokens are counted locally and recorded
once at the end, and nothing is logged per token. If the client disconnects,
Hyper drops the response stream, the guard is dropped with it, and the
request is recorded as `cancelled`. Kerniq does not run models; these are
conventions for applications that do. They map onto the OpenTelemetry GenAI
conventions (`gen_ai.server.time_to_first_token`,
`gen_ai.server.request.duration`, `gen_ai.client.token.usage`) for teams
standardizing on those.

## Testing metrics

```rust
use kerniq_observability_testing::TestTelemetry;
use kerniq_server::observability::KeyValue;

# #[tokio::main(flavor = "current_thread")]
# async fn main() {
let telemetry = TestTelemetry::new().await;
let counter = telemetry.observability().meter("t").u64_counter("payments.processed").build();
counter.add(1, &[KeyValue::new("payment.type", "transfer")]);

let metrics = telemetry.metrics();
assert_eq!(metrics.total("payments.processed", &[("payment.type", "transfer")]), 1.0);
telemetry.assert_bounded_metric_labels();
# }
```
