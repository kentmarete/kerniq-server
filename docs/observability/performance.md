# Observability performance

Observability is not free. This page measures what each signal costs a
Kerniq application, so you can choose with numbers. It makes no
"zero-overhead" claims.

## Design rules the numbers depend on

* No network I/O on the request path; export runs on SDK-managed threads.
* Measurements are aggregated by the OpenTelemetry SDK; Kerniq keeps no
  copy.
* Instruments, route attribute values and propagators are created once.
  Metric attributes are built in a stack buffer.
* Bodies are never read or buffered; the response wrapper adds a length per
  frame.
* Queues are bounded; when full, items are dropped (counted), never waited
  for.

## Method

`benches/run-observability.sh` runs the same application
(`benches/src/observability.rs`: `GET /users/{id}` returning `ok` and
emitting one `INFO` event; `GET /stream` returning 64 × 64-byte frames)
under each configuration:

| Scenario | Configuration |
|---|---|
| `baseline` | no observability |
| `metrics` | HTTP metrics aggregated in-process, no exporter |
| `prometheus` | metrics + `/metrics`, scraped every second during the run |
| `otlp-metrics` | metrics pushed over OTLP/HTTP every second |
| `json-logs` | JSON console logs (one line per request) through the non-blocking writer, stdout → `/dev/null` |
| `otlp-logs` | log records exported over OTLP/HTTP, no console |
| `logs-noop-export` | the same log pipeline with an exporter that does nothing (isolates in-process cost) |
| `traces-sampled` | request spans, 10 % sampled, OTLP/HTTP |
| `traces-noop-export` | the same with a no-op exporter |
| `all` | Prometheus + OTLP metrics, runtime metrics, JSON + OTLP logs, 10 % traces |
| `collector-down` | all OTLP signals pointed at a closed port |

OTLP data goes to `bench-server sink`, a separate process that accepts and
discards it, so serialization and sending are measured but receiving is not
charged to the server. `loadgen` (closed-loop HTTP/1.1 keep-alive) runs as a
third process. CPU and RSS come from `ps`; shutdown time is SIGINT → exit
(connection drain plus the final telemetry flush). Runs are interleaved
(every scenario once, then again) and the median of three is reported, with
the 1-minute load average so a noisy machine is visible.

Allocations come from `cargo run --release -p kerniq-benches --bin
alloc-count`: a counting global allocator over 10 000 in-process requests,
with real SDK pipelines and no-op exporters. Allocations on export threads
are included.

The correctness side of the same rules is tested, not benchmarked: see
`crates/kerniq-observability-testing/tests/otlp.rs` (unreachable Collector,
hanging Collector with a saturated queue, failing Collector during shutdown)
and `crates/kerniq-observability-axum/tests/http.rs` (no buffering of
streams, client disconnects).

## Results

Environment: Apple M3 Pro (11 cores, 18 GiB), macOS 27.2, Rust 1.99.1,
release profile (`lto = "thin"`, `codegen-units = 1`), OpenTelemetry Rust
0.33.1.

### Allocations per request

In-process, current-thread runtime, real SDK pipelines with no-op exporters
(`benches/results/allocations.md`):

| Configuration | `/users/{id}` allocs | bytes | `/stream` allocs | bytes |
|---|---:|---:|---:|---:|
| baseline | 17 | 1 886 | 16 | 8 429 |
| metrics | 20 | 2 654 | 19 | 9 197 |
| traces, 100 % sampled | 49 | 6 448 | 41 | 12 105 |
| traces, 10 % sampled | 49 | 6 133 | 41 | 11 773 |
| JSON console logs (+1 line/request) | 31 | 3 561 | 28 | 9 912 |
| log export (+1 record/request) | 23 | 3 206 | 19 | 9 197 |
| metrics + traces 10 % + log export + JSON | 63 | 7 987 | 51 | 12 944 |

Readings:

* **Metrics: +3 allocations, independent of the number of frames.** They come
  from the extra `Router::layer` boxing (Axum boxes each layered route) and
  boxing the observed body. Recording into the SDK allocates nothing:
  attributes are built on the stack and the route value is cached per
  route.
* **Streaming adds no per-frame allocation** in any configuration: `/stream`
  (64 frames) costs the same or fewer allocations than `/users/{id}`.
* **Tracing costs ~32 allocations per request whether 10 % or 100 % of
  traces are sampled.** `tracing-opentelemetry` builds the span and its
  attributes before the sampling decision; sampling saves export work, not
  instrumentation. If that matters at your request rate, exclude
  the hottest routes from instrumentation with `exclude_http_route` (this
  removes their metrics too) or trace with a lower `TracesConfig::filter`
  only where needed.

### Throughput and latency

**Pending a clean run.** The first full run of
`benches/run-observability.sh` was discarded: background system processes
(media analysis, Spotlight indexing, a virtual machine) took about five of the
eleven cores midway through, and every scenario measured after that point
fell to about 50 000 req/s regardless of configuration. Kerniq does not
publish numbers from a loaded machine; reproduce on an idle machine with

```text
benches/run-observability.sh 3 8 64 4 4
```

and set acceptance budgets from that baseline.

What the same harness *did* establish, in short A/B runs taken minutes apart
under identical conditions (64 connections, 4 server workers, 4-second
runs; indicative, single runs):

| Change | Before | After |
|---|---:|---:|
| Streaming (`/stream`) with tracing: no span clone per frame, no busy/idle clock reads | 59 k streams/s | 69 k |
| … and no OpenTelemetry context activation per enter | 69 k | 77 k |
| JSON console logs: append-only span-field storage instead of re-serializing on every `record` | 134 k req/s | 206 k |

OTLP log export (one record per request) measured 158 k req/s, against
162 k for the same pipeline with a no-op exporter and 173 k for the
baseline in the same session: export itself (protobuf encoding, HTTP) is not
where the log pipeline's cost is; the per-record work in the bridge and the
batch processor is.

These optimizations are in the code: `ObservedBody` moves the span out
instead of cloning it, the OpenTelemetry layer runs with
`with_tracked_inactivity(false)`, context activation is only enabled when
log export needs it (`TracesConfig::activate_otel_context`), and
`logging::SpanFields` appends recorded span fields instead of re-parsing
them.

### Bounded behavior under failure

From the resilience tests (debug build, so the absolute times are
pessimistic):

| Situation | Recording | Shutdown | Memory |
|---|---|---|---|
| Collector unreachable, 10 000 spans | completes without waiting for the network | returns within the 2 s timeout | queue-bounded; spans dropped and counted |
| Collector accepts but never answers, 50 000 spans, queue 16 | completes; > 40 000 dropped at enqueue | within the 2 s timeout | 16 queued spans + one batch in flight |
| Collector returns 500 during shutdown | unaffected | reports the failure in `ShutdownReport` | — |

## Trade-offs

| Choice | Cost | Benefit |
|---|---|---|
| Duration measured to the end of the body | one body wrapper per response | correct streaming durations, aborted-stream detection |
| Request span when only console logs are on | the span (see allocations) | request context (`spans`) in every log line |
| Non-blocking console writer | one thread, a 16k-line buffer | a slow stdout never stalls requests; lines dropped when full |
| Batch processors | one thread each | no export work on request threads |
| Context activation off by default | OpenTelemetry-API code doesn't see the span as current | ~10 % streaming throughput |

