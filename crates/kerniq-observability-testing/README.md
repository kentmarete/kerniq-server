# kerniq-observability-testing

[![crates.io](https://img.shields.io/crates/v/kerniq-observability-testing.svg)](https://crates.io/crates/kerniq-observability-testing)
[![docs.rs](https://img.shields.io/docsrs/kerniq-observability-testing)](https://docs.rs/kerniq-observability-testing)

Deterministic telemetry tests for Kerniq applications: a real `Observability` wired to the OpenTelemetry SDK in-memory exporters, with snapshots and assertions for metrics, spans and logs.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio, and of its observability
layer, Kerniq Observe.

```toml
[dev-dependencies]
kerniq-observability-testing = "0.1"
```

## Documentation

* [Observability overview](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/overview.md)
* [Architecture](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/architecture.md)
* [API reference](https://docs.rs/kerniq-observability-testing)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
