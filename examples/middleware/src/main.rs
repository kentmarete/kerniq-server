//! Built-in middleware and configuration-driven limits.
//!
//! Middleware runs in declaration order: the first one declared is the
//! outermost (sees the request first and the response last).
//!
//! ```sh
//! RUST_LOG=info cargo run -p example-middleware
//! curl -i localhost:3000/
//! curl -i localhost:3000/slow                         # 408 after 2 s
//! for i in 1 2 3 4 5 6; do curl -s -o /dev/null -w '%{http_code}\n' localhost:3000/; done  # 429s
//! curl -s localhost:3000/metrics
//! ```

use std::time::Duration;

use kerniq_server::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let metrics = Metrics::new();
    let registry = metrics.registry();

    let config = ServerConfig::new()
        .request_timeout(Duration::from_secs(2)) // -> Timeout middleware
        .max_concurrent_requests(256) // -> ConcurrencyLimit middleware
        .max_body_size(64 * 1024); // -> BodyLimit middleware

    let app = App::new()
        .config(config)
        .middleware(RequestId::new()) // outermost: every log line has the id
        .middleware(Logger::new())
        .middleware(metrics)
        .middleware(SecurityHeaders::new())
        .middleware(Cors::default())
        .middleware(Compression::new())
        .middleware(RateLimit::per_second(5).by_ip())
        .get("/", || async { "Hello, middleware" })
        .get("/slow", || async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            "too late"
        })
        .get("/metrics", move || {
            let registry = registry.clone();
            async move { registry.render() }
        });

    app.run("127.0.0.1:3000").await
}
