#!/usr/bin/env bash
# Real-HTTP comparison of Kerniq and Axum.
#
#   benches/run-http.sh [runs=3] [seconds=8] [connections=64] [server-workers=4] [client-threads=4]
#
# For every scenario, Kerniq and Axum servers are started alternately (same
# binary, same runtime size), driven by `loadgen` from a separate process,
# and measured with `ps` (CPU time and resident memory). Prints a Markdown
# table of medians across runs.
set -euo pipefail

RUNS=${1:-3}; SECS=${2:-8}; CONNS=${3:-64}; WORKERS=${4:-4}; CLIENT=${5:-4}
ADDR=127.0.0.1:8090
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cargo build -q --release -p kerniq-benches --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release"

cpu_secs() { # ps cputime ([[dd-]hh:]mm:ss.cc) -> seconds
  ps -o time= -p "$1" | awk -F'[:-]' '{ s=0; for (i=1;i<=NF;i++) s=s*60+$i; print s }'
}
median() { sort -g | awk '{ a[NR]=$1 } END { print (NR%2) ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2 }'; }

declare -A PATHS=([minimal]=/ [json]=/accounts/42 [routes-50]=/accounts/42 [middleware]=/ [middleware-fused]=/)
SCENARIOS=(minimal json routes-50 middleware middleware-fused)
TMP=$(mktemp -d)

for scenario in "${SCENARIOS[@]}"; do
  for run in $(seq 1 "$RUNS"); do
    for fw in kerniq axum; do
      "$BIN/bench-server" "$scenario" "$fw" "$ADDR" "$WORKERS" >/dev/null 2>&1 &
      pid=$!
      for _ in $(seq 1 100); do nc -z 127.0.0.1 8090 2>/dev/null && break; sleep 0.05; done
      cpu0=$(cpu_secs $pid)
      result=$("$BIN/loadgen" "http://$ADDR${PATHS[$scenario]}" "$CONNS" "$SECS" "$CLIENT")
      cpu1=$(cpu_secs $pid)
      rss_kb=$(ps -o rss= -p $pid | tr -d ' ')
      kill -INT $pid; wait $pid 2>/dev/null || true
      # CPU% of one core over the measured window (+1 s warm-up).
      cpu=$(awk -v a="$cpu0" -v b="$cpu1" -v s="$SECS" 'BEGIN { printf "%.0f", (b-a)/(s+1)*100 }')
      echo "$result" | sed -E "s/^\{/{\"scenario\":\"$scenario\",\"fw\":\"$fw\",\"run\":$run,\"cpu_pct\":$cpu,\"rss_mb\":$(awk -v k="$rss_kb" 'BEGIN{printf "%.1f", k/1024}'),/" >> "$TMP/results.jsonl"
      sleep 0.5
    done
  done
done

echo
echo "| Scenario | Framework | Req/s | p50 ms | p95 ms | p99 ms | Server CPU % | Server RSS MB | Errors |"
echo "|---|---|---:|---:|---:|---:|---:|---:|---:|"
for scenario in "${SCENARIOS[@]}"; do
  for fw in kerniq axum; do
    rows=$(grep "\"scenario\":\"$scenario\",\"fw\":\"$fw\"" "$TMP/results.jsonl")
    field() { echo "$rows" | sed -E "s/.*\"$1\":([0-9.]+).*/\1/" | median; }
    printf "| %s | %s | %.0f | %.3f | %.3f | %.3f | %.0f | %.1f | %.0f |\n" \
      "$scenario" "$fw" "$(field rps)" "$(field p50_ms)" "$(field p95_ms)" "$(field p99_ms)" \
      "$(field cpu_pct)" "$(field rss_mb)" "$(field errors)"
  done
done
cp "$TMP/results.jsonl" "$ROOT/benches/results/latest.jsonl" 2>/dev/null || true
