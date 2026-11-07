# Contributing to Kerniq Server

Thank you for helping. This guide covers the workflow and the rules every
change follows.

## Setup

```sh
git clone <repository> && cd kerniq-server
cargo test --workspace
```

The toolchain is pinned to stable with `rustfmt` and `clippy`
(`rust-toolchain.toml`). The minimum supported Rust version is 1.88.

## Quality gates

Every pull request must pass all of these:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo test -p kerniq-server --no-default-features
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

If you change a CLI template, also generate every template against your
checkout and run `kerniq check` and `kerniq test` in each project:

```sh
cargo build -p kerniq-cli
KERNIQ="$PWD/target/debug/kerniq"
OUT=$(mktemp -d)
for t in basic rest modular production; do
  "$KERNIQ" new "demo-$t" --template "$t" --kerniq-path "$PWD" --dir "$OUT"
  (cd "$OUT/demo-$t" && "$KERNIQ" check && "$KERNIQ" test)
done
```

## Design rules

1. **Reuse the stack.** Tokio executes, Hyper speaks HTTP, Axum routes and
   extracts, Tower composes. Do not reimplement any of them.
2. **Justify abstractions.** Before adding a type, state the problem it
   solves and why Axum or Tower does not already solve it. Record important
   decisions in `docs/architecture.md`.
3. **Stay interoperable.** Anything Axum or Tower accepts should keep
   working. Never introduce a handler trait or a closed middleware model.
4. **One way to do a thing**, unless there is a documented reason.
5. **No placeholders.** No `todo!()`, no settings the stack cannot honor, no
   features without tests.
6. **Measure performance claims.** Changes on the request path should come
   with `alloc-count` and criterion numbers (`cargo bench -p kerniq-benches`).
   Never commit fabricated or unreproducible results.
7. **Errors never leak internals.** Client-visible messages are public text;
   sources are logged.
8. **Keep the dependency graph acyclic**, and keep optional dependencies
   behind features.

## Documentation

Public items need doc comments with examples. The guides in `docs/` and the
README are compiled as doctests, so snippets must stay correct. Use
`rust,no_run` for snippets that start a server.

## Commits and changelog

Write focused commits with descriptive messages. Add user-visible changes to
`CHANGELOG.md` under "Unreleased".

## License

By contributing you agree that your contributions are dual-licensed under
MIT OR Apache-2.0, as described in `LICENSE`.
