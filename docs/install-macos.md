# macOS Install

The macOS installer runs `telemon-exporter` as a LaunchDaemon and enables the
supported macOS baseline: registration, Prometheus scrape visibility, system
uptime/memory/CPU count, and public macOS thermal pressure state.

The stable macOS baseline signal for thermal health is public thermal pressure
state. Numeric Apple Silicon telemetry is optional through
`collectors.macos_macmon`, which uses the `macmon` Rust library directly. It is
disabled in the packaged default config until you intentionally enable it for
Apple Silicon validation. The binary must also be built with the
`macos-macmon` Cargo feature; otherwise the collector reports unsupported even
if the YAML flag is set.

## Build

```bash
# Baseline macOS exporter without numeric macmon telemetry.
cargo build --release -p telemon-exporter

# Apple Silicon build with the optional macmon backend.
cargo build --release -p telemon-exporter --features macos-macmon
```

## Install

For UUID enrollment, start the central registry stack first and confirm the
client host can reach `http://<server-ip>:9186/healthz`. The service retries
registration if the registry is temporarily unavailable, but first install is
easiest to verify with the registry online.

The recommended scrape path is normal LAN IPv4 connectivity from Prometheus to
the Mac. Leave `--advertised-addr` blank unless you intentionally need a
specific hostname or IP. A blank advertised address lets the registry use the
observed source IP from the Mac's heartbeat, which is usually the correct LAN
target. Tailscale can be used deliberately later, but it should not be the
default scrape path.

```bash
sudo packaging/macos/install.sh ./target/release/telemon-exporter
```

When creating a new config, the installer copies
`packaging/macos/config.default.yml`. Re-running the installer updates the
binary and service file but preserves existing config edits.

To test Apple Silicon macmon telemetry, first install a binary built with
`--features macos-macmon`. After install, edit
`/Library/Application Support/Telemon/exporter.yml` and set:

```yaml
collectors:
  macos_macmon:
    enabled: true
```

Then restart the service with `sudo launchctl kickstart -k system/com.telemon.exporter`.

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
  --machine-uuid <shared-machine-uuid-if-dual-boot> \
  ./target/release/telemon-exporter
```

If `--prometheus-server-ip` is omitted, the installer derives the scrape source
IP from `--registry-server` and adds the same source-restricted `pf` rule.
Omit `--advertised-addr` to let the registry use the connection source IP as the
Prometheus scrape target. Use `--machine-uuid` when dual-boot or multi-OS
installs should share one physical-machine identity.

In interactive installs, the device-name prompt defaults to a short local Mac
name from `scutil --get LocalHostName`, `hostname -s`, or the first hostname
label. It should not default to a full Tailscale MagicDNS name. If macOS shows
Apple Private Relay warnings during networking work, treat those as separate
network/privacy notices; they are not evidence that Telemon is using Tailscale.

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
curl http://127.0.0.1:9185/json
curl http://<exporter-lan-ip>:9185/metrics
curl http://<exporter-lan-ip>:9185/metrics/static
tail -n 100 /Library/Logs/Telemon/exporter.log
tail -n 100 /Library/Logs/Telemon/exporter.err.log
```

For registry enrollment, check the registry from the monitoring server:

```bash
curl http://<server-ip>:9186/prometheus/sd
```

Service discovery includes both the compatibility label `host`, which mirrors
`device_name`, and `target_host`, which is the actual host Prometheus will
scrape. With blank `advertised_addr`, `target_host` should be the observed LAN
IP.

When `macos_macmon` is enabled on Apple Silicon in both the binary feature set
and the config file, confirm
`exporter_collector_up{collector="macos_macmon"} 1`, look for `macmon_*`
temperature, power, clock, utilization, and memory metrics, and verify `/json`
returns valid JSON. If the collector reports `0`, check
`/Library/Logs/Telemon/exporter.err.log`.

From the Prometheus server:

```bash
curl -v --connect-timeout 3 http://<exporter-lan-ip>:9185/metrics
```

## Uninstall

```bash
sudo packaging/macos/uninstall.sh
```

The uninstall script preserves `/Library/Application Support/Telemon/exporter.yml`.
It removes the `pf` anchor created by `--prometheus-server-ip`. Use
`--preserve-firewall` for service repair or migration where Prometheus should
keep scrape access, and `--remove-files` for a full local reset.
For full reset commands, see `uninstall.md`.
For repeated test cleanup, see `macos-test-cleanup.md`.
