# macOS LaunchDaemon Skeleton

Phase 3 adds LaunchDaemon installation without macOS temperature collection.

Default paths:

```text
Binary:  /usr/local/libexec/telemon/telemon-exporter
Config:  /Library/Application Support/Telemon/exporter.yml
Plist:   /Library/LaunchDaemons/com.telemon.exporter.plist
Logs:    /Library/Logs/Telemon/exporter.log
```

Install:

```bash
sudo packaging/macos/install.sh ./target/release/telemon-exporter
```

Allow Prometheus to scrape this host:

```bash
sudo packaging/macos/install.sh --prometheus-server-ip <monitoring-server-ip> ./target/release/telemon-exporter
```

Registry enrollment accepts `--registry-server`, `--enrollment-token`,
`--user-name`, `--device-name`, optional `--advertised-addr`, and optional
`--machine-uuid` for dual-boot or multi-OS physical-machine grouping.

Check:

```bash
launchctl print system/com.telemon.exporter
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
tail -n 100 /Library/Logs/Telemon/exporter.log
```

Uninstall:

```bash
sudo packaging/macos/uninstall.sh
```

The uninstall script preserves `/Library/Application Support/Telemon/exporter.yml`.
