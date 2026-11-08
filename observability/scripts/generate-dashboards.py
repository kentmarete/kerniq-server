#!/usr/bin/env python3
"""Generates the Grafana dashboards in observability/grafana/dashboards.

Every metric referenced here is emitted by Kerniq Observe or by
examples/observability-demo. `check-dashboards.py` verifies that against a
real scrape, so dashboards cannot reference metrics that do not exist.

    python3 observability/scripts/generate-dashboards.py
"""

import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "grafana" / "dashboards"
DS = {"type": "prometheus", "uid": "${datasource}"}
SEL = 'service_name=~"$service"'


def target(expr, legend="", ref="A", instant=False):
    t = {"datasource": DS, "expr": expr, "legendFormat": legend, "refId": ref}
    if instant:
        t["instant"] = True
        t["format"] = "table"
    return t


class Board:
    def __init__(self, uid, title, description):
        self.uid, self.title, self.description = uid, title, description
        self.panels = []
        self.y = 0
        self.x = 0
        self.row_h = 0
        self.next_id = 1

    def _place(self, w, h):
        if self.x + w > 24:
            self.x = 0
            self.y += self.row_h
            self.row_h = 0
        pos = {"x": self.x, "y": self.y, "w": w, "h": h}
        self.x += w
        self.row_h = max(self.row_h, h)
        return pos

    def row(self, title):
        if self.x:
            self.y += self.row_h
        self.x, self.row_h = 0, 0
        self.panels.append({
            "type": "row", "title": title, "id": self.next_id, "collapsed": False,
            "gridPos": {"x": 0, "y": self.y, "w": 24, "h": 1}, "panels": [],
        })
        self.next_id += 1
        self.y += 1

    def panel(self, kind, title, targets, unit="short", w=8, h=8, description="", **extra):
        p = {
            "type": kind,
            "title": title,
            "id": self.next_id,
            "datasource": DS,
            "description": description,
            "gridPos": self._place(w, h),
            "targets": [dict(t, refId=chr(65 + i)) for i, t in enumerate(targets)],
            "fieldConfig": {"defaults": {"unit": unit}, "overrides": []},
            "options": {},
        }
        if kind == "timeseries":
            p["fieldConfig"]["defaults"]["custom"] = {"lineWidth": 1, "fillOpacity": 10}
            p["options"] = {"legend": {"displayMode": "list", "placement": "bottom"},
                            "tooltip": {"mode": "multi"}}
        if kind == "stat":
            p["options"] = {"reduceOptions": {"calcs": ["lastNotNull"]}, "colorMode": "value",
                            "graphMode": "area"}
        p.update(extra)
        self.panels.append(p)
        self.next_id += 1

    def ts(self, title, targets, unit="short", **kw):
        self.panel("timeseries", title, targets, unit, **kw)

    def stat(self, title, expr, unit="short", w=6, h=4, **kw):
        self.panel("stat", title, [target(expr)], unit, w=w, h=h, **kw)

    def write(self, filename, variables=None):
        templating = [
            {"name": "datasource", "type": "datasource", "query": "prometheus",
             "current": {"text": "Prometheus", "value": "prometheus"}, "hide": 0},
            {"name": "service", "type": "query", "datasource": DS,
             "query": "label_values(http_server_request_duration_seconds_count, service_name)",
             "refresh": 2, "includeAll": True, "multi": True, "allValue": ".*",
             "current": {"text": "All", "value": "$__all"}},
        ] + (variables or [])
        board = {
            "uid": self.uid, "title": self.title, "description": self.description,
            "tags": ["kerniq"], "timezone": "browser", "schemaVersion": 39, "version": 1,
            "editable": True, "refresh": "10s", "time": {"from": "now-30m", "to": "now"},
            "templating": {"list": templating}, "panels": self.panels,
            "links": [{"type": "dashboards", "tags": ["kerniq"], "asDropdown": True,
                       "title": "Kerniq dashboards"}],
        }
        (OUT / filename).write_text(json.dumps(board, indent=2) + "\n")


def quantile(q, metric, by="le", extra_sel="", window="$__rate_interval"):
    sel = SEL + (("," + extra_sel) if extra_sel else "")
    return f"histogram_quantile({q}, sum by ({by}) (rate({metric}_bucket{{{sel}}}[{window}])))"


DUR = "http_server_request_duration_seconds"
TTFB = "kerniq_http_server_time_to_first_byte_seconds"


def overview():
    b = Board("kerniq-overview", "Kerniq / Overview",
              "Golden signals for Kerniq services: traffic, errors, latency, saturation.")
    b.row("Golden signals")
    b.stat("Requests / s", f"sum(rate({DUR}_count{{{SEL}}}[$__rate_interval]))", "reqps")
    b.stat("Server error ratio",
           f'sum(rate({DUR}_count{{{SEL},error_type!=""}}[$__rate_interval])) / '
           f"clamp_min(sum(rate({DUR}_count{{{SEL}}}[$__rate_interval])), 1e-9)",
           "percentunit", description="5xx responses, aborted requests and body errors.")
    b.stat("p95 latency", quantile(0.95, DUR), "s")
    b.stat("Active requests", f"sum(http_server_active_requests{{{SEL}}})", "short",
           description="Includes the full lifetime of streaming responses.")
    b.ts("Throughput by status", [target(
        f"sum by (http_response_status_code) (rate({DUR}_count{{{SEL}}}[$__rate_interval]))",
        "{{http_response_status_code}}")], "reqps", w=12)
    b.ts("Latency", [
        target(quantile(0.5, DUR), "p50"),
        target(quantile(0.95, DUR), "p95"),
        target(quantile(0.99, DUR), "p99"),
    ], "s", w=12)
    b.row("Process")
    b.ts("CPU (cores)", [target(
        f"rate(process_cpu_time_seconds_total{{{SEL}}}[$__rate_interval])", "{{service_name}}")],
        "short", w=8)
    b.ts("Resident memory", [target(f"process_memory_usage_bytes{{{SEL}}}", "{{service_name}}")],
         "bytes", w=8)
    b.ts("Tokio alive tasks", [target(f"kerniq_runtime_tokio_alive_tasks{{{SEL}}}", "{{service_name}}")],
         "short", w=8)
    b.row("Top routes")
    b.panel("table", "Routes by traffic", [target(
        f"topk(15, sum by (http_route, http_request_method) "
        f"(rate({DUR}_count{{{SEL},http_route!=\"\"}}[5m])))", instant=True)],
        "reqps", w=12, h=9)
    b.panel("table", "Slowest routes (p95)", [target(
        f"topk(15, histogram_quantile(0.95, sum by (le, http_route) "
        f"(rate({DUR}_bucket{{{SEL},http_route!=\"\"}}[5m]))))", instant=True)],
        "s", w=12, h=9)
    b.row("Telemetry pipeline health")
    b.ts("Telemetry items by outcome", [target(
        f"sum by (kerniq_signal, kerniq_outcome) (rate(kerniq_telemetry_items_total{{{SEL}}}[$__rate_interval]))",
        "{{kerniq_signal}} {{kerniq_outcome}}")], "short", w=12,
        description="offered − exported − failed = queued + dropped (OTLP pipelines).")
    b.ts("Export failures", [target(
        f'sum by (kerniq_signal) (rate(kerniq_telemetry_exports_total{{{SEL},kerniq_outcome="failure"}}[$__rate_interval]))',
        "{{kerniq_signal}}")], "short", w=12)
    b.write("kerniq-overview.json")


def http():
    b = Board("kerniq-http", "Kerniq / HTTP",
              "HTTP server metrics by route template (OpenTelemetry semantic conventions).")
    route = 'http_route=~"$route"'
    b.row("Traffic and errors")
    b.ts("Requests by route", [target(
        f"sum by (http_route) (rate({DUR}_count{{{SEL},{route}}}[$__rate_interval]))", "{{http_route}}")],
        "reqps", w=12)
    b.ts("Errors by error.type", [target(
        f'sum by (http_route, error_type) (rate({DUR}_count{{{SEL},{route},error_type!=""}}[$__rate_interval]))',
        "{{http_route}} {{error_type}}")], "reqps", w=12,
        description="5xx status codes, kerniq.aborted (client went away), kerniq.body_error.")
    b.ts("4xx by route", [target(
        f'sum by (http_route, http_response_status_code) (rate({DUR}_count{{{SEL},{route},http_response_status_code=~"4.."}}[$__rate_interval]))',
        "{{http_route}} {{http_response_status_code}}")], "reqps", w=12)
    b.ts("Active requests", [target(
        f"sum by (http_request_method) (http_server_active_requests{{{SEL}}})", "{{http_request_method}}")],
        "short", w=12)
    b.row("Latency")
    b.ts("p95 duration by route", [target(
        f"histogram_quantile(0.95, sum by (le, http_route) (rate({DUR}_bucket{{{SEL},{route}}}[$__rate_interval])))",
        "{{http_route}}")], "s", w=12)
    b.ts("p95 time to first byte by route", [target(
        f"histogram_quantile(0.95, sum by (le, http_route) (rate({TTFB}_bucket{{{SEL},{route}}}[$__rate_interval])))",
        "{{http_route}}")], "s", w=12,
        description="For streaming responses this is the time to the first chunk.")
    b.panel("heatmap", "Duration distribution", [target(
        f"sum by (le) (rate({DUR}_bucket{{{SEL},{route}}}[$__rate_interval]))", "{{le}}")],
        "s", w=24, h=9, options={"calculate": False, "yAxis": {"unit": "s"}})
    b.row("Payload sizes")
    b.ts("p95 response size by route", [target(
        f"histogram_quantile(0.95, sum by (le, http_route) (rate(http_server_response_body_size_bytes_bucket{{{SEL},{route}}}[$__rate_interval])))",
        "{{http_route}}")], "bytes", w=12)
    b.ts("Response bytes / s", [target(
        f"sum by (http_route) (rate(http_server_response_body_size_bytes_sum{{{SEL},{route}}}[$__rate_interval]))",
        "{{http_route}}")], "Bps", w=12)
    b.write("kerniq-http.json", [
        {"name": "route", "type": "query", "datasource": DS,
         "query": f'label_values({DUR}_count{{{SEL}}}, http_route)',
         "refresh": 2, "includeAll": True, "multi": True, "allValue": ".*",
         "current": {"text": "All", "value": "$__all"}},
    ])


def runtime():
    b = Board("kerniq-runtime", "Kerniq / Runtime",
              "Tokio runtime and process metrics (MetricsConfig::runtime). Measured with stable "
              "Tokio APIs only; blocking-pool and per-worker metrics are not collected.")
    b.row("Tokio")
    b.ts("Worker threads", [target(f"kerniq_runtime_tokio_workers{{{SEL}}}", "{{service_name}}")], w=8)
    b.ts("Alive tasks", [target(f"kerniq_runtime_tokio_alive_tasks{{{SEL}}}", "{{service_name}}")], w=8)
    b.ts("Global queue depth", [target(f"kerniq_runtime_tokio_global_queue_depth{{{SEL}}}", "{{service_name}}")],
         w=8, description="Sustained growth means workers cannot keep up with spawned work.")
    b.row("Process")
    b.ts("CPU (cores)", [target(f"rate(process_cpu_time_seconds_total{{{SEL}}}[$__rate_interval])",
                                "{{service_name}}")], w=6)
    b.ts("Resident memory", [target(f"process_memory_usage_bytes{{{SEL}}}", "{{service_name}}")],
         "bytes", w=6)
    b.ts("Virtual memory", [target(f"process_memory_virtual_bytes{{{SEL}}}", "{{service_name}}")],
         "bytes", w=6)
    b.ts("Open file descriptors", [target(f"process_unix_file_descriptor_count{{{SEL}}}", "{{service_name}}")],
         w=6)
    b.row("Application queues")
    b.ts("Queue depth", [target(f"queue_depth{{{SEL}}}", "{{queue_name}}")], w=8)
    b.ts("Jobs processed / s", [target(
        f"sum by (queue_name, outcome) (rate(queue_jobs_processed_total{{{SEL}}}[$__rate_interval]))",
        "{{queue_name}} {{outcome}}")], "short", w=8)
    b.ts("p95 queue wait", [target(quantile(0.95, "queue_job_wait_duration_seconds", "le, queue_name"),
                                   "{{queue_name}}")], "s", w=8)
    b.write("kerniq-runtime.json")


def inference():
    b = Board("kerniq-inference", "Kerniq / AI inference",
              "Long-running inference workloads (kerniq_telemetry::inference conventions).")
    model = 'inference_model=~"$model"'
    m = f"{SEL},{model}"
    b.row("Requests")
    b.stat("Requests / s", f"sum(rate(inference_requests_total{{{m}}}[$__rate_interval]))", "reqps")
    b.stat("Active streams", f"sum(inference_active_streams{{{m}}})")
    b.stat("p95 time to first token", quantile(0.95, "inference_time_to_first_token_seconds", extra_sel=model), "s")
    b.stat("Output tokens / s", f"sum(rate(inference_tokens_generated_total{{{m}}}[$__rate_interval]))")
    b.ts("Requests by outcome", [target(
        f"sum by (inference_model, inference_outcome) (rate(inference_requests_total{{{m}}}[$__rate_interval]))",
        "{{inference_model}} {{inference_outcome}}")], "reqps", w=12,
        description="cancelled = the client disconnected before generation finished.")
    b.ts("Errors by type", [target(
        f"sum by (inference_model, error_type) (rate(inference_errors_total{{{m}}}[$__rate_interval]))",
        "{{inference_model}} {{error_type}}")], "reqps", w=12)
    b.row("Latency")
    b.ts("Time to first token", [
        target(quantile(0.5, "inference_time_to_first_token_seconds", "le, inference_model", model), "p50 {{inference_model}}"),
        target(quantile(0.95, "inference_time_to_first_token_seconds", "le, inference_model", model), "p95 {{inference_model}}"),
    ], "s", w=8)
    b.ts("Queue wait", [
        target(quantile(0.95, "inference_queue_duration_seconds", "le, inference_model", model), "p95 {{inference_model}}"),
    ], "s", w=8)
    b.ts("Total duration", [
        target(quantile(0.95, "inference_duration_seconds", "le, inference_model", model), "p95 {{inference_model}}"),
    ], "s", w=8)
    b.row("Tokens")
    b.ts("Tokens / s", [
        target(f"sum by (inference_model) (rate(inference_tokens_generated_total{{{m}}}[$__rate_interval]))",
               "output {{inference_model}}"),
        target(f"sum by (inference_model) (rate(inference_tokens_input_total{{{m}}}[$__rate_interval]))",
               "input {{inference_model}}"),
    ], "short", w=12)
    b.ts("Per-request throughput (p50)", [target(
        quantile(0.5, "inference_token_throughput_per_second", "le, inference_model", model),
        "{{inference_model}}")], "short", w=12, description="Output tokens per second of generation, per request.")
    b.row("HTTP view of the stream endpoint")
    b.ts("Stream lifetime vs time to first byte (p95)", [
        target(f'histogram_quantile(0.95, sum by (le) (rate({DUR}_bucket{{{SEL},http_route="/v1/generate"}}[$__rate_interval])))',
               "request duration (stream lifetime)"),
        target(f'histogram_quantile(0.95, sum by (le) (rate({TTFB}_bucket{{{SEL},http_route="/v1/generate"}}[$__rate_interval])))',
               "time to first byte"),
    ], "s", w=12)
    b.ts("Abandoned streams", [target(
        f'sum(rate({DUR}_count{{{SEL},http_route="/v1/generate",error_type="kerniq.aborted"}}[$__rate_interval]))',
        "client disconnected")], "reqps", w=12)
    b.write("kerniq-inference.json", [
        {"name": "model", "type": "query", "datasource": DS,
         "query": f"label_values(inference_requests_total{{{SEL}}}, inference_model)",
         "refresh": 2, "includeAll": True, "multi": True, "allValue": ".*",
         "current": {"text": "All", "value": "$__all"}},
    ])


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    overview()
    http()
    runtime()
    inference()
    print(f"wrote dashboards to {OUT}")
