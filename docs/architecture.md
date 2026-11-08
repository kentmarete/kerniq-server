# Architecture

Kerniq Server is a thin, opinionated layer over the established Rust HTTP
stack. This document records *what* Kerniq builds on, *what* it adds, and the
decisions behind its public API, including where the original specification
was changed because Rust or the underlying libraries required it.

## Foundations

| Concern | Provided by | Kerniq's role |
|---|---|---|
| Async execution | Tokio | Spawns tasks; owns a runtime only in `run_blocking` |
| HTTP/1.1 and HTTP/2 | Hyper (via `hyper-util`'s auto connection builder) | Accept loop, limits, graceful drain |
| Routing, extraction, handlers | Axum | Ergonomic registration, prefixes, groups, mounting |
| Middleware | Tower / tower-http | Ordered, deferred composition; production middleware |
| Serialization | Serde / serde_json | Error envelope, `Json<T>` |
| Errors | thiserror | Startup/config error enums |
| Diagnostics | tracing | Request spans, lifecycle events |
| OpenAPI | Utoipa | Serving documents and Swagger UI |
| Validation | validator | `ValidatedJson`, `ValidatedQuery` |

Kerniq contains **no** HTTP parser, **no** executor and **no** routing engine.
Every request is matched by Axum's router and executed by Hyper on Tokio.

## Crates

```text
                     kerniq-core
                (Error, ServerConfig, ServerError)
                 ▲          ▲            ▲
                 │          │            │
           kerniq-http  kerniq-routing  kerniq-runtime
          (extractors,  (Router, Group, (accept loop,
           responses)    Middleware)     Shutdown, tasks)
                 ▲          ▲   ▲        ▲
                 │          │   │        │
                 │  kerniq-middleware    │
                 │  (Logger, Cors, ...)  │
                 │          ▲            │
                 │          │            │   kerniq-openapi
                 │          │            │   (routing + utoipa)
                 └──────────┴──── kerniq-server ◀──┘ (optional)
                               (App, prelude, re-exports)

  kerniq-cli — independent; generates projects that depend on kerniq-server
```

The graph is acyclic. Each crate exists because it has a distinct
responsibility and dependency profile:

* **kerniq-core** depends only on `axum-core`, `http`, Serde and
  `thiserror`, so every other crate can share the error model cheaply.
* **kerniq-http** is the only crate that knows about request parsing.
* **kerniq-routing** owns the `Router` and the `Middleware` trait, because
  both `Router` and `App` compose middleware the same way.
* **kerniq-runtime** is the only crate that touches sockets and signals.
* **kerniq-middleware** keeps optional, dependency-heavy middleware (CORS,
  compression, tracing) behind features, away from minimal builds.
* **kerniq-openapi** isolates Utoipa (a proc-macro dependency).
* **kerniq-server** is the facade users depend on.
* **kerniq-cli** shares no code with the runtime library.

### Refinements to the proposed layout

* The proposal placed `application.rs` (the `App`) in `kerniq-core`. `App`
  needs routing, middleware and the runtime, so placing it in core would
  create a cycle. It lives in `kerniq-server` (`builder.rs`).
* `context.rs`, `state.rs` and `lifecycle.rs` in core were dropped: Axum's
  `State`/`FromRef` already solve state, and lifecycle code belongs in
  `kerniq-runtime`. Adding wrappers would have been abstraction without value.
* Integration tests live in `crates/kerniq-server/tests/` (they test the
  public API), benchmarks in a dedicated `benches/` package, and examples are
  workspace packages under `examples/`.

## Key decisions

### 1. Handlers are Axum handlers

Kerniq does not define a handler trait. `App::get(path, handler)` accepts any
`axum::handler::Handler<T, S>`, so every Axum extractor, response type and
third-party integration works unchanged. Kerniq's own extractors (`Path`,
`Query`, `Json`, `Header`, ...) are ordinary `FromRequestParts`/`FromRequest`
implementations.

**What Kerniq adds:** its `Path`, `Query` and `Json` delegate to Axum's and
differ only in their rejection type, which is the Kerniq JSON error. That
gives one error format for the whole application without re-implementing
parsing.

### 2. Router ownership and conversion

`kerniq_routing::Router<S>` wraps an `axum::Router<S>` plus:

* a **prefix**, applied when the router is mounted or converted,
* a **middleware stack**, applied when the router is finalized,
* a list of `RouteInfo` records (build-time only) for diagnostics.

Routes are registered directly into the inner Axum router. Mounting uses
Axum's `nest` (or `merge` at the root), which *flattens* routes into the
parent's table, so composing routers has no per-request cost and no router is
reconstructed per request.

Conversions:

| From | To | How |
|---|---|---|
| `App<S>` | `axum::Router` | `app.into_axum_router()` (fallbacks, state and middleware applied) |
| `Router<S>` | `axum::Router<S>` | `router.into_axum()` or `.into()` |
| `axum::Router<S>` | `Router<S>` | `Router::from(axum_router)` |
| `axum::Router<S>` | part of an `App<S>` | `app.mount_axum(path, axum_router)` |

### 3. `mount` cannot be overloaded

The specification shows both `.mount(api)` and `.mount("/users", users)`.
Rust has no overloading, so Kerniq follows Axum's vocabulary:

* `mount(path, router)` — mount under a path (combined with the router's own
  prefix),
* `merge(router)` — mount at the router's own prefix.

### 4. Route groups

`Router::group(|router| { router.get(..); router.post(..); })` matches the
specification exactly. The closure receives a `Group` whose methods take
`&mut self` and return `&mut Self`, so both statement and chained styles
work. (`Router` methods take `self` by value for fluent chaining; the
by-reference `Group` exists because a closure cannot return a moved builder
from a block of statements.)

### 5. Path syntax

Axum 0.8 uses `{id}` and `{*rest}` and *panics* on `:id` / `*rest`. Kerniq
accepts both and rewrites the legacy syntax at registration time. Nothing is
lost (a segment beginning with `:` or `*` cannot be a literal in Axum 0.8) and
there is no per-request cost. Documentation uses `{id}` as canonical.

### 6. Generic application state

`App::new()` is `App<()>`. `App::state(value)` turns it into `App<T>`;
routes registered before the call (which cannot use `State<T>`) are kept by
converting the inner router with Axum's `with_state`. State is provided once,
when the app is finalized. Handlers extract it with `State<T>`, and sub-states
work through Axum's `FromRef`. Kerniq does not wrap state in an `Arc`
automatically: Axum clones the state for each extraction, so applications put
expensive resources behind `Arc` (connection pools already are).

### 7. Middleware ordering and scope

Axum's `Router::layer` wraps only the routes that exist *when it is called*.
`App::new().middleware(Logger::new()).get("/", h)`, the style in the
specification, would silently not log `/`. Kerniq therefore records middleware
in a `MiddlewareStack` and applies it at finalization:

* middleware applies to all routes regardless of declaration order,
* **the first middleware declared is the outermost**,
* app-level middleware also wraps the 404 fallback,
* router-level middleware wraps only that router's routes.

Middleware is attached with `Router::layer`, i.e. after routing, per route.
`MatchedPath` and path parameters are therefore visible to middleware, and any
state that must be global (semaphores, rate-limit buckets, metrics) lives
behind an `Arc` in the layer. This is why `ConcurrencyLimit` uses Tower's
`GlobalConcurrencyLimitLayer`.

`.middleware(m)` accepts Kerniq's `Middleware` trait, which lets one
configured unit apply several layers. `.layer(l)` accepts any Tower layer and
is exactly `.middleware(TowerLayer(l))`; both share the same ordered stack.
There is one stack and one ordering rule, and two entry points: one in Tower
vocabulary, one for Kerniq bundles.

### 8. Tokio runtime ownership

* `app.run(addr).await` and `app.serve(listener).await` run on the caller's
  runtime.
* `app.run_blocking(addr)` builds a multi-threaded runtime, honoring
  `ServerConfig::workers`. If a runtime is already active on the thread it
  returns `ServerError::NestedRuntime` instead of panicking.

`ServerConfig::workers` is ignored, with a warning, under `run`: the
application owns that runtime.

### 9. The accept loop

`axum::serve` does not support a connection limit or a bounded drain, so
`kerniq-runtime` drives `hyper-util`'s auto (HTTP/1 + HTTP/2) connection
builder directly:

1. acquire a connection permit (if `max_connections` is set),
2. accept, set `TCP_NODELAY`, optionally insert `ConnectInfo`,
3. serve the connection with upgrades (WebSockets), watched by
   `GracefulShutdown`.

On shutdown the listener is dropped, so new connections are refused. Open
connections are asked to finish (HTTP/1 after the in-flight response, HTTP/2
via `GOAWAY`). After `shutdown_timeout` the remaining connections are closed.
Background tasks then get up to `shutdown_timeout` to finish.

### 10. Feature boundaries

| Feature | Pulls in |
|---|---|
| `json` (default) | `axum/json` |
| `tracing` (default) | `tower-http/trace` (`Logger`) |
| `cors` | `tower-http/cors` |
| `compression` | `tower-http/compression-full` (+ codecs) |
| `websocket` | `axum/ws` (+ tungstenite) |
| `openapi` | `kerniq-openapi`, `utoipa` |
| `validation` | `validator` |
| `config-file` | `toml` |

The `tracing` crate itself is always present. It is small, has no cost
without a subscriber, and Axum, Hyper and Tower already depend on it, so
making it optional would not shrink builds. A build with no default features
has 91 crates in its dependency tree; one with every feature has 192.

### 11. Error model

A single `Error` type, one pointer wide, carries a status, a machine code, a
public message, optional details and headers, and an optional source. The
source is logged and never serialized. There is deliberately **no** blanket
`impl<E: std::error::Error> From<E> for Error`: it would forbid applications
from writing `impl From<DomainError> for Error`, and it would silently turn
every domain error into a 500. `ResultExt::or_internal()` makes that
conversion explicit instead.

### 12. OpenAPI

Utoipa generates documents at compile time. `kerniq-openapi` serves the
document (serialized once) and a Swagger UI page that loads
`swagger-ui-dist` from a CDN, which avoids `utoipa-swagger-ui`'s build-time
asset download. Air-gapped deployments can mount `utoipa-swagger-ui`
through `mount_axum`.

### 13. Observability

Kerniq Observe (`kerniq-observability` and its satellite crates) is an
optional layer over OpenTelemetry, `tracing` and Prometheus, reached through
`App::observability` behind the `observability*` features. It has its own
architecture document: [docs/observability/architecture.md](observability/architecture.md).
