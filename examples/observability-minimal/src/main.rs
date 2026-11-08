//! The smallest useful setup: HTTP metrics on `/metrics` for Prometheus and
//! human-readable logs. No Collector required.
//!
//! ```text
//! cargo run -p example-observability-minimal
//! curl localhost:3000/users/42
//! curl localhost:3000/metrics
//! ```

use kerniq_server::observability::prelude::*;
use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let observability = Observability::builder()
        .service_name("minimal-api")
        .metrics(MetricsConfig::prometheus())
        .logs(LogsConfig::compact())
        .build()
        .await?;

    App::new()
        .observability(observability)
        .get("/health", || async { "OK" })
        .get("/users/{id}", |Path(id): Path<u64>| async move {
            tracing::info!(user_id = id, "user fetched");
            format!("user {id}")
        })
        .run("127.0.0.1:3000")
        .await
}
