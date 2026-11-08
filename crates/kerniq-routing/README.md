# kerniq-routing

[![crates.io](https://img.shields.io/crates/v/kerniq-routing.svg)](https://crates.io/crates/kerniq-routing)
[![docs.rs](https://img.shields.io/docsrs/kerniq-routing)](https://docs.rs/kerniq-routing)

Routing for Kerniq Server: Flask-style `Router` with prefixes, groups and mounting, and deferred, order-preserving middleware composition, all on top of Axum's router.

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
* [Routing](https://kerniq-server.com/routing.html)
* [Middleware](https://kerniq-server.com/middleware.html)
* [API reference for `kerniq_routing`](https://docs.rs/kerniq-routing)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
