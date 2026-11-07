# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Guide for connecting databases and message brokers
  (`docs/integrations.md`): PostgreSQL/MySQL/SQLite with `sqlx`, Redis,
  MongoDB, Kafka and RabbitMQ, with a summary in the README.

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

[Unreleased]: https://github.com/kerniq/kerniq-server/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kerniq/kerniq-server/releases/tag/v0.1.0
