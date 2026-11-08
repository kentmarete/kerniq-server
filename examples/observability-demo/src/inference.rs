//! A simulated LLM endpoint streaming tokens as server-sent events, with the
//! `kerniq_telemetry::inference` conventions.
//!
//! * queue wait: requests wait for one of `CONCURRENCY` model slots,
//! * time to first token, token throughput and total duration are recorded
//!   once per request — never one measurement or log line per token,
//! * a client disconnect drops the response stream, which drops the
//!   `InferenceRequest` guard and records the request as `cancelled`,
//! * the model label is bounded; prompts and output are never recorded.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use kerniq_server::observability::Observability;
use kerniq_server::observability::prelude::*;
use kerniq_server::observability::telemetry::inference::{InferenceMetrics, InferenceRequest};
use kerniq_server::prelude::*;
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::business::AppState;

const CONCURRENCY: usize = 4;
static MODEL: BoundedLabel = BoundedLabel::new(&["chat-small", "chat-large"], "other");

#[derive(Clone)]
pub struct Inference {
    metrics: InferenceMetrics,
    slots: Arc<Semaphore>,
}

impl Inference {
    pub fn new(observability: &Observability) -> Self {
        Self {
            metrics: InferenceMetrics::new(&observability.meter("kerniq-demo.inference")),
            slots: Arc::new(Semaphore::new(CONCURRENCY)),
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().get("/v1/generate", generate)
}

#[derive(Deserialize)]
struct GenerateQuery {
    #[serde(default)]
    model: String,
    #[serde(default = "default_tokens")]
    tokens: u32,
}

fn default_tokens() -> u32 {
    64
}

struct Generation {
    request: InferenceRequest,
    _slot: OwnedSemaphorePermit,
    remaining: u32,
    per_token: Duration,
    first: bool,
}

async fn generate(
    State(state): State<AppState>,
    Query(q): Query<GenerateQuery>,
) -> Result<Response> {
    let inference = state.inference;
    let model = MODEL.get(&q.model);
    let mut request = inference
        .metrics
        .start([KeyValue::new("inference.model", model)]);
    request.input_tokens(32);

    // Queue: wait for a model slot. Bounded wait, then shed load.
    let slot = match tokio::time::timeout(
        Duration::from_secs(10),
        inference.slots.clone().acquire_owned(),
    )
    .await
    {
        Ok(Ok(slot)) => slot,
        _ => {
            request.fail("overloaded");
            return Err(Error::service_unavailable("all model slots are busy"));
        }
    };
    request.execution_started();
    tracing::info!(model, max_tokens = q.tokens, "generation started");

    let (prefill, per_token) = if model == "chat-large" {
        (Duration::from_millis(250), Duration::from_millis(40))
    } else {
        (Duration::from_millis(80), Duration::from_millis(15))
    };
    tokio::time::sleep(prefill).await;

    let state = Generation {
        request,
        _slot: slot,
        remaining: q.tokens.clamp(1, 2_000),
        per_token,
        first: true,
    };
    let stream = futures_util::stream::unfold(Some(state), |state| async move {
        let mut g = state?;
        if g.remaining == 0 {
            tracing::info!(tokens = g.request.generated(), "generation finished");
            g.request.finish();
            let done = Bytes::from_static(b"data: [DONE]\n\n");
            return Some((Ok::<_, Infallible>(done), None));
        }
        if !g.first {
            tokio::time::sleep(g.per_token).await;
        }
        g.first = false;
        g.remaining -= 1;
        g.request.tokens_generated(1);
        Some((Ok(Bytes::from_static(b"data: tok\n\n")), Some(g)))
    });

    Ok((
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
