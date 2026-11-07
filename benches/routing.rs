//! In-process dispatch: Kerniq vs Axum for the same routes.
//!
//! Measures routing + extraction + handler + response construction with no
//! network I/O, which isolates the framework-layer overhead.
//!
//! ```sh
//! cargo bench -p kerniq-benches --bench routing
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use kerniq_benches::{SCENARIOS, dispatch};
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (name, kerniq, axum, path) in SCENARIOS.iter().filter(|s| !s.0.starts_with("middleware")) {
        let mut group = c.benchmark_group(*name);
        let (k, a) = (kerniq(), axum());
        group.bench_function("kerniq", |b| {
            b.to_async(&rt)
                .iter(|| async { black_box(dispatch(&k, path).await) })
        });
        group.bench_function("axum", |b| {
            b.to_async(&rt)
                .iter(|| async { black_box(dispatch(&a, path).await) })
        });
        group.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
