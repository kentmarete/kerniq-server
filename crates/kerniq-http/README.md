# kerniq-http

[![crates.io](https://img.shields.io/crates/v/kerniq-http.svg)](https://crates.io/crates/kerniq-http)
[![docs.rs](https://img.shields.io/docsrs/kerniq-http)](https://docs.rs/kerniq-http)

Request extractors and response types for Kerniq Server: `Path`, `Query`, `Json` with Kerniq error responses, typed `Header<T>`, `ValidatedJson` / `ValidatedQuery` (feature `validation`), `Created` and `NoContent`.

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
* [Extractors](https://kerniq-server.com/extractors.html)
* [Handlers, responses and errors](https://kerniq-server.com/handlers.html)
* [WebSockets](https://kerniq-server.com/websockets.html)
* [API reference for `kerniq_http`](https://docs.rs/kerniq-http)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
