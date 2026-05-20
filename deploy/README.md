# Telemon Docker Deployment

This directory runs the central monitoring stack:

- The registry service accepts device enrollment and heartbeat updates.
- Prometheus discovers registered devices through HTTP service discovery.
- Grafana reads Prometheus and loads the included dashboards.

The hardware exporter can run natively on desktops and Unraid, or in Docker on
Linux servers and NAS boxes. For Unraid host sensors, the validated path is the
native bootstrap installer started through the Unraid User Scripts plugin.

## Prerequisites

- Docker Engine and Docker Compose v2 on the monitoring server.
- For production image pulls, the `deploy/` directory and `.env` image settings
  are enough.
- For local development builds, the full project repository must be present.
- The monitoring server is on the same LAN as the exporter hosts.
- The registry service is reachable from exporter hosts on TCP `9186`.
- After enrollment, each exporter host is reachable from the server on TCP `9185`.
- Each exporter is bound to a LAN-reachable address, not only `127.0.0.1`.

The production compose file pulls the registry image from GHCR by default. Set
the image owner before starting the stack:

```bash
export TELEMON_REGISTRY_IMAGE=ghcr.io/<owner>/telemon-registry:edge
```

For local development builds, copy or clone the full repository to the server:

```bash
git clone <repo-url> telemon
cd telemon
```

If you copy files manually for development, copy the project root directory. Do
not copy only `deploy/`.

Use the local-build override when you want the registry image built from source:

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.local-build.yml up -d --build
```

## Configure Registry

Edit `deploy/registry/config.yml` before starting the stack:

```yaml
registry:
  listen: "0.0.0.0:9186"
  storage_path: "/data/devices.json"
  enrollment_token: "change-me"
  device_stale_after_seconds: 120
```

Change `enrollment_token` to a LAN-local shared secret. Clients must provide
the same token during installation. The registry stores generated device UUIDs
and the last observed client IP in the `registry_data` Docker volume.

Prometheus no longer uses static device targets. `deploy/prometheus/prometheus.yml`
uses HTTP service discovery with coarse device-level adaptive scrape buckets
plus a low-frequency static scrape:

```yaml
scrape_configs:
  - job_name: "telemon-15s"
    metrics_path: /metrics
    scrape_interval: 15s
    http_sd_configs:
      - url: "http://registry:9186/prometheus/sd/15s"
        refresh_interval: 5s

  - job_name: "telemon-static"
    metrics_path: /metrics/static
    scrape_interval: 300s
    http_sd_configs:
      - url: "http://registry:9186/prometheus/sd"
        refresh_interval: 30s
```

The full Prometheus config includes `15s`, `10s`, `5s`, and `1s` dynamic jobs.
Exporters publish `telemon_requested_scrape_interval_seconds`; the registry
places each device in the matching service-discovery endpoint. Each dynamic
scrape still includes all enabled dynamic sensor metrics for that exporter.
Long-term storage reduction should be handled later with downsampling or
retention policy rather than per-sensor scrape schedules.

## Start the Stack

Commands below assume you are running from the repository root unless they explicitly show `cd deploy`.

Run from the repository root:

```bash
docker compose -f deploy/docker-compose.yml config
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps
```

Or run from this directory:

```bash
cd deploy
docker compose config
docker compose up -d
docker compose ps
```

The `cd deploy` form still requires `deploy/` to live inside the full project
directory because the registry image build context points at the project root.

Prometheus is available at:

```text
http://<server-ip>:9090
```

Grafana is available at:

```text
http://<server-ip>:3000
```

Default Grafana login:

```text
admin / change-me
```

Change this password before exposing Grafana beyond a trusted LAN.

Verify the registry from the monitoring server:

```bash
curl http://127.0.0.1:9186/healthz
curl http://127.0.0.1:9186/prometheus/sd
```

Before any clients enroll, `/prometheus/sd` should normally return an empty JSON
array. From a client host, confirm that the same registry is reachable through
the server's LAN address:

```bash
curl http://<server-ip>:9186/healthz
```

If this fails, fix the monitoring server IP, Docker port publishing, or server
firewall before installing clients.

## Enroll Clients

Install each exporter with the registry address, enrollment token, user label,
and device label. The registry returns an opaque UUID; the client stores it
locally and sends heartbeats so IP changes are reflected in Prometheus
discovery. Use the optional advertised address only when the scrape target is
different from the registry-observed source IP.

The client retries enrollment if the registry is temporarily unavailable, but
setup is easiest to validate when the registry is already running.

Linux `.deb` example:

```bash
sudo env \
  TELEMON_REGISTRY_SERVER=registry.example.local:9186 \
  TELEMON_ENROLLMENT_TOKEN=change-me \
  TELEMON_USER_NAME=example-user \
  TELEMON_DEVICE_NAME=linux-desktop \
  dpkg -i dist/linux/telemon-exporter_*.deb
```

Linux Docker exporter example for servers and container-first NAS hosts:

```bash
mkdir -p /srv/telemon/exporter

export TELEMON_DOCKER_CONFIG_DIR=/srv/telemon/exporter
export TELEMON_REGISTRY_SERVER=registry.example.local:9186
export TELEMON_ENROLLMENT_TOKEN=change-me
export TELEMON_USER_NAME=example-user
export TELEMON_DEVICE_NAME=linux-server
export TELEMON_ADVERTISED_ADDR=<server-lan-ip>

export TELEMON_EXPORTER_IMAGE=ghcr.io/<owner>/telemon-exporter:edge
docker compose -f deploy/exporter/docker-compose.production.yml up -d
```

The production Docker exporter uses host networking, listens on `9185`, disables
fake metrics, mounts host `/sys` read-only at `/host/sys`, and stores UUID/config
state in the configured `/config` directory. For side-by-side native versus
Docker validation, use the `9187` test compose file documented below.

Linux bootstrap artifact fallback for unsupported distros and Unraid native
installs:

```bash
sudo bash install.sh \
  --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name linux-desktop
```

On Unraid, add the installer's printed `nohup ... run-telemon-exporter.sh`
startup command to the User Scripts plugin and configure it to run at array
start.

Linux source-script example for local development:

```bash
sudo bash packaging/linux/install.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name linux-desktop \
  ./target/release/telemon-exporter
```

Windows example:

```powershell
.\packaging\windows\install-service.ps1 `
  -BinaryPath .\target\release\telemon-exporter.exe `
  -RegistryServer registry.example.local:9186 `
  -EnrollmentToken change-me `
  -UserName example-user `
  -DeviceName gaming-pc `
  -AdvertisedAddr exporter.example.local
```

macOS example:

```bash
sudo packaging/macos/install.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name macbook \
  ./target/release/telemon-exporter
```

If the Prometheus server IP is not supplied separately, installers derive it
from the registry address and add the source-restricted firewall rule where the
OS supports it.

For Docker exporter details, including Unraid and optional NVIDIA setup, see
`deploy/exporter/README.md`.

For side-by-side Unraid native versus Docker validation, or OMV Docker
validation, use `deploy/exporter/UNRAID_OMV_VALIDATION.md`. That flow runs the
Docker exporter on port `9187` with fake metrics disabled so it can be compared
directly against the native exporter on port `9185`.

Check the exporter from the monitoring server after enrollment:

```bash
curl http://<exporter-lan-ip>:9185/healthz
curl http://<exporter-lan-ip>:9185/metrics
curl http://<exporter-lan-ip>:9185/metrics/static
```

## Debug Prometheus

Check registry health from the monitoring server:

```bash
curl http://127.0.0.1:9186/healthz
curl http://127.0.0.1:9186/prometheus/sd
docker compose -f deploy/docker-compose.yml logs -f registry
```

After a client enrolls, `/prometheus/sd` should return a target like
`exporter.example.local:9185` with `device_uuid`, `user_name`, and `device_name` labels.
Devices disappear from service discovery after
`device_stale_after_seconds` without a heartbeat.

Adaptive service-discovery buckets can be checked directly:

```bash
curl http://127.0.0.1:9186/prometheus/sd/15s
curl http://127.0.0.1:9186/prometheus/sd/5s
curl http://127.0.0.1:9186/prometheus/sd/1s
```

Check container logs:

```bash
docker compose -f deploy/docker-compose.yml logs -f prometheus
```

Open Prometheus:

```text
http://<server-ip>:9090
```

Then check:

- `Status -> Targets`
- Query `up{job=~"telemon-(15s|10s|5s|1s)"}`
- Query `telemon_requested_scrape_interval_seconds`
- Query `telemon_collector_up`
- Query `telemon_collector_supported`

Expected healthy target:

```text
up{job="telemon-15s",device_name="linux-desktop",user_name="example-user"} 1
```

If a target is down, test the scrape URL from the monitoring server:

```bash
curl -v http://exporter.example.local:9185/metrics
```

Prometheus must be able to reach the IP and port returned by registry HTTP
service discovery. That IP comes from the registry's last observed client
heartbeat.

## Debug Grafana

Check container logs:

```bash
docker compose -f deploy/docker-compose.yml logs -f grafana
```

Grafana is provisioned automatically:

- Datasource: `Prometheus`
- Datasource URL inside Docker: `http://prometheus:9090`
- Dashboard folder: `Telemon`
- Dashboard files: `deploy/grafana/dashboards/`

After logging in, open:

- `Telemon / Telemon Overview`
- `Telemon / Telemon Temperature`
- `Telemon / Telemon NVIDIA GPU`

If dashboards load but show no data, check Prometheus targets first. Grafana can only show what Prometheus has scraped.
Dashboards include filters for `user_name`, `device_name`, `machine_uuid`,
`os`, and `device_uuid`.

## Reload After Config Changes

After changing Prometheus scrape settings or alert rules:

```bash
docker compose -f deploy/docker-compose.yml restart prometheus
```

After changing registry config:

```bash
docker compose -f deploy/docker-compose.yml restart registry
```

After changing Grafana dashboards or provisioning files:

```bash
docker compose -f deploy/docker-compose.yml restart grafana
```

For a full restart:

```bash
docker compose -f deploy/docker-compose.yml down
docker compose -f deploy/docker-compose.yml up -d
```

Named Docker volumes keep Prometheus and Grafana data across restarts:

- `registry_data`
- `prometheus_data`
- `grafana_data`

## Common Failures

Prometheus target is down:

- Exporter is not running on the monitored host.
- Exporter is bound to `127.0.0.1` instead of a LAN address.
- Host firewall blocks TCP `9185`.
- The client has not enrolled or heartbeated recently.
- The monitoring server and exporter host are not on the same reachable network.

Docker build fails with `could not find Cargo.toml in /app`:

- The server likely has only the `deploy/` directory, or Docker was run with the
  wrong build context.
- The registry image needs the full Rust project source.
- Copy or clone the full repository to the server, then run:

```bash
docker compose -f deploy/docker-compose.yml config
docker compose -f deploy/docker-compose.yml up -d
```

- Confirm the resolved registry build context points to the project root:

```bash
docker compose -f deploy/docker-compose.yml config
```

If `tcpdump` on the exporter host shows SYN packets from the Prometheus server
but no SYN-ACK response, the exporter host is usually dropping the connection
locally. Installers can add a source-restricted firewall rule:

```bash
sudo env TELEMON_PROMETHEUS_IP=<monitoring-server-ip> dpkg -i dist/linux/telemon-exporter_*.deb
sudo bash install.sh --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz --prometheus-server-ip <monitoring-server-ip>
```

Windows and macOS installers also support `-PrometheusServerIp` and
`--prometheus-server-ip` respectively. Installer-created firewall rules are recorded by the native installers and removed by their uninstall paths; manually-created test rules remain user-managed.

Grafana shows no data:

- Prometheus target is down.
- The selected dashboard time range has no scraped samples.
- Grafana datasource provisioning failed; check Grafana logs.

Prometheus or Grafana is unreachable from your browser:

- Server firewall blocks TCP `9090` or `3000`.
- Docker containers are not running.
- You are using `localhost` from another machine instead of the monitoring server IP.

Dashboard changes do not appear:

- Restart Grafana.
- Confirm the JSON file is under `deploy/grafana/dashboards/`.
- Confirm `deploy/grafana/provisioning/dashboards/dashboards.yml` still points to `/var/lib/grafana/dashboards`.

## Stop the Stack

Stop containers while keeping data volumes:

```bash
docker compose -f deploy/docker-compose.yml down
```

Remove data volumes too:

```bash
docker compose -f deploy/docker-compose.yml down -v
```
