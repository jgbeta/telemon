#!/usr/bin/env bash
set -euo pipefail

launchctl print system/com.telemon.exporter
curl --fail --max-time 5 http://127.0.0.1:9185/healthz
curl --fail --max-time 5 http://127.0.0.1:9185/metrics
