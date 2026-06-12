# Telemon

Telemon is a small Rust Prometheus-based LAN hardware telemetry project.

The exporter runs on the monitored host and exposes Prometheus text metrics. It
can run natively on desktops and Unraid, or as a host-monitoring Docker
container on Linux servers and NAS boxes. Prometheus and Grafana are provided
through Docker Compose.

## Quick Start

For local exporter-only development:

```bash
cargo run -p telemon-cli -- exporter check --config config.example.yml
cargo run -p telemon-cli -- exporter print-metrics --config config.example.yml
cargo run -p telemon-cli -- exporter run --config config.example.yml
```

In another terminal:

```bash
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/readyz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
```

For a LAN deployment with UUID enrollment, start the monitoring stack first:

```bash
docker compose -f deploy/docker-compose.yml up -d
curl http://127.0.0.1:9186/healthz
curl http://127.0.0.1:9186/prometheus/sd
```

Prometheus is available at `http://localhost:9090`. Grafana is available at `http://localhost:3000` with `admin` / `change-me`.
The stack includes the Telemon hub at `http://localhost:9186`. Its internal
registry capability enrolls clients once. Clients keep an opaque UUID locally
and send heartbeats so IP changes reach Prometheus HTTP service discovery.
Prometheus uses coarse device-level adaptive scrape buckets (`15s`, `10s`,
`5s`, `1s`) for dynamic telemetry and a separate low-frequency static scrape
for metadata-like values. Each dynamic scrape returns all enabled dynamic sensor
metrics for that exporter; storage optimization is expected to come later from
downsampling rather than per-sensor scrape scheduling.

After the registry is reachable from a client host, install the exporter with
the registry address, enrollment token, user name, and device name. See
`deploy/README.md` for the full server-first flow. For native fallback installs,
build a release artifact with `scripts/build-release.sh` and see
`docs/install-bootstrap.md`.
For Linux servers and NAS boxes such as OMV or container-first Unraid installs,
the Docker exporter is the preferred path. Native installs remain available for
desktops, `.deb` package installs, and Unraid User Scripts fallback/baseline
testing.

Production Docker installs are expected to pull public GHCR images after the
first GitHub publish. See `docs/github-ghcr.md`.

The current Prometheus metric catalog is documented in
`docs/prometheus-metrics.md` and `docs/prometheus-metrics.csv`.
For clean removal and fresh reinstall testing, see `docs/uninstall.md`.
