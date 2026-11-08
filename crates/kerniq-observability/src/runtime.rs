//! Tokio runtime and process metrics.
//!
//! Every instrument here is *observable*: its callback runs only when the
//! SDK collects metrics (a Prometheus scrape or an OTLP export interval).
//! Nothing polls in the background and nothing is recorded per request.
//!
//! | Metric | Source | Status |
//! |---|---|---|
//! | `kerniq.runtime.tokio.workers` | `RuntimeMetrics::num_workers` | measured |
//! | `kerniq.runtime.tokio.alive_tasks` | `RuntimeMetrics::num_alive_tasks` | measured |
//! | `kerniq.runtime.tokio.global_queue.depth` | `RuntimeMetrics::global_queue_depth` | measured |
//! | `process.cpu.time` | `getrusage`/`GetProcessTimes` (via `cpu-time`) | measured |
//! | `process.memory.usage` | RSS (via `memory-stats`) | measured |
//! | `process.memory.virtual` | virtual size (via `memory-stats`) | measured |
//! | `process.unix.file_descriptor.count` | entries of `/proc/self/fd` or `/dev/fd` | measured, Unix only |
//!
//! Blocking-pool activity, per-worker busy time, steal counts and poll
//! histograms need `--cfg tokio_unstable` and are **not** collected; Kerniq
//! uses stable Tokio APIs only. Use `tokio-metrics` or `tokio-console` when
//! you need them.

use kerniq_telemetry::semconv::runtime as names;
use opentelemetry::metrics::Meter;

/// Registers the runtime and process instruments on `meter`. Must be
/// called from inside the Tokio runtime to observe (the runtime handle is
/// captured once).
pub(crate) fn register(meter: &Meter) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let h = handle.clone();
        meter
            .u64_observable_gauge(names::TOKIO_WORKERS)
            .with_unit("{thread}")
            .with_description("Tokio worker threads.")
            .with_callback(move |o| o.observe(h.metrics().num_workers() as u64, &[]))
            .build();
        let h = handle.clone();
        meter
            .u64_observable_gauge(names::TOKIO_ALIVE_TASKS)
            .with_unit("{task}")
            .with_description("Tokio tasks that have been spawned and not yet completed.")
            .with_callback(move |o| o.observe(h.metrics().num_alive_tasks() as u64, &[]))
            .build();
        let h = handle;
        meter
            .u64_observable_gauge(names::TOKIO_GLOBAL_QUEUE_DEPTH)
            .with_unit("{task}")
            .with_description("Tasks waiting in the Tokio global (injection) queue.")
            .with_callback(move |o| o.observe(h.metrics().global_queue_depth() as u64, &[]))
            .build();
    } else {
        tracing::warn!("runtime metrics requested outside a Tokio runtime; Tokio gauges disabled");
    }

    meter
        .f64_observable_counter(names::PROCESS_CPU_TIME)
        .with_unit("s")
        .with_description("Total CPU seconds used by the process (user + system).")
        .with_callback(|o| {
            if let Ok(time) = cpu_time::ProcessTime::try_now() {
                o.observe(time.as_duration().as_secs_f64(), &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge(names::PROCESS_MEMORY_USAGE)
        .with_unit("By")
        .with_description("Resident set size of the process.")
        .with_callback(|o| {
            if let Some(stats) = memory_stats::memory_stats() {
                o.observe(stats.physical_mem as u64, &[]);
            }
        })
        .build();
    meter
        .u64_observable_gauge(names::PROCESS_MEMORY_VIRTUAL)
        .with_unit("By")
        .with_description("Virtual memory size of the process.")
        .with_callback(|o| {
            if let Some(stats) = memory_stats::memory_stats() {
                o.observe(stats.virtual_mem as u64, &[]);
            }
        })
        .build();
    #[cfg(unix)]
    meter
        .u64_observable_gauge(names::PROCESS_OPEN_FDS)
        .with_unit("{file_descriptor}")
        .with_description("Open file descriptors.")
        .with_callback(|o| {
            if let Some(count) = open_fds() {
                o.observe(count, &[]);
            }
        })
        .build();
}

#[cfg(unix)]
fn open_fds() -> Option<u64> {
    let dir = if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    };
    // The directory handle itself is one of the entries.
    std::fs::read_dir(dir)
        .ok()
        .map(|entries| (entries.count() as u64).saturating_sub(1))
}
