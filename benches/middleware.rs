//! In-process dispatch through an equivalent middleware stack
//! (request IDs, security headers, timeout): Kerniq vs Axum + tower-http.
//!
//! ```sh
//! cargo bench -p kerniq-benches --bench middleware
//! ```

use criterion::{Criterion, criterion_group, criterion_main};
use kerniq_benches::{axum_middleware, dispatch, kerniq_middleware, kerniq_middleware_fused};
use std::hint::black_box;

fn bench(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (k, f, a) = (
        kerniq_middleware(),
        kerniq_middleware_fused(),
        axum_middleware(),
    );
    let mut group = c.benchmark_group("middleware");
    group.bench_function("kerniq", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(dispatch(&k, "/").await) })
    });
    group.bench_function("kerniq-fused", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(dispatch(&f, "/").await) })
    });
    group.bench_function("axum", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(dispatch(&a, "/").await) })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
