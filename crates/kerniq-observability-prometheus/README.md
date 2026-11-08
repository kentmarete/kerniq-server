# kerniq-observability-prometheus

[![crates.io](https://img.shields.io/crates/v/kerniq-observability-prometheus.svg)](https://crates.io/crates/kerniq-observability-prometheus)
[![docs.rs](https://img.shields.io/docsrs/kerniq-observability-prometheus)](https://docs.rs/kerniq-observability-prometheus)

Prometheus scrape endpoint for Kerniq Server, backed by the OpenTelemetry Prometheus exporter: on the application listener or a separate one, collected on demand, encoded off the async workers.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio, and of its observability
layer, Kerniq Observe.

> **Most applications should not depend on this crate directly.** Enable the
> `observability-*` features of [`kerniq-server`](https://crates.io/crates/kerniq-server)
> and use `kerniq_server::observability`.


## Documentation

* [Observability overview](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/overview.md)
* [Architecture](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/architecture.md)
* [API reference](https://docs.rs/kerniq-observability-prometheus)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
