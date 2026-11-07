# Migration guide

This guide covers what to change in your code when you upgrade Kerniq Server
across a release with breaking changes. For the full list of changes in every
release, see [CHANGELOG.md](CHANGELOG.md).

## Versioning

Kerniq follows [Semantic Versioning](https://semver.org/). While the version
is `0.x`, Cargo treats the minor version as the breaking one:

| Upgrade | Breaking changes? | Example |
|---|---|---|
| `0.1.x` → `0.1.y` | No. Update with `cargo update -p kerniq-server`. | `0.1.0` → `0.1.3` |
| `0.x` → `0.(x+1)` | Possibly. Read this guide's section for that release. | `0.1.4` → `0.2.0` |
| `1.x` → `2.0` | Possibly. Read this guide's section for that release. | |

Depend on `kerniq-server` rather than the internal `kerniq-*` crates. The
internal crates are versioned together with it, but their APIs are not
covered by this guide.

## Upgrading

1. Read the section below for every release between your version and the
   target version, oldest first.
2. Bump the version in your `Cargo.toml` and run `cargo update -p kerniq-server`.
3. Run `cargo check --all-targets`. Deprecated items warn for at least one
   minor release before they are removed, and each warning names the
   replacement.
4. Run your tests. Behavior changes (status codes, error codes, defaults) are
   listed under **Behavior changes** in each section, because the compiler
   cannot catch them.

## Releases

### 0.1.0

First release. There is nothing to migrate from.

<!--
Template for a release with breaking changes. Copy it above the previous
release, newest first.

### 0.X.0

One or two sentences on what changed and why.

#### Breaking changes

- **`OldName` renamed to `NewName`.**

  ```rust
  // Before
  use kerniq_server::OldName;
  // After
  use kerniq_server::NewName;
  ```

#### Behavior changes

- **Short summary.** What responses or defaults now differ, and how to keep
  the old behavior if that is possible.

#### Deprecations

- `old_fn` is deprecated in favor of `new_fn` and will be removed in 0.Y.0.
-->
