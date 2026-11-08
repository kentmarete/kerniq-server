# kerniq-runtime

[![crates.io](https://img.shields.io/crates/v/kerniq-runtime.svg)](https://crates.io/crates/kerniq-runtime)
[![docs.rs](https://img.shields.io/docsrs/kerniq-runtime)](https://docs.rs/kerniq-runtime)

Server lifecycle for Kerniq Server, built on Tokio and Hyper: the accept loop with connection limits, bounded graceful drain, shutdown coordination and background tasks.

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
* [Runtime, lifecycle and graceful shutdown](https://kerniq-server.com/runtime.html)
* [API reference for `kerniq_runtime`](https://docs.rs/kerniq-runtime)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
