# Vendor integrations

Kerniq applications export OpenTelemetry (OTLP) or expose Prometheus metrics.
Every vendor below is reached through one of those; no vendor SDK is linked
into the application, and switching vendors is a Collector configuration
change.

Recommended topology: application → OTLP → **OpenTelemetry Collector**
(sidecar, DaemonSet or gateway) → vendor. The Collector holds credentials,
retries, fans out, samples and transforms; the application stays simple.

| Backend | Metrics | Traces | Logs | How |
|---|---|---|---|---|
| Prometheus | ✓ | | | scrape `/metrics` (Option A) or Collector `prometheus` / `prometheusremotewrite` exporter |
| Grafana Mimir | ✓ | | | Collector `prometheusremotewrite` or OTLP (`/otlp`) |
| Grafana Tempo | | ✓ | | OTLP, directly or through the Collector |
| Grafana Loki | | | ✓ | OTLP (`http://loki:3100/otlp`, Loki 3+) |
| Grafana | — | — | — | visualization only; queries the stores above |
| Jaeger | | ✓ | | OTLP (Jaeger accepts OTLP natively on 4317/4318) |
| Datadog | ✓ | ✓ | ✓ | Collector `datadog` exporter + `datadog/connector`, or the Datadog Agent's OTLP intake |
| Elastic / ELK | ✓ | ✓ | ✓ | OTLP to Elastic's managed OTLP endpoint or APM Server; or Collector `elasticsearch` exporter |
| Splunk | ✓ | ✓ | ✓ | Collector `splunk_hec` exporter (Splunk Enterprise/Cloud); Splunk Observability Cloud accepts OTLP |
| Sumo Logic | ✓ | ✓ | ✓ | Collector `sumologic` exporter, or OTLP/HTTP to a Sumo OTLP source |
| Azure Monitor / Application Insights | ✓ | ✓ | ✓ | Collector `azuremonitor` exporter (connection string) |

Ready-made Collector configurations:

* `observability/otel-collector/config.yaml` — Prometheus, Tempo, Loki,
* `observability/otel-collector/config-multi-exporter.yaml` — the same plus
  Elastic, Splunk, Sumo Logic and Azure Monitor at once,
* `observability/otel-collector/config-datadog.yaml` — Datadog with APM
  trace metrics.

All three pass `otelcol-contrib validate` (0.130.0).

## Notes per vendor

**Datadog.** Use the `datadog/connector` in the traces pipeline so APM
service metrics (hits, errors, latency) are computed before sampling.
Histograms are sent as Datadog distributions. The Datadog Agent (7.x) can
also receive OTLP directly (`otlp_config` in `datadog.yaml`); point the
application's OTLP endpoint at the Agent.

**Elastic.** Elastic Cloud provides a managed OTLP endpoint (Authorization:
`ApiKey …`); self-managed deployments use APM Server, which accepts OTLP.
Logs land in `logs-*` data streams with `trace.id` for correlation.

**Splunk.** Splunk HEC needs a token and an index; Splunk Observability Cloud
(APM / Infrastructure) ingests OTLP with an access token header.

**Sumo Logic.** Create a hosted collector with an OTLP/HTTP source and use
its URL; or use the `sumologic` exporter.

**Azure Monitor.** The `azuremonitor` exporter maps spans to requests and
dependencies, logs to traces; set the Application Insights connection
string.

**Prometheus-only shops.** Option A needs nothing but a scrape config.
Traces and logs can be added later through OTLP without touching metrics.

## Why not vendor SDKs

* Vendor SDKs duplicate the OpenTelemetry pipeline (and its cost) inside the
  process and tie code to one backend.
* Credentials and endpoints would live in every service instead of one
  Collector.
* Every vendor above accepts OpenTelemetry, directly or through its
  Collector exporter.

Kerniq will add a vendor-specific adapter only if a vendor requirement
cannot be met through OTLP and the Collector; none has been found.
