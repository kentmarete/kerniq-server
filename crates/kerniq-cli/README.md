# kerniq-cli

[![crates.io](https://img.shields.io/crates/v/kerniq-cli.svg)](https://crates.io/crates/kerniq-cli)

`kerniq`: project scaffolding and development commands for
[Kerniq Server](https://kerniq-server.com/), a lightweight, asynchronous web
framework for Rust built on Axum, Tower, Hyper and Tokio.

## Install

```sh
cargo install kerniq-cli
```

## Usage

```sh
kerniq new customer-api --template production
cd customer-api
kerniq dev      # run, rebuild and restart on change
kerniq check    # fmt --check, check, clippy -D warnings
kerniq test
kerniq build
```

Templates: `basic`, `rest`, `modular` and `production`. Every template
compiles, passes `kerniq check`, and passes its own tests immediately after
generation. Use `--kerniq-path <checkout>` to generate a project that depends
on a local Kerniq Server checkout instead of crates.io.

The CLI does not link against the framework: it generates projects that
depend on [`kerniq-server`](https://crates.io/crates/kerniq-server).

## Documentation

* [Getting started](https://kerniq-server.com/getting-started.html)
* [Kerniq Server guides](https://kerniq-server.com/)
* [Repository](https://github.com/kerniqq/kerniq-server)

## License

Licensed under either of
[Apache License 2.0](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/kerniqq/kerniq-server/blob/main/LICENSE-MIT),
at your option.
