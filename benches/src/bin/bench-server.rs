//! Serves one benchmark scenario over HTTP.
//!
//! ```sh
//! bench-server <scenario> <kerniq|axum> [address] [worker-threads]
//! ```
//!
//! `kerniq` serves through `App::run` (Kerniq's accept loop, graceful
//! shutdown, `ConnectInfo`); `axum` serves the equivalent Axum router with
//! `axum::serve`. Both use a multi-threaded Tokio runtime with the same
//! number of workers.

use kerniq_benches::SCENARIOS;
use kerniq_server::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (scenario, framework) = (
        args.get(1).map(String::as_str),
        args.get(2).map(String::as_str),
    );
    let address = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:8090".into());
    let workers: usize = args.get(4).and_then(|w| w.parse().ok()).unwrap_or(4);

    let Some((_, kerniq, axum, _)) = SCENARIOS.iter().find(|s| Some(s.0) == scenario) else {
        let names: Vec<_> = SCENARIOS.iter().map(|s| s.0).collect();
        eprintln!(
            "usage: bench-server <{}> <kerniq|axum> [address] [workers]",
            names.join("|")
        );
        std::process::exit(2);
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async move {
        match framework {
            Some("kerniq") => {
                // `KERNIQ_*` variables (e.g. KERNIQ_CONNECT_INFO=false) apply.
                let config = ServerConfig::from_env().expect("valid KERNIQ_* configuration");
                App::new()
                    .config(config)
                    .mount_axum("/", kerniq())
                    .run(&address)
                    .await
                    .expect("kerniq server");
            }
            Some("axum") => {
                let listener = tokio::net::TcpListener::bind(&address).await.expect("bind");
                axum::serve(listener, axum())
                    .with_graceful_shutdown(async {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await
                    .expect("axum server");
            }
            _ => {
                eprintln!("framework must be `kerniq` or `axum`");
                std::process::exit(2);
            }
        }
    });
}
