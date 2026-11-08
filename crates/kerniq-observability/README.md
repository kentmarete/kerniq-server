# kerniq-observability

[![crates.io](https://img.shields.io/crates/v/kerniq-observability.svg)](https://crates.io/crates/kerniq-observability)
[![docs.rs](https://img.shields.io/docsrs/kerniq-observability)](https://docs.rs/kerniq-observability)

Vendor-neutral metrics, structured logs and distributed tracing for Kerniq Server: OpenTelemetry metrics, logs and traces, OTLP export and Prometheus scraping behind one builder, with bounded queues, drop accounting and bounded shutdown.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio, and of its observability
layer, Kerniq Observe.

> **Most applications should not depend on this crate directly.** Enable the
> `observability-*` features of [`kerniq-server`](https://crates.io/crates/kerniq-server)
> and use `kerniq_server::observability`.

```toml
[dependencies]
kerniq-server = { version = "0.1", features = ["full-observability"] }
```

```rust,ignore
let observability = Observability::builder()
    .service_name("payment-api")
    .metrics(MetricsConfig::prometheus())
    .logs(LogsConfig::json())
    .traces(TracesConfig::otlp().sampling_ratio(0.1))
    .build()
    .await?;

App::new().observability(observability).get("/", handler).run("0.0.0.0:3000").await
```

Features: `metrics`, `logs`, `tracing` (default), `otlp`, `otlp-tls`, `prometheus`, `runtime`, `full`.

## Documentation

* [Observability overview](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/overview.md)
* [Architecture](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/architecture.md)
* [API reference](https://docs.rs/kerniq-observability)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
