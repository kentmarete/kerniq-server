//! Counts heap allocations per request for each Kerniq/Axum scenario.
//!
//! ```sh
//! cargo run --release -p kerniq-benches --bin alloc-count
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use kerniq_benches::{SCENARIOS, dispatch};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: delegates every operation to the system allocator unchanged; the
// counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `System` with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const REQUESTS: u64 = 10_000;

fn measure(rt: &tokio::runtime::Runtime, router: &axum::Router, path: &'static str) -> (f64, f64) {
    rt.block_on(async {
        for _ in 0..100 {
            dispatch(router, path).await;
        }
        let (a0, b0) = (
            ALLOCATIONS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
        );
        for _ in 0..REQUESTS {
            dispatch(router, path).await;
        }
        let (a1, b1) = (
            ALLOCATIONS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
        );
        (
            (a1 - a0) as f64 / REQUESTS as f64,
            (b1 - b0) as f64 / REQUESTS as f64,
        )
    })
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    println!(
        "| Scenario | Kerniq allocs/req | Axum allocs/req | Kerniq bytes/req | Axum bytes/req |"
    );
    println!("|---|---:|---:|---:|---:|");
    for (name, kerniq, axum, path) in SCENARIOS {
        let (ka, kb) = measure(&rt, &kerniq(), path);
        let (aa, ab) = measure(&rt, &axum(), path);
        println!("| {name} | {ka:.1} | {aa:.1} | {kb:.0} | {ab:.0} |");
    }

    // Observability overhead: the same app, each configuration, both
    // routes. Allocations on export threads are included (they are
    // amortized per request).
    println!();
    println!(
        "| Observability | /users/{{id}} allocs/req | bytes/req | /stream allocs/req | bytes/req |"
    );
    println!("|---|---:|---:|---:|---:|");
    for scenario in kerniq_benches::observability::in_process::SCENARIOS {
        let (router, guard, observability) =
            rt.block_on(kerniq_benches::observability::in_process::build(scenario));
        let (ua, ub) = measure(&rt, &router, "/users/42");
        let (sa, sb) = measure(&rt, &router, "/stream");
        println!("| {scenario} | {ua:.1} | {ub:.0} | {sa:.1} | {sb:.0} |");
        drop(guard);
        if let Some(o) = observability {
            rt.block_on(o.shutdown());
        }
    }
}
