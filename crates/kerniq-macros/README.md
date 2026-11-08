# kerniq-macros

[![crates.io](https://img.shields.io/crates/v/kerniq-macros.svg)](https://crates.io/crates/kerniq-macros)
[![docs.rs](https://img.shields.io/docsrs/kerniq-macros)](https://docs.rs/kerniq-macros)

Procedural macros for Kerniq Server, currently the `#[authorize(..)]` role and permission attribute for handlers. Its expansions refer to `::kerniq_server`, so use it through the `macros` feature of `kerniq-server`.

This crate is part of [Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous
web framework for Rust built on Axum, Tower, Hyper and Tokio.

> **Most applications should not depend on this crate directly.** Depend on
> [`kerniq-server`](https://crates.io/crates/kerniq-server), which re-exports
> everything here through `kerniq_server::prelude`. The internal `kerniq-*`
> crates are versioned together with `kerniq-server`, but only its API is
> covered by the [migration guide](https://kerniq-server.com/migration.html).

```toml
[dependencies]
kerniq-server = { version = "0.1", features = ["macros"] }
```

## Documentation

* [Kerniq Server guides](https://kerniq-server.com/)
* [Authorization](https://kerniq-server.com/authorization.html)
* [API reference for `kerniq_macros`](https://docs.rs/kerniq-macros)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
