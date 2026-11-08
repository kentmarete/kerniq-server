# kerniq-telemetry

[![crates.io](https://img.shields.io/crates/v/kerniq-telemetry.svg)](https://crates.io/crates/kerniq-telemetry)
[![docs.rs](https://img.shields.io/docsrs/kerniq-telemetry)](https://docs.rs/kerniq-telemetry)

Shared telemetry conventions for Kerniq: OpenTelemetry semantic-convention metric and attribute names, bounded-label helpers, service resource metadata, and reusable AI inference instruments (time to first token, token throughput, active streams, cancellations). Depends only on the OpenTelemetry API.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio, and of its observability
layer, Kerniq Observe.


## Documentation

* [Observability overview](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/overview.md)
* [Architecture](https://github.com/kerniqq/kerniq-server/blob/main/docs/observability/architecture.md)
* [API reference](https://docs.rs/kerniq-telemetry)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
