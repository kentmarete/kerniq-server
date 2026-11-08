//! Business instrumentation: banking transactions, loan processing,
//! database calls, authentication, outbound HTTP and queue processing.
//!
//! Every instrument is created once, in `Metrics::new`, and reused. Metric
//! attributes are bounded enumerations; identifiers (accounts, users,
//! transactions, amounts) go on spans and log events only.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use kerniq_server::observability::Observability;
use kerniq_server::observability::opentelemetry::propagation::TextMapPropagator;
use kerniq_server::observability::prelude::*;
use kerniq_server::observability::propagation::inject_current_context;
use kerniq_server::prelude::*;
use kerniq_server::runtime::Shutdown;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::inference::Inference;

// Bounded label sets. Anything else maps to "other".
static CURRENCY: BoundedLabel = BoundedLabel::new(&["USD", "EUR", "GBP", "KES"], "other");
static TX_TYPE: BoundedLabel = BoundedLabel::new(&["deposit", "withdrawal", "transfer"], "other");
static LOAN_PRODUCT: BoundedLabel = BoundedLabel::new(&["personal", "mortgage", "auto"], "other");

#[derive(Clone)]
pub struct Metrics {
    transactions: Counter<u64>,
    transaction_amount: Histogram<u64>,
    loan_applications: Counter<u64>,
    loan_decision_duration: Histogram<f64>,
    loans_in_review: UpDownCounter<i64>,
    db_duration: Histogram<f64>,
    auth_attempts: Counter<u64>,
    client_duration: Histogram<f64>,
    jobs: Counter<u64>,
    job_wait: Histogram<f64>,
}

impl Metrics {
    fn new(meter: &Meter, queue_depth: Arc<AtomicI64>) -> Self {
        // Asynchronous instrument: read only when metrics are collected.
        meter
            .i64_observable_gauge("queue.depth")
            .with_unit("{job}")
            .with_description("Jobs waiting in the settlement queue.")
            .with_callback(move |o| {
                o.observe(
                    queue_depth.load(Ordering::Relaxed),
                    &[KeyValue::new("queue.name", "settlement")],
                )
            })
            .build();
        Self {
            transactions: meter
                .u64_counter("bank.transactions")
                .with_unit("{transaction}")
                .with_description("Banking transactions by type, currency and outcome.")
                .build(),
            transaction_amount: meter
                .u64_histogram("bank.transaction.amount")
                .with_unit("{minor_unit}")
                .with_description("Transaction amounts in minor currency units.")
                .with_boundaries(vec![
                    100.0,
                    1_000.0,
                    10_000.0,
                    100_000.0,
                    1_000_000.0,
                    10_000_000.0,
                ])
                .build(),
            loan_applications: meter
                .u64_counter("loan.applications")
                .with_unit("{application}")
                .with_description("Loan applications by product and decision.")
                .build(),
            loan_decision_duration: meter
                .f64_histogram("loan.decision.duration")
                .with_unit("s")
                .with_description("Time to reach a loan decision.")
                .build(),
            loans_in_review: meter
                .i64_up_down_counter("loan.applications.in_review")
                .with_unit("{application}")
                .build(),
            db_duration: meter
                // OpenTelemetry database client convention.
                .f64_histogram("db.client.operation.duration")
                .with_unit("s")
                .with_boundaries(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0])
                .build(),
            auth_attempts: meter
                .u64_counter("auth.attempts")
                .with_unit("{attempt}")
                .with_description("Authentication attempts by method and outcome.")
                .build(),
            client_duration: meter
                // OpenTelemetry HTTP client convention.
                .f64_histogram("http.client.request.duration")
                .with_unit("s")
                .build(),
            jobs: meter
                .u64_counter("queue.jobs.processed")
                .with_unit("{job}")
                .build(),
            job_wait: meter
                .f64_histogram("queue.job.wait.duration")
                .with_unit("s")
                .build(),
        }
    }
}

struct Job {
    enqueued: Instant,
    kind: &'static str,
}

#[derive(Clone)]
pub struct AppState {
    metrics: Metrics,
    queue: mpsc::Sender<Job>,
    receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<Job>>>,
    queue_depth: Arc<AtomicI64>,
    next_id: Arc<AtomicU64>,
    client: Client<HttpConnector, Empty<Bytes>>,
    propagator: Arc<dyn TextMapPropagator + Send + Sync>,
    self_url: Arc<str>,
    pub inference: Inference,
}

impl AppState {
    pub fn new(observability: &Observability, port: u16) -> Self {
        let queue_depth = Arc::new(AtomicI64::new(0));
        // Bounded: a full queue rejects work instead of growing.
        let (tx, rx) = mpsc::channel(1_000);
        Self {
            metrics: Metrics::new(&observability.meter("kerniq-demo"), queue_depth.clone()),
            queue: tx,
            receiver: Arc::new(tokio::sync::Mutex::new(rx)),
            queue_depth,
            next_id: Arc::new(AtomicU64::new(1)),
            client: Client::builder(TokioExecutor::new()).build_http(),
            propagator: observability.propagator(),
            self_url: format!("http://127.0.0.1:{port}").into(),
            inference: Inference::new(observability),
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .post("/payments", create_payment)
        .get("/accounts/{id}", get_account)
        .post("/loans", apply_for_loan)
        .post("/login", login)
        .get("/internal/fx-rate", fx_rate)
}

#[derive(Deserialize)]
struct PaymentQuery {
    #[serde(rename = "type", default = "default_type")]
    kind: String,
    #[serde(default = "default_amount")]
    amount_minor: u64,
    #[serde(default = "default_currency")]
    currency: String,
}

fn default_type() -> String {
    "transfer".into()
}
fn default_amount() -> u64 {
    1_000
}
fn default_currency() -> String {
    "USD".into()
}

/// A traced database call recorded with the OpenTelemetry DB conventions.
async fn db_call(metrics: &Metrics, operation: &'static str, latency: Duration) {
    let span = tracing::info_span!(
        "db.query",
        otel.kind = "client",
        db.system.name = "postgresql",
        db.operation.name = operation
    );
    let start = Instant::now();
    tracing::Instrument::instrument(tokio::time::sleep(latency), span).await;
    metrics.db_duration.record(
        start.elapsed().as_secs_f64(),
        &[
            KeyValue::new("db.system.name", "postgresql"),
            KeyValue::new("db.operation.name", operation),
        ],
    );
}

/// An outbound HTTP call with W3C context injected, so the callee's span
/// joins this trace.
async fn fetch_fx_rate(state: &AppState, currency: &'static str) -> Option<String> {
    let span = tracing::info_span!(
        "GET",
        otel.kind = "client",
        otel.name = "GET /internal/fx-rate",
        http.request.method = "GET",
        server.address = "127.0.0.1",
        http.response.status_code = tracing::field::Empty,
    );
    let start = Instant::now();
    let result = tracing::Instrument::instrument(
        async {
            let mut request = http::Request::get(format!(
                "{}/internal/fx-rate?currency={currency}",
                state.self_url
            ))
            .body(Empty::<Bytes>::new())
            .ok()?;
            inject_current_context(state.propagator.as_ref(), request.headers_mut());
            let response = state.client.request(request).await.ok()?;
            tracing::Span::current()
                .record("http.response.status_code", response.status().as_u16());
            let body = response.into_body().collect().await.ok()?.to_bytes();
            Some(String::from_utf8_lossy(&body).into_owned())
        },
        span,
    )
    .await;
    state.metrics.client_duration.record(
        start.elapsed().as_secs_f64(),
        &[
            KeyValue::new("http.request.method", "GET"),
            KeyValue::new("server.address", "fx-service"),
        ],
    );
    result
}

async fn create_payment(
    State(state): State<AppState>,
    Query(q): Query<PaymentQuery>,
) -> Result<Response> {
    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let kind = TX_TYPE.get(&q.kind);
    let currency = CURRENCY.get(&q.currency);
    // Identifiers and amounts belong on spans and logs, not metric labels.
    tracing::info!(
        transaction_id = id,
        amount_minor = q.amount_minor,
        currency,
        kind,
        "payment received"
    );

    db_call(&state.metrics, "SELECT", Duration::from_millis(3)).await;
    if currency != "USD" {
        fetch_fx_rate(&state, currency).await;
    }

    let outcome = if q.amount_minor > 5_000_000 {
        "rejected"
    } else {
        "accepted"
    };
    if outcome == "accepted" {
        db_call(&state.metrics, "INSERT", Duration::from_millis(5)).await;
        let job = Job {
            enqueued: Instant::now(),
            kind: "settlement",
        };
        if state.queue.try_send(job).is_ok() {
            state.queue_depth.fetch_add(1, Ordering::Relaxed);
        } else {
            tracing::warn!(transaction_id = id, "settlement queue full");
        }
    }
    let attrs = [
        KeyValue::new("transaction.type", kind),
        KeyValue::new("currency", currency),
        KeyValue::new("outcome", outcome),
    ];
    state.metrics.transactions.add(1, &attrs);
    state
        .metrics
        .transaction_amount
        .record(q.amount_minor, &attrs[..2]);

    if outcome == "rejected" {
        tracing::warn!(transaction_id = id, "payment rejected: limit exceeded");
        return Err(Error::unprocessable("amount exceeds the transaction limit"));
    }
    Ok((StatusCode::CREATED, format!("{{\"transaction_id\":{id}}}")).into_response())
}

async fn get_account(State(state): State<AppState>, Path(id): Path<u64>) -> Result<String> {
    db_call(&state.metrics, "SELECT", Duration::from_millis(2)).await;
    if id == 0 {
        return Err(Error::not_found("no such account"));
    }
    Ok(format!("{{\"id\":{id},\"balance_minor\":125000}}"))
}

#[derive(Deserialize)]
struct LoanQuery {
    #[serde(default)]
    product: String,
    #[serde(default)]
    amount_minor: u64,
}

#[tracing::instrument(skip_all, fields(loan.product))]
async fn apply_for_loan(
    State(state): State<AppState>,
    Query(q): Query<LoanQuery>,
) -> (StatusCode, &'static str) {
    let product = LOAN_PRODUCT.get(&q.product);
    tracing::Span::current().record("loan.product", product);
    let m = &state.metrics;
    let start = Instant::now();
    m.loans_in_review
        .add(1, &[KeyValue::new("loan.product", product)]);

    // Credit check and affordability, as two child spans.
    tracing::Instrument::instrument(
        tokio::time::sleep(Duration::from_millis(15)),
        tracing::info_span!("credit_check"),
    )
    .await;
    db_call(m, "SELECT", Duration::from_millis(4)).await;
    let decision = if q.amount_minor > 50_000_000 {
        "declined"
    } else {
        "approved"
    };

    m.loans_in_review
        .add(-1, &[KeyValue::new("loan.product", product)]);
    let attrs = [
        KeyValue::new("loan.product", product),
        KeyValue::new("loan.decision", decision),
    ];
    m.loan_decision_duration
        .record(start.elapsed().as_secs_f64(), &attrs);
    m.loan_applications.add(1, &attrs);
    tracing::info!(decision, "loan decision");
    (StatusCode::OK, decision)
}

#[derive(Deserialize)]
struct LoginQuery {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

// `password` is never recorded: `skip_all` keeps arguments off the span, and
// only the username is logged.
#[tracing::instrument(skip_all)]
async fn login(State(state): State<AppState>, Query(q): Query<LoginQuery>) -> StatusCode {
    tracing::info!(username = %q.username, "Authentication request received");
    let ok = !q.username.is_empty() && q.password.len() >= 8;
    let outcome = if ok { "success" } else { "failure" };
    state.metrics.auth_attempts.add(
        1,
        &[
            KeyValue::new("auth.method", "password"),
            KeyValue::new("outcome", outcome),
        ],
    );
    if ok {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::UNAUTHORIZED
    }
}

async fn fx_rate(Query(q): Query<std::collections::HashMap<String, String>>) -> String {
    tokio::time::sleep(Duration::from_millis(8)).await;
    let currency = q.get("currency").map_or("USD", |c| CURRENCY.get(c));
    tracing::debug!(currency, "fx rate served");
    format!("{{\"currency\":\"{currency}\",\"rate\":1.08}}")
}

/// Processes settlement jobs until shutdown.
pub async fn queue_worker(state: AppState, shutdown: Shutdown) {
    let mut receiver = state.receiver.lock().await;
    loop {
        let job = tokio::select! {
            job = receiver.recv() => match job { Some(job) => job, None => break },
            () = shutdown.wait() => break,
        };
        state.queue_depth.fetch_sub(1, Ordering::Relaxed);
        let attrs = [
            KeyValue::new("queue.name", "settlement"),
            KeyValue::new("job.kind", job.kind),
        ];
        state
            .metrics
            .job_wait
            .record(job.enqueued.elapsed().as_secs_f64(), &attrs);
        let span = tracing::info_span!("settle", otel.kind = "consumer", job.kind = job.kind);
        tracing::Instrument::instrument(tokio::time::sleep(Duration::from_millis(20)), span).await;
        state.metrics.jobs.add(
            1,
            &[
                attrs[0].clone(),
                attrs[1].clone(),
                KeyValue::new("outcome", "success"),
            ],
        );
    }
}
