# macOS Install

Phase 3 supports a LaunchDaemon skeleton. Exact macOS thermal collection is deferred to Phase 6.

## Build

```bash
cargo build --release
```

## Install

For UUID enrollment, start the central registry stack first and confirm the
client host can reach `http://<server-ip>:9186/healthz`. The service retries
registration if the registry is temporarily unavailable, but first install is
easiest to verify with the registry online.

```bash
sudo packaging/macos/install.sh ./target/release/telemon-exporter
```

To allow Prometheus to scrape this host during install, pass the Prometheus server IP:

```bash
sudo packaging/macos/install.sh --prometheus-server-ip <monitoring-server-ip> ./target/release/telemon-exporter
```

This adds a `pf` rule allowing TCP `9185` only from the specified Prometheus server IP.

To enroll the device with the registry during install:

```bash
sudo packaging/macos/install.sh \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name macbook \
  --advertised-addr exporter.example.local \
  ./target/release/telemon-exporter
```

If `--prometheus-server-ip` is omitted, the installer derives the scrape source
IP from `--registry-server` and adds the same source-restricted `pf` rule.
Omit `--advertised-addr` to let the registry use the connection source IP as the
Prometheus scrape target.

Installed paths:

```text
Binary:  /usr/local/libexec/telemon/telemon-exporter
Config:  /Library/Application Support/Telemon/exporter.yml
State:   /Library/Application Support/Telemon/state/device-id
Plist:   /Library/LaunchDaemons/com.telemon.exporter.plist
Logs:    /Library/Logs/Telemon/exporter.log
```

## Check

```bash
launchctl print system/com.telemon.exporter
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
curl http://<exporter-lan-ip>:9185/metrics
curl http://<exporter-lan-ip>:9185/metrics/static
tail -n 100 /Library/Logs/Telemon/exporter.log
tail -n 100 /Library/Logs/Telemon/exporter.err.log
```

For registry enrollment, check the registry from the monitoring server:

```bash
curl http://<server-ip>:9186/prometheus/sd
```

From the Prometheus server:

```bash
curl -v --connect-timeout 3 http://<exporter-lan-ip>:9185/metrics
```

## Uninstall

```bash
sudo packaging/macos/uninstall.sh
```

The uninstall script preserves `/Library/Application Support/Telemon/exporter.yml`.
It removes the `pf` anchor created by `--prometheus-server-ip`.
