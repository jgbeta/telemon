# Docker Exporter

Use this path for Linux servers and NAS boxes where a container install is the
most practical option. It is the preferred path for OMV and container-first
Unraid installs. Native installs remain useful for desktops, `.deb` package
flows, and Unraid User Scripts fallback/baseline testing.

The container monitors the host, so it runs with host networking and read-only
host sysfs mounts.

## Build

From the repository root:

```bash
docker build -f deploy/exporter/Dockerfile -t telemon-exporter:dev .
```

## Production Compose

Use the production compose file for one exporter on a server/NAS host. It
listens on `9185`, disables fake metrics, persists config/state under
`/config`, and mounts host `/sys` read-only at `/host/sys`.

```bash
mkdir -p /srv/telemon/exporter

export TELEMON_EXPORTER_IMAGE=ghcr.io/<owner>/telemon-exporter:edge
export TELEMON_DOCKER_CONFIG_DIR=/srv/telemon/exporter
export TELEMON_REGISTRY_SERVER=<server-ip>:9186
export TELEMON_ENROLLMENT_TOKEN=<token>
export TELEMON_USER_NAME=<user label>
export TELEMON_DEVICE_NAME=<device label>
export TELEMON_ADVERTISED_ADDR=<server-lan-ip>

docker compose -f deploy/exporter/docker-compose.production.yml up -d
docker compose -f deploy/exporter/docker-compose.production.yml logs -f
```

Required values:

```yaml
TELEMON_REGISTRY_SERVER: "<server-ip>:9186"
TELEMON_ENROLLMENT_TOKEN: "<token>"
TELEMON_USER_NAME: "<user label>"
TELEMON_DEVICE_NAME: "<device label>"
```

Use `TELEMON_ADVERTISED_ADDR` when the registry cannot infer the host LAN
IP that Prometheus should scrape.
Use `TELEMON_MACHINE_UUID` when multiple OS installs should share one
physical-machine identity while keeping separate registry device UUIDs.

Adaptive sampling defaults are enabled in the generated config:

```text
TELEMON_ADAPTIVE_NORMAL_SECONDS=15
TELEMON_ADAPTIVE_WARM_SECONDS=10
TELEMON_ADAPTIVE_HOT_SECONDS=5
TELEMON_ADAPTIVE_CRITICAL_SECONDS=1
TELEMON_ADAPTIVE_TEMPERATURE_WARM_CELSIUS=60
TELEMON_ADAPTIVE_TEMPERATURE_HOT_CELSIUS=75
TELEMON_ADAPTIVE_TEMPERATURE_CRITICAL_CELSIUS=85
```

For Unraid production Docker installs, set:

```bash
export TELEMON_DOCKER_CONFIG_DIR=/boot/config/plugins/telemon-docker
```

Production Docker defaults hide unknown hwmon chips. During debugging, set
`TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN=true` or use the validation compose
file.

## Development Compose

`deploy/exporter/docker-compose.yml` is a simple development example. Prefer
`docker-compose.production.yml` for OMV, Unraid, and other server/NAS installs.

## Unraid/OMV A/B Validation

Use `deploy/exporter/docker-compose.unraid-test.yml` when you want to compare
Docker against an existing native Unraid bootstrap install. The validation
compose file:

- Runs on port `9187` so the native exporter can keep using `9185`.
- Uses a separate config/state directory.
- Disables fake metrics so fake data cannot hide a broken hwmon collector.
- Mounts host `/sys` read-only at `/host/sys`.

See `deploy/exporter/UNRAID_OMV_VALIDATION.md` for the exact Unraid and OMV
commands and interpretation steps.

## Unraid

For the validated Unraid native path, install with the root `install.sh`, then
add the printed `nohup ... run-telemon-exporter.sh ...` command to the
Unraid User Scripts plugin and configure it to run at array start. See
`docs/install-bootstrap.md`.

If you use the Docker exporter on Unraid, use host network mode and mount:

```text
/boot/config/plugins/telemon -> /config
/sys -> /host/sys:ro
```

Do not mount only `/sys/class/hwmon`. On many Linux systems, including Unraid,
the hwmon entries are symlinks into `/sys/devices`; mounting the full read-only
`/sys` tree lets the collector follow those links while still using
`/host/sys/class/hwmon` as its configured root.

Recommended environment:

```text
TELEMON_REGISTRY_SERVER=<server-ip>:9186
TELEMON_ENROLLMENT_TOKEN=<token>
TELEMON_USER_NAME=<user label>
TELEMON_DEVICE_NAME=unraid
TELEMON_ADVERTISED_ADDR=<unraid-lan-ip>
TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN=true
```

The UUID is stored at `/config/state/device-id`, so it survives container
updates.

`TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN=true` is useful on Unraid and NAS
hardware where kernel driver names may not map cleanly to known CPU, GPU, or
storage components. If native `install.sh` reports more sensors than Docker,
check the Docker mount first.

## NVIDIA

NVIDIA is optional. Non-NVIDIA hosts still run and report the collector as
unsupported. `telemon_collector_supported{collector="nvidia_nvml"} 0`
means NVML is not available to the exporter and is independent of Linux hwmon
temperature collection.

For NVIDIA hosts, configure the Unraid or Docker NVIDIA runtime so the container
can see the GPU devices and NVML library. If automatic library injection does
not work, mount the host NVML library and set:

```text
TELEMON_NVIDIA_LIBRARY_PATHS=/usr/lib/libnvidia-ml.so.1
```

## Verify

On the exporter host:

```bash
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
```

On the monitoring server:

```bash
curl http://<exporter-host-ip>:9185/metrics
curl http://<exporter-host-ip>:9185/metrics/static
curl http://<server-ip>:9186/prometheus/sd
```
