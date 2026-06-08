# macOS LaunchDaemon Install

The macOS installer runs `telemon-exporter` as a LaunchDaemon with the macOS
baseline collectors enabled by default: system uptime, memory, CPU count, and
public macOS thermal pressure state. Numeric Apple Silicon telemetry is
available through the optional disabled `macos_macmon` collector, which uses the
`macmon` Rust library directly for average CPU/GPU temperature, power, clocks,
utilization, memory, swap, and SoC metadata. Build the exporter with the
`macos-macmon` Cargo feature before enabling this collector in YAML.

Default paths:

```text
Binary:  /usr/local/libexec/telemon/telemon-exporter
Config:  /Library/Application Support/Telemon/exporter.yml
State:   /Library/Application Support/Telemon/state/device-id
Plist:   /Library/LaunchDaemons/com.telemon.exporter.plist
Logs:    /Library/Logs/Telemon/exporter.log
```

Build and install:

```bash
# Baseline exporter.
cargo build --release -p telemon-exporter

# Apple Silicon exporter with optional macmon telemetry.
cargo build --release -p telemon-exporter --features macos-macmon

sudo packaging/macos/install.sh ./target/release/telemon-exporter
```

Allow Prometheus to scrape this host:

```bash
sudo packaging/macos/install.sh --prometheus-server-ip <monitoring-server-ip> ./target/release/telemon-exporter
```

Registry enrollment accepts `--registry-server`, `--enrollment-token`,
`--user-name`, `--device-name`, optional `--advertised-addr`, and optional
`--machine-uuid` for dual-boot or multi-OS physical-machine grouping.

LAN scraping is the recommended default. Leave `--advertised-addr` blank to let
the registry use the observed source IP from the exporter's heartbeat. Use a
Tailscale or other overlay address only when you intentionally want Prometheus
to scrape through that network. Interactive installs default `device_name` to a
short local Mac name, not a Tailscale MagicDNS FQDN.
Apple Private Relay warnings are separate macOS networking/privacy notices; they
do not mean Telemon is using Tailscale.

Check:

```bash
launchctl print system/com.telemon.exporter
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
curl http://127.0.0.1:9185/json
tail -n 100 /Library/Logs/Telemon/exporter.log
tail -n 100 /Library/Logs/Telemon/exporter.err.log
```

With a `--features macos-macmon` build and `collectors.macos_macmon.enabled: true` on Apple Silicon, confirm:

```bash
curl http://127.0.0.1:9185/metrics | grep 'info_collector_up{collector="macos_macmon"} 1'
curl http://127.0.0.1:9185/metrics | grep 'hw_macmon_\\|sys_macmon_'
curl http://127.0.0.1:9185/metrics | grep 'sys_cpu_freq_mhz.*source="macmon"'
curl http://127.0.0.1:9185/json
curl http://127.0.0.1:9185/metrics/static | grep 'source="macmon"'
```

Uninstall:

```bash
sudo packaging/macos/uninstall.sh
```

The uninstall script preserves `/Library/Application Support/Telemon/exporter.yml`.
Use `--preserve-firewall` for service repair or migration, and `--remove-files`
for a full local reset.
