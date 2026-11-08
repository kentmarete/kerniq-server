# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.3] - 2026-11-08

All crates are released together at 0.1.3. The five observability crates
are new on crates.io; the other crates are unchanged apart from the version.

### Added

- **Kerniq Observe**: vendor-neutral observability built on OpenTelemetry
  0.33, `tracing` and Prometheus. New crates `kerniq-observability`
  (builder, providers, OTLP, logging, lifecycle), `kerniq-observability-axum`
  (HTTP instrumentation), `kerniq-observability-prometheus` (scrape
  endpoint), `kerniq-telemetry` (conventions, bounded labels, AI inference
  instruments) and `kerniq-observability-testing` (in-memory exporters and
  assertions).
- `App::observability` behind the new `kerniq-server` features
  `observability`, `observability-metrics`, `observability-logs`,
  `observability-tracing`, `otlp`, `otlp-tls`, `prometheus`,
  `runtime-metrics` and `full-observability`. None is enabled by default
  and `full` does not include them.
- HTTP server metrics following the OpenTelemetry semantic conventions
  (`http.server.request.duration`, `http.server.active_requests`, request and
  response body sizes) plus `kerniq.http.server.time_to_first_byte`, with
  streaming-aware measurement and `error.type` for aborted streams.
- Prometheus scraping (Option A) and OTLP export of metrics, logs and traces
  (Option B), JSON/pretty/compact console logs with trace correlation, W3C
  trace-context and baggage propagation, runtime and process metrics,
  export-pipeline statistics and bounded shutdown.
- `observability/`: Prometheus, OpenTelemetry Collector (default,
  multi-exporter and Datadog), Tempo and Grafana provisioning, four
  dashboards and a Docker Compose environment for both architectures.
- `examples/observability-minimal` and `examples/observability-demo`.
- Guides in `docs/observability/`.
- `benches/run-observability.sh` and observability scenarios in
  `bench-server` and `alloc-count`.

### Fixed

- `loadgen` sent a request before the HTTP/1 connection was ready for it,
  occasionally failing requests with "connection was not ready" when
  response timing varied; it now awaits readiness.

## [0.1.2] - 2026-11-08

All crates are released together at 0.1.2. `kerniq-server` 0.1.1 was
published on its own, against 0.1.0 of the internal crates; 0.1.2 brings
every crate back to the same version.

### Added

- Opt-in authorization: `Authorities` (roles and permissions of an
  identity), `Policy<T>` and the `Authorized<T, P>` extractor, which rejects
  with `401` without an identity and `403` when the policy fails.
- `macros` feature with the `#[authorize(..)]` handler attribute, in the
  spirit of Spring's `@PreAuthorize`: `has_role`, `has_any_role`,
  `has_all_roles`, `has_permission`, `has_any_permission`,
  `has_all_permissions`, `authenticated()`, `check(fn)`, combined with
  `&&`, `||` and `!`. New crate `kerniq-macros`.
- `Require<T>` middleware for router-level role and permission rules
  (`Require::<User>::role("ADMIN")`, `any_role`, `permission`, `check`,
  `policy::<P>()`, `.and`, `.or`, `.message`), the counterpart of Spring
  Security's `requestMatchers(..).hasRole(..)`.
- Guide for connecting databases and message brokers
  (`docs/integrations.md`): PostgreSQL/MySQL/SQLite with `sqlx`, Redis,
  MongoDB, Kafka and RabbitMQ, with a summary in the README.
- Claude Code skill for building services with Kerniq
  (`skills/kerniq-server/SKILL.md`), with install instructions in the
  README.
- `examples/error-handling`: domain errors mapped with `From`, `?`, error
  codes and details, internal errors that never leak, headers such as
  `Retry-After`, and a custom fallback.
- `MIGRATION.md`, a guide to upgrading across breaking releases.
- `#![warn(missing_docs)]` in every library crate, and docs for the
  variants of `RequireFuture` and `RateLimitFuture`.
- A README for every published crate, shown on crates.io and docs.rs.

### Changed

- The homepage is now <https://kerniq-server.com/>, which hosts the guides.
  README links point there (and to GitHub for repository files), so they
  work on crates.io and docs.rs.
- `kerniq-macros` uses `syn` 3.
- Raised the minimum versions of dependencies to their current releases.
- Internal crates depend on each other at `0.1.2`.

### Fixed

- docs.rs builds. `#![feature(doc_auto_cfg)]` was removed from nightly
  Rust (merged into `doc_cfg`), which failed every docs.rs build; the crates
  now use `doc_cfg`. Every library crate builds its docs with all features
  enabled and shows which feature each item needs.

## [0.1.0] - 2026-11-07

First release.

### Added

- `App` builder: route registration for every standard HTTP method, typed
  state (`App<S>`), configuration, middleware, fallbacks, background tasks,
  shutdown signals; `run`, `serve`, `run_blocking`, `into_axum_router`.
- `Router` with prefixes, groups, mounting (`mount`, `merge`, `mount_axum`),
  nested fallbacks, route introspection, and `:id` / `{id}` path syntax.
- Error model: a single `Error` type with a JSON envelope, constructors for
  common statuses, details, headers, logged-but-hidden sources, and
  `ResultExt` / `OptionExt`.
- Extractors with JSON rejections: `Path`, `Query`, `Json`, `Header<T>`
  (`UserAgent`, `ContentType`, `Authorization`, `BearerToken`, `XRequestId`),
  `ValidatedJson`, `ValidatedQuery`; responses `Created`, `NoContent`.
- Deferred, ordered middleware composition (first declared = outermost) for
  both apps and routers; `Middleware` trait and `TowerLayer` adapter.
- Middleware: `Logger`, `RequestId`, `Timeout`, `BodyLimit`,
  `ConcurrencyLimit`, `Cors`, `Compression`, `SecurityHeaders`, `RateLimit`,
  `Auth` / `Authenticator` / `Identity`, `Metrics` (Prometheus format).
- Server runtime: Hyper HTTP/1 + HTTP/2 accept loop with connection limits,
  header read timeout, `TCP_NODELAY`, `ConnectInfo`, graceful shutdown with
  bounded drain, OS signal handling, `BackgroundTasks`.
- `ServerConfig` from code, `KERNIQ_*` environment variables and TOML files.
- WebSocket support (Axum), OpenAPI documents and Swagger UI (Utoipa).
- `kerniq` CLI: `new` (basic, rest, modular, production templates), `dev`,
  `build`, `check`, `test`.
- Twelve runnable examples, guides in `docs/`, and benchmarks against native
  Axum (criterion, allocation counts, real-HTTP load test).

[Unreleased]: https://github.com/kerniqq/kerniq-server/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/kerniqq/kerniq-server/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/kerniqq/kerniq-server/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/kerniqq/kerniq-server/releases/tag/v0.1.0
