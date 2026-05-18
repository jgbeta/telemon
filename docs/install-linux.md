# Linux Install

## Build Package

```bash
bash packaging/linux/package-deb.sh
```

## Install

For UUID enrollment, start the central registry stack first and confirm the
client host can reach `http://<server-ip>:9186/healthz`. The service retries
registration if the registry is temporarily unavailable, but first install is
easiest to verify with the registry online.

For Debian/Ubuntu-like systems, the `.deb` package is the preferred install
path:

```bash
sudo dpkg -i dist/linux/telemon-exporter_*.deb
```

To allow Prometheus to scrape this host during package install, pass the Prometheus server IP:

```bash
sudo env TELEMON_PROMETHEUS_IP=<monitoring-server-ip> dpkg -i dist/linux/telemon-exporter_*.deb
```

For source-script installs, use:

```bash
sudo bash packaging/linux/install.sh --prometheus-server-ip <monitoring-server-ip> ./target/release/telemon-exporter
```

The source-script path is mainly for local development. For unsupported Linux
variants or hosts that should not have Cargo, use the root bootstrap installer
with a release artifact.

For Linux servers and NAS installs that already use Docker, including OMV and
container-first Unraid installs, prefer the Docker exporter path in
`deploy/exporter/README.md`. The native bootstrap installer remains the
validated Unraid User Scripts fallback/baseline and a useful path for Linux
desktops or hosts where Docker is not desired.

On UFW-based systems this adds:

```bash
sudo ufw allow from <monitoring-server-ip> to any port 9185 proto tcp comment 'telemon prometheus scrape'
sudo ufw reload
```

The installer does not enable UFW if it is inactive.

## Bootstrap Install

Use the root `install.sh` when you want a quick compatibility install, are
testing multiple machines quickly, or are on an unsupported Linux variant such
as Unraid. This script installs a prebuilt release artifact or raw binary; it
does not build the Rust project for you. See `install-bootstrap.md` for the full
bootstrap guide, including Unraid and non-systemd behavior.

```bash
scripts/build-release.sh
sudo bash install.sh --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz
```

To enroll the device with the registry during package install:

```bash
sudo env \
  TELEMON_REGISTRY_SERVER=registry.example.local:9186 \
  TELEMON_ENROLLMENT_TOKEN=change-me \
  TELEMON_USER_NAME=example-user \
  TELEMON_DEVICE_NAME=linux-desktop \
  TELEMON_ADVERTISED_ADDR=exporter.example.local \
  dpkg -i dist/linux/telemon-exporter_*.deb
```

For source-script installs:

```bash
sudo bash packaging/linux/install.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name linux-desktop \
  --advertised-addr exporter.example.local \
  ./target/release/telemon-exporter
```

If `--prometheus-server-ip` or `TELEMON_PROMETHEUS_IP` is omitted, the
installer derives the scrape source IP from the registry server address and
applies the same UFW rule where possible.

Installed paths:

```text
.deb binary:        /usr/bin/telemon-exporter
bootstrap binary:  /usr/local/bin/telemon-exporter
Config:            /etc/telemon/exporter.yml
State:             /var/lib/telemon/exporter/device-id
Service:           /lib/systemd/system/telemon-exporter.service or /etc/systemd/system/telemon-exporter.service
Logs:              journald when systemd is used
```

## Check

```bash
systemctl status telemon-exporter
journalctl -u telemon-exporter -n 100
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
curl http://<exporter-lan-ip>:9185/metrics
curl http://<exporter-lan-ip>:9185/metrics/static
```

For registry enrollment, check the service logs for registration or heartbeat
messages and check the registry from the monitoring server:

```bash
curl http://<server-ip>:9186/prometheus/sd
```

From the Prometheus server:

```bash
curl -v --connect-timeout 3 http://<exporter-lan-ip>:9185/metrics
```

If packets reach the exporter host but the connection times out, inspect local firewall behavior:

```bash
sudo tcpdump -ni any 'host <monitoring-server-ip> and tcp port 9185'
sudo ufw status verbose
sudo ufw status numbered
```

## Remove

```bash
sudo dpkg -r telemon-exporter
```

The package treats `/etc/telemon/exporter.yml` as a conffile. Removing the package preserves local config. Purge behavior is handled by `dpkg`.

If the installer created a UFW rule from `TELEMON_PROMETHEUS_IP` or
`--prometheus-server-ip`, uninstall removes that rule.
