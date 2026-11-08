#!/usr/bin/env python3
"""Verifies that every metric and label used by the Grafana dashboards
exists in a real Prometheus scrape of a Kerniq service.

    # 1. run the demo and generate some traffic
    cargo run -p example-observability-demo &
    observability/scripts/smoke.sh localhost:3000
    # 2. scrape and check
    curl -s localhost:3000/metrics > /tmp/scrape.txt
    python3 observability/scripts/check-dashboards.py /tmp/scrape.txt

Exits non-zero, listing the offenders, if a dashboard references a metric
name or a label that the scrape does not contain.
"""

import json
import pathlib
import re
import sys

PROMQL_FUNCS = {
    "sum", "rate", "irate", "increase", "histogram_quantile", "topk", "bottomk", "by",
    "without", "clamp_min", "clamp_max", "avg", "max", "min", "count", "label_values",
    "on", "ignoring", "group_left", "group_right", "le", "and", "or", "unless", "offset",
}


def scrape_inventory(text):
    metrics, labels = set(), set()
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        m = re.match(r"([a-zA-Z_:][a-zA-Z0-9_:]*)(\{([^}]*)\})?", line)
        if not m:
            continue
        name = m.group(1)
        metrics.add(name)
        for base_suffix in ("_bucket", "_sum", "_count"):
            if name.endswith(base_suffix):
                metrics.add(name[: -len(base_suffix)])
        if m.group(3):
            labels.update(re.findall(r'([a-zA-Z_][a-zA-Z0-9_]*)="', m.group(3)))
    return metrics, labels


def exprs(node):
    if isinstance(node, dict):
        for k, v in node.items():
            if k in ("expr", "query") and isinstance(v, str):
                yield v
            else:
                yield from exprs(v)
    elif isinstance(node, list):
        for v in node:
            yield from exprs(v)


def referenced(expr):
    expr = re.sub(r'"[^"]*"', '""', expr)  # drop string literals
    metrics, labels = set(), set()
    for sel in re.findall(r"\{([^}]*)\}", expr):
        labels.update(re.findall(r"([a-zA-Z_][a-zA-Z0-9_]*)\s*(?:=~|!~|!=|=)", sel))
    for grp in re.findall(r"\b(?:by|without)\s*\(([^)]*)\)", expr):
        labels.update(x.strip() for x in grp.split(",") if x.strip())
    for m in re.findall(r"label_values\([^,]*,\s*([a-zA-Z_]+)\)", expr):
        labels.add(m)
    bare = re.sub(r"\{[^}]*\}", "", expr)
    bare = re.sub(r"\[[^\]]*\]", "", bare)
    for token in re.findall(r"[a-zA-Z_:][a-zA-Z0-9_:]*", bare):
        if token in PROMQL_FUNCS or token in labels:
            continue
        if "_" in token:
            metrics.add(token)
    labels.discard("le")
    return metrics, labels


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    have_metrics, have_labels = scrape_inventory(pathlib.Path(sys.argv[1]).read_text())
    root = pathlib.Path(__file__).resolve().parent.parent / "grafana" / "dashboards"
    problems = []
    for path in sorted(root.glob("*.json")):
        board = json.loads(path.read_text())
        for expr in exprs(board):
            metrics, labels = referenced(expr)
            for m in sorted(metrics - have_metrics):
                problems.append(f"{path.name}: unknown metric `{m}` in: {expr}")
            for label in sorted(labels - have_labels):
                problems.append(f"{path.name}: unknown label `{label}` in: {expr}")
    if problems:
        print("\n".join(problems))
        return 1
    print(f"ok: every metric and label in {len(list(root.glob('*.json')))} dashboards exists in the scrape")
    return 0


if __name__ == "__main__":
    sys.exit(main())
