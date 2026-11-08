#!/usr/bin/env bash
# Observability overhead: the same Kerniq app under each observability
# configuration, over real HTTP.
#
#   benches/run-observability.sh [runs=3] [seconds=8] [connections=64] [server-workers=4] [client-threads=4]
#
# For each scenario and workload (`/users/42`: small response;
# `/stream`: 64 × 64-byte frames), `bench-server obs <scenario>` is started,
# driven by `loadgen` from a separate process and measured with `ps`. OTLP
# scenarios export to `bench-server sink` in another process. The
# `prometheus` and `all` scenarios are scraped once per second during the
# run. Shutdown time is SIGINT → process exit (drain + telemetry flush).
# Prints a Markdown table of medians.
set -euo pipefail

RUNS=${1:-3}; SECS=${2:-8}; CONNS=${3:-64}; WORKERS=${4:-4}; CLIENT=${5:-4}
ADDR=127.0.0.1:8091
SINK=127.0.0.1:4399
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cargo build -q --release -p kerniq-benches --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release"
TMP=$(mktemp -d)
SCENARIOS=(baseline metrics prometheus otlp-metrics json-logs otlp-logs logs-noop-export traces-sampled traces-noop-export all collector-down)
WORKLOADS=(users stream)

now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time*1000'; }
cpu_secs() { ps -o time= -p "$1" | awk -F'[:-]' '{ s=0; for (i=1;i<=NF;i++) s=s*60+$i; print s }'; }
median() { sort -g | awk '{ a[NR]=$1 } END { print (NR%2) ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2 }'; }
wait_port() { for _ in $(seq 1 200); do nc -z "${1%:*}" "${1#*:}" 2>/dev/null && return 0; sleep 0.05; done; return 1; }

"$BIN/bench-server" sink "$SINK" >/dev/null 2>&1 &
SINK_PID=$!
trap 'kill -INT $SINK_PID 2>/dev/null || true' EXIT
wait_port "$SINK"

# Runs are the outer loop, so slow drift in machine load (thermal limits,
# background work) spreads over every scenario instead of biasing the last.
echo "load average at start: $(sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg)" >&2
for run in $(seq 1 "$RUNS"); do
  for workload in "${WORKLOADS[@]}"; do
    path=/users/42; [ "$workload" = stream ] && path=/stream
    for scenario in "${SCENARIOS[@]}"; do
      KERNIQ_BENCH_SINK="http://$SINK" "$BIN/bench-server" obs "$scenario" "$ADDR" "$WORKERS" \
        >/dev/null 2>"$TMP/stderr" &
      pid=$!
      wait_port "$ADDR"
      scraper=""
      if [ "$scenario" = prometheus ] || [ "$scenario" = all ]; then
        ( while :; do curl -s -o /dev/null "http://$ADDR/metrics"; sleep 1; done ) &
        scraper=$!
      fi
      cpu0=$(cpu_secs $pid)
      result=$("$BIN/loadgen" "http://$ADDR$path" "$CONNS" "$SECS" "$CLIENT")
      cpu1=$(cpu_secs $pid)
      rss_kb=$(ps -o rss= -p $pid | tr -d ' ')
      [ -n "$scraper" ] && kill "$scraper" 2>/dev/null && wait "$scraper" 2>/dev/null || true
      t0=$(now_ms); kill -INT $pid; wait $pid 2>/dev/null || true; t1=$(now_ms)
      stats=$(grep -o '"stats":{[^}]*}' "$TMP/stderr" || echo '"stats":{}')
      cpu=$(awk -v a="$cpu0" -v b="$cpu1" -v s="$SECS" 'BEGIN { printf "%.0f", (b-a)/(s+1)*100 }')
      load=$(sysctl -n vm.loadavg 2>/dev/null | awk '{print $2}' || awk '{print $1}' /proc/loadavg)
      echo "$result" | sed -E "s/^\{/{\"workload\":\"$workload\",\"scenario\":\"$scenario\",\"run\":$run,\"load1\":$load,\"cpu_pct\":$cpu,\"rss_mb\":$(awk -v k="$rss_kb" 'BEGIN{printf "%.1f", k/1024}'),\"shutdown_ms\":$((t1-t0)),$stats,/" >> "$TMP/results.jsonl"
      sleep 0.5
    done
  done
done

for workload in "${WORKLOADS[@]}"; do
  echo
  echo "### Workload: $workload"
  echo
  echo "| Scenario | Req/s | vs baseline | p50 ms | p95 ms | p99 ms | CPU % | RSS MB | Shutdown ms | Errors | Load avg |"
  echo "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
  base=""
  for scenario in "${SCENARIOS[@]}"; do
    rows=$(grep "\"workload\":\"$workload\",\"scenario\":\"$scenario\"," "$TMP/results.jsonl")
    field() { echo "$rows" | sed -E "s/.*\"$1\":([0-9.]+).*/\1/" | median; }
    rps=$(field rps); [ -z "$base" ] && base=$rps
    printf "| %s | %.0f | %+.1f%% | %.3f | %.3f | %.3f | %.0f | %.1f | %.0f | %.0f | %.1f |\n" \
      "$scenario" "$rps" "$(awk -v r="$rps" -v b="$base" 'BEGIN{print (r/b-1)*100}')" \
      "$(field p50_ms)" "$(field p95_ms)" "$(field p99_ms)" "$(field cpu_pct)" \
      "$(field rss_mb)" "$(field shutdown_ms)" "$(field errors)" "$(field load1)"
  done
done
cp "$TMP/results.jsonl" "$ROOT/benches/results/observability.jsonl"
