# Performance and benchmarking

Kerniq's goal is to add as little cost as possible on top of Axum. Routing,
extraction and the HTTP protocol are Axum's and Hyper's, so the questions
that matter are:

1. What does Kerniq's composition (prefixes, deferred middleware, fallbacks,
   state) add per request?
2. What does Kerniq's server loop (connection limits, bounded drain,
   slow-header protection) add per request?

Each was measured against an equivalent native Axum application. Kerniq
does not claim to be faster than Axum: it runs on Axum.

## Method

All scenarios are defined once in `benches/src/lib.rs` as Kerniq/Axum pairs
with identical handlers and routes:

| Scenario | Request | Notes |
|---|---|---|
| `minimal` | `GET /` → `"Hello, World!"` | |
| `json` | `GET /accounts/42` → 5-field JSON | `Path<u64>` + `Json` |
| `routes-50` | same, among 50 other parameterized routes | routing table cost |
| `middleware` | `GET /` | request IDs + 4 security headers + 30 s timeout; Kerniq as 3 `.middleware(..)` calls, Axum as one `ServiceBuilder` of tower-http layers |
| `middleware-fused` | `GET /` | the same Kerniq middleware fused into one `ServiceBuilder` |

Three instruments:

* **In-process dispatch** (`cargo bench -p kerniq-benches`): criterion,
  `tower::ServiceExt::oneshot` on a current-thread runtime. No sockets, so it
  isolates framework-layer cost.
* **Allocations** (`cargo run --release -p kerniq-benches --bin alloc-count`):
  a counting global allocator over 10,000 in-process requests.
* **Real HTTP** (`benches/run-http.sh [runs] [secs] [conns] [server workers] [client threads]`):
  `bench-server` (Kerniq via `App::run`, Axum via `axum::serve`) and
  `loadgen` (closed-loop HTTP/1.1 keep-alive client) run as separate
  processes. Server CPU time and RSS come from `ps`. Kerniq and Axum runs
  alternate, and the table shows the median of 3 runs.

Environment for the results below: Apple M3 Pro (11 cores, 18 GiB), macOS
27.2, Rust 1.99.1, release profile (`lto = "thin"`, `codegen-units = 1`).
HTTP runs: 64 connections, 8 s measured after a 1 s warm-up, 4 server
workers, 4 client threads, client and server on the same machine.

Raw outputs are in `benches/results/`. Reproduce them on your own hardware
before relying on any number here.

## Results

### Allocations per request (in-process)

| Scenario | Kerniq allocs/req | Axum allocs/req | Kerniq bytes/req | Axum bytes/req |
|---|---:|---:|---:|---:|
| minimal | 13 | 14 | 1145 | 1153 |
| json | 19 | 20 | 2043 | 2027 |
| routes-50 | 19 | 20 | 2043 | 2027 |
| middleware | 34 | 30 | 5133 | 5437 |
| middleware-fused | 27 | 30 | 4365 | 5437 |

### Dispatch latency (in-process, criterion median)

| Scenario | Kerniq | Axum |
|---|---:|---:|
| minimal | 388 ns | 366 ns |
| json | 642 ns | 640 ns |
| routes-50 | 648 ns | 636 ns |
| middleware | 1.95 µs | 2.70 µs |
| middleware-fused | 2.05 µs | 2.70 µs |

Run-to-run variance on this machine is about ±5%. In a single criterion run
measuring an Axum router, Axum with fallbacks, and Kerniq's minimal app side
by side, all fell between 343 and 365 ns, with Kerniq at 346 ns. The
`minimal` gap above is ordering and code-layout noise, not overhead.

### Real HTTP (median of 3 runs)

| Scenario | Framework | Req/s | p50 ms | p95 ms | p99 ms | Server CPU % | Server RSS MB |
|---|---|---:|---:|---:|---:|---:|---:|
| minimal | Kerniq | 223,061 | 0.278 | 0.467 | 0.561 | 307 | 6.9 |
| minimal | Axum | 231,981 | 0.267 | 0.449 | 0.532 | 300 | 6.8 |
| json | Kerniq | 226,590 | 0.274 | 0.459 | 0.546 | 311 | 7.1 |
| json | Axum | 225,671 | 0.274 | 0.461 | 0.548 | 301 | 7.1 |
| routes-50 | Kerniq | 221,089 | 0.281 | 0.472 | 0.563 | 310 | 7.5 |
| routes-50 | Axum | 226,789 | 0.273 | 0.460 | 0.547 | 302 | 8.5 |
| middleware | Kerniq | 215,657 | 0.288 | 0.488 | 0.579 | 320 | 7.4 |
| middleware | Axum | 206,727 | 0.301 | 0.504 | 0.605 | 321 | 7.3 |
| middleware-fused | Kerniq | 214,906 | 0.289 | 0.491 | 0.587 | 318 | 7.3 |
| middleware-fused | Axum | 208,597 | 0.300 | 0.494 | 0.585 | 323 | 7.3 |

No run had errors.

## Findings and the changes they led to

**1. A redundant body-limit layer.** Kerniq originally always applied
`DefaultBodyLimit` from `ServerConfig::max_body_size`. With the default value
that layer changes nothing, but every Axum `Router::layer` boxes the route
once more. Removing it when the limit equals Axum's built-in 2 MiB took the
minimal scenario from 17 to 13 allocations per request.

**2. Timeout error handling.** `Timeout` was first built as
`HandleErrorLayer` + `tower::timeout`. `HandleError` boxes its future, so
`Timeout` is now a single Tower service that keeps the `Sleep` inline and
renders the JSON `408` itself. The middleware scenario dropped from 42 to 34
allocations per request.

**3. Per-call layering.** Each `.middleware(..)` is one `Router::layer`,
which adds roughly two allocations per request. Fusing middleware with
`ServiceBuilder` (`into_layer()` on every built-in) removes that cost, and
Kerniq's fused stack allocates less than the equivalent tower-http stack (27
vs 30). Kerniq's `SecurityHeaders` sets all headers in one layer, where
tower-http needs one `SetResponseHeaderLayer` per header. See
[middleware](middleware.md#fusing-middleware-on-hot-paths).

**4. Slow-header protection costs about 3–4% on trivial endpoints.** The
remaining gap in `minimal` over HTTP comes from Kerniq's default
`header_read_timeout` (30 s), which arms a Hyper timer for every request on a
keep-alive connection. `axum::serve` sets no such timeout. Measured directly
(3 alternating runs each, `benches/results/header-timeout.txt`):

| Server | Req/s (mean of 3) |
|---|---:|
| Kerniq, `header_read_timeout = 30` (default) | 220,741 |
| Kerniq, `header_read_timeout = 0` | 228,686 |
| Axum (`axum::serve`) | 225,827 |

The protection stays on by default because a production server should not
leave slowloris attacks open silently. Set `KERNIQ_HEADER_READ_TIMEOUT=0` (or
`.header_read_timeout(Duration::ZERO)`) when a proxy in front of the service
already enforces header timeouts.

**5. `ConnectInfo` is not measurable.** Inserting the peer address per
request (`connect_info = true`, the default) did not move throughput beyond
run-to-run noise.

## Summary

With equivalent configuration, Kerniq adds no measurable overhead over Axum
for routing, extraction, JSON and state. Its middleware composition is free
when fused and about two allocations per extra layer otherwise. Its one
default with a visible cost (slow-header protection) is documented and can be
switched off.

## Limitations

* Client and server shared one machine, so absolute numbers understate a
  dedicated server. Compare the ratios, not the totals.
* macOS networking differs from Linux. Run `benches/run-http.sh` on your
  target platform.
* Only HTTP/1.1 keep-alive load was generated. HTTP/2 and TLS were not
  measured.
* Memory is resident set size after the run, not peak heap.
