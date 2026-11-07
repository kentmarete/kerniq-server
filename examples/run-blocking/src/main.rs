//! A synchronous `main`: Kerniq creates and owns the Tokio runtime.
//!
//! ```sh
//! cargo run -p example-run-blocking
//! KERNIQ_WORKERS=2 cargo run -p example-run-blocking
//! ```

use kerniq_server::prelude::*;

fn main() -> Result<()> {
    let config = ServerConfig::from_env()?;
    let address = config.address();
    println!(
        "listening on http://{address} with {} worker threads",
        config
            .workers
            .map_or_else(|| "default".to_owned(), |n| n.to_string())
    );

    App::new()
        .config(config)
        .get("/", || async { "OK" })
        .run_blocking(address)
}
