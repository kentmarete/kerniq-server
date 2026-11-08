# kerniq-core

[![crates.io](https://img.shields.io/crates/v/kerniq-core.svg)](https://crates.io/crates/kerniq-core)
[![docs.rs](https://img.shields.io/docsrs/kerniq-core)](https://docs.rs/kerniq-core)

Core types shared by every Kerniq Server crate: the `Error` model and its JSON envelope, `ServerConfig` (builder, `KERNIQ_*` environment variables, TOML files) and `ServerError`.

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
* [Handlers, responses and errors](https://kerniq-server.com/handlers.html)
* [Configuration](https://kerniq-server.com/configuration.html)
* [API reference for `kerniq_core`](https://docs.rs/kerniq-core)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
