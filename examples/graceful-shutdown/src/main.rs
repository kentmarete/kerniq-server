//! Graceful shutdown, background tasks and connection draining.
//!
//! ```sh
//! cargo run -p example-graceful-shutdown
//! curl localhost:3000/slow &   # start a 5 s request...
//! # ...then press Ctrl+C (or `kill -TERM <pid>`): the request completes,
//! # the background task stops cleanly, and the process exits.
//! curl -X POST localhost:3000/shutdown   # or trigger shutdown over HTTP
//! ```

use std::time::Duration;

use kerniq_server::prelude::*;

#[derive(Clone)]
struct AppState {
    shutdown: Shutdown,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let config = ServerConfig::new()
        .shutdown_timeout(Duration::from_secs(15)) // max drain time
        .max_connections(10_000);

    let app = App::new().config(config);
    let shutdown = app.shutdown_handle();

    let app = app
        .state(AppState { shutdown })
        .get("/slow", || async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "finished despite shutdown"
        })
        .post("/shutdown", |State(state): State<AppState>| async move {
            state.shutdown.trigger();
            "shutting down"
        })
        .background_task("reporter", |shutdown: Shutdown| async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tokio::select! {
                    _ = tick.tick() => println!("background task: working"),
                    () = shutdown.wait() => {
                        println!("background task: flushing and exiting");
                        break;
                    }
                }
            }
        });

    // Ctrl+C and SIGTERM trigger shutdown by default; `.shutdown_signal(..)`
    // replaces that with any future.
    app.run("127.0.0.1:3000").await
}
