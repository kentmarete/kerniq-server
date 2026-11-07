//! Typed, shared application state without global variables.
//!
//! The state is cloned for every request that extracts it, so it holds
//! `Arc`s and handles that are cheap to clone. `FromRef` lets handlers
//! extract just the part they need.
//!
//! ```sh
//! cargo run -p example-shared-state
//! curl localhost:3000/health
//! curl -X POST localhost:3000/visits && curl localhost:3000/visits
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kerniq_server::axum::extract::FromRef;
use kerniq_server::prelude::*;
use serde::Serialize;

/// A stand-in for a database pool (pools such as `sqlx::PgPool` are already
/// cheap, `Arc`-based handles).
#[derive(Clone)]
struct Database {
    visits: Arc<AtomicU64>,
}

#[derive(Clone)]
struct AppState {
    database: Database,
    service_name: Arc<str>,
}

impl FromRef<AppState> for Database {
    fn from_ref(state: &AppState) -> Self {
        state.database.clone()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    service: String,
    status: String,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        service: state.service_name.to_string(),
        status: "healthy".into(),
    })
}

/// Extracts only the database via `FromRef`.
async fn record_visit(State(db): State<Database>) -> String {
    let n = db.visits.fetch_add(1, Ordering::Relaxed) + 1;
    format!("visit #{n} recorded")
}

async fn visits(State(db): State<Database>) -> String {
    db.visits.load(Ordering::Relaxed).to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = AppState {
        database: Database {
            visits: Arc::new(AtomicU64::new(0)),
        },
        service_name: "customer-api".into(),
    };

    let app = App::new()
        .state(state)
        .get("/health", health)
        .post("/visits", record_visit)
        .get("/visits", visits);

    println!("listening on http://127.0.0.1:3000");
    app.run("127.0.0.1:3000").await
}
