#!/usr/bin/env bash
set -u

OUT_DIR="debug-bundles/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT_DIR"

run_capture() {
  local name="$1"
  shift
  {
    echo "$ $*"
    "$@"
  } > "$OUT_DIR/$name" 2>&1 || true
}

run_capture date.txt date -u
run_capture uname.txt uname -a
run_capture id.txt id
run_capture cargo-check.txt cargo run -- check --config config.example.yml
run_capture discover-dev.txt cargo run -- discover --config config.example.yml
run_capture print-metrics-dev.txt cargo run -- print-metrics --config config.example.yml
run_capture healthz.txt curl -v --max-time 5 http://127.0.0.1:9185/healthz
run_capture readyz.txt curl -v --max-time 5 http://127.0.0.1:9185/readyz
run_capture metrics.txt curl -v --max-time 5 http://127.0.0.1:9185/metrics

if command -v telemon-exporter >/dev/null 2>&1; then
  run_capture exporter-check-installed.txt telemon-exporter check --config /etc/telemon/exporter.yml
  run_capture discover-installed.txt telemon-exporter discover --config /etc/telemon/exporter.yml
fi

if command -v systemctl >/dev/null 2>&1; then
  run_capture systemctl-status.txt systemctl status telemon-exporter
fi

if command -v journalctl >/dev/null 2>&1; then
  run_capture journalctl.txt journalctl -u telemon-exporter -n 200 --no-pager
fi

if [ -d /sys/class/hwmon ]; then
  run_capture hwmon-files.txt find /sys/class/hwmon -maxdepth 2 -type f \( -name "name" -o -name "temp*_input" -o -name "temp*_label" -o -name "temp*_crit" -o -name "temp*_max" \) -print
fi

if command -v docker >/dev/null 2>&1; then
  run_capture docker-compose-ps.txt docker compose -f deploy/docker-compose.yml ps
fi

echo "Debug bundle written to $OUT_DIR"
