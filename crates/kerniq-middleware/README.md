# kerniq-middleware

[![crates.io](https://img.shields.io/crates/v/kerniq-middleware.svg)](https://crates.io/crates/kerniq-middleware)
[![docs.rs](https://img.shields.io/docsrs/kerniq-middleware)](https://docs.rs/kerniq-middleware)

Production middleware for Kerniq Server: `Logger`, `RequestId`, `Timeout`, `BodyLimit`, `ConcurrencyLimit`, `Cors`, `Compression`, `SecurityHeaders`, `RateLimit`, `Auth`, `Require` and Prometheus metrics. Every item is ordinary Tower middleware.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio.

> **Most applications should not depend on this crate directly.** Depend on
> [`kerniq-server`](https://crates.io/crates/kerniq-server), which re-exports
> everything here through `kerniq_server::prelude`. The internal `kerniq-*`
> crates are versioned together with `kerniq-server`, but only its API is
> covered by the [migration guide](https://kerniq-server.com/migration.html).

```toml
[dependencies]
kerniq-server = "0.1"
```

## Documentation

* [Kerniq Server guides](https://kerniq-server.com/)
* [Middleware](https://kerniq-server.com/middleware.html)
* [Authorization](https://kerniq-server.com/authorization.html)
* [API reference for `kerniq_middleware`](https://docs.rs/kerniq-middleware)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
