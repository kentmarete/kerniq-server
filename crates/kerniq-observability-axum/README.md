# kerniq-observability-axum

[![crates.io](https://img.shields.io/crates/v/kerniq-observability-axum.svg)](https://crates.io/crates/kerniq-observability-axum)
[![docs.rs](https://img.shields.io/docsrs/kerniq-observability-axum)](https://docs.rs/kerniq-observability-axum)

Tower/Axum HTTP instrumentation: OpenTelemetry HTTP server metrics with route templates, a request span continuing the W3C trace context, and streaming-aware measurement (time to first byte, stream lifetime, client disconnects) without buffering bodies. Depends on the OpenTelemetry API only.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio, and of its observability
layer, Kerniq Observe.

> **Most applications should not depend on this crate directly.** Enable the
> `observability-*` features of [`kerniq-server`](https://crates.io/crates/kerniq-server)
> and use `kerniq_server::observability`.

Works with plain Axum too: apply `HttpInstrumentation::new().meter(&meter).into_layer()` with `Router::layer`.

Features: `metrics`, `tracing` (default).

## Documentation

* [Observability overview](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/overview.md)
* [Architecture](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/architecture.md)
* [API reference](https://docs.rs/kerniq-observability-axum)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
