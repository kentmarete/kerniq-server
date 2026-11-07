//! Kerniq and Axum together, in both directions.
//!
//! ```sh
//! cargo run -p example-axum-integration
//! curl localhost:3000/kerniq/hello     # Kerniq app nested in an Axum app
//! curl localhost:3000/kerniq/legacy/ping  # Axum router mounted in Kerniq
//! curl localhost:3000/status           # plain Axum route
//! ```

use std::time::Duration;

use axum::routing::get;
use kerniq_server::prelude::*;
use tower_http::timeout::TimeoutLayer;

/// An existing Axum router, with its own handlers and Tower layers.
fn legacy_router() -> axum::Router {
    axum::Router::new()
        .route("/ping", get(|| async { "pong from axum" }))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
}

/// An Axum handler using Axum's own extractors works in Kerniq unchanged.
async fn axum_style(axum::extract::Path(name): axum::extract::Path<String>) -> String {
    format!("hello, {name} (axum extractor)")
}

#[tokio::main]
async fn main() -> Result<()> {
    // Kerniq app that mounts an Axum router and uses an Axum handler.
    let kerniq = App::new()
        .get("/hello", || async { "hello from kerniq" })
        .get("/greet/{name}", axum_style)
        .mount_axum("/legacy", legacy_router());

    // Convert the Kerniq app into an Axum router and nest it in an Axum app.
    let app = axum::Router::new()
        .route("/status", get(|| async { "axum status: ok" }))
        .nest("/kerniq", kerniq.into_axum_router());

    // Serve with plain `axum::serve`. (`App::run` would add graceful shutdown,
    // connection limits and background tasks; here Axum is in charge.)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await?;
    Ok(())
}
