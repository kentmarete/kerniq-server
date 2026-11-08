//! Runtime and process metrics are observed at collection time only.

use kerniq_observability::prelude::*;
use kerniq_observability_testing::MetricSnapshot;
use kerniq_telemetry::semconv::runtime as names;
use opentelemetry_sdk::metrics::InMemoryMetricExporter;

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn runtime_and_process_metrics() {
    let exporter = InMemoryMetricExporter::default();
    let observability = Observability::builder()
        .metrics(
            MetricsConfig::new()
                .runtime(true)
                .exporter(exporter.clone()),
        )
        .install_subscriber(false)
        .build()
        .await
        .unwrap();

    let tasks: Vec<_> = (0..5)
        .map(|_| {
            tokio::spawn(async { tokio::time::sleep(std::time::Duration::from_secs(5)).await })
        })
        .collect();

    let provider = observability.meter_provider().unwrap().clone();
    tokio::task::spawn_blocking(move || provider.force_flush().unwrap())
        .await
        .unwrap();
    let snap = MetricSnapshot::latest(&exporter.get_finished_metrics().unwrap());

    assert_eq!(snap.total(names::TOKIO_WORKERS, &[]), 3.0);
    assert!(snap.total(names::TOKIO_ALIVE_TASKS, &[]) >= 5.0);
    assert!(snap.contains(names::TOKIO_GLOBAL_QUEUE_DEPTH));
    assert!(snap.total(names::PROCESS_CPU_TIME, &[]) > 0.0);
    assert!(snap.total(names::PROCESS_MEMORY_USAGE, &[]) > 1_000_000.0);
    assert!(snap.total(names::PROCESS_MEMORY_VIRTUAL, &[]) > 0.0);
    #[cfg(unix)]
    assert!(snap.total(names::PROCESS_OPEN_FDS, &[]) >= 3.0);

    for task in tasks {
        task.abort();
    }
}

#[tokio::test]
async fn runtime_metrics_are_off_by_default() {
    let exporter = InMemoryMetricExporter::default();
    let observability = Observability::builder()
        .metrics(MetricsConfig::new().exporter(exporter.clone()))
        .install_subscriber(false)
        .build()
        .await
        .unwrap();
    observability
        .meter_provider()
        .unwrap()
        .force_flush()
        .unwrap();
    let snap = MetricSnapshot::latest(&exporter.get_finished_metrics().unwrap());
    assert!(!snap.contains(names::TOKIO_WORKERS));
    assert!(!snap.contains(names::PROCESS_CPU_TIME));
}
