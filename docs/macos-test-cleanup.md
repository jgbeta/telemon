# macOS Test Cleanup

Use this guide to reset a Mac after Telemon install, LaunchDaemon, or package
experiments.

## Normal Uninstall

From the repository or mounted installer resources:

```bash
sudo packaging/macos/uninstall.sh
```

This removes:

- `/Library/LaunchDaemons/com.telemon.exporter.plist`
- the loaded `com.telemon.exporter` LaunchDaemon
- the Telemon-managed `pf` anchor and block in `/etc/pf.conf`

It preserves:

- `/usr/local/libexec/telemon/telemon-exporter`
- `/Library/Application Support/Telemon/exporter.yml`
- `/Library/Application Support/Telemon/state`
- `/Library/Logs/Telemon`

## Full Test Reset

Use this before a clean reinstall test:

```bash
sudo packaging/macos/uninstall.sh --remove-files
```

This removes the LaunchDaemon, installer-managed firewall state, installed
binary, config, state, and logs.

## Verify Cleanup

```bash
launchctl print system/com.telemon.exporter
test ! -f /Library/LaunchDaemons/com.telemon.exporter.plist
test ! -e /usr/local/libexec/telemon/telemon-exporter
curl --connect-timeout 3 http://127.0.0.1:9185/healthz
lsof -nP -iTCP:9185 -sTCP:LISTEN
sudo test ! -f /etc/pf.anchors/com.telemon.exporter
sudo grep -n 'telemon exporter' /etc/pf.conf
```

Expected after full reset:

- `launchctl print` reports the service is missing.
- The plist and binary are gone.
- `curl` cannot connect.
- No process is listening on TCP `9185`.
- The Telemon `pf` anchor and marked block are gone.

## Manual Cleanup If A Test Was Interrupted

Use these only when the uninstall script cannot run or a test was interrupted:

```bash
sudo launchctl bootout system /Library/LaunchDaemons/com.telemon.exporter.plist 2>/dev/null || true
sudo rm -f /Library/LaunchDaemons/com.telemon.exporter.plist
sudo rm -rf /usr/local/libexec/telemon
sudo rm -rf "/Library/Application Support/Telemon"
sudo rm -rf /Library/Logs/Telemon
sudo rm -f /etc/pf.anchors/com.telemon.exporter
```

If a stale foreground exporter is still running:

```bash
pgrep -af 'telemon.*exporter.*run'
```

Stop only the matching test process.

## Preserve Scrape Access During Repair

For service repair where Prometheus should keep scrape access:

```bash
sudo packaging/macos/uninstall.sh --preserve-firewall
```

Use the normal uninstall or full reset afterward to remove the firewall rule.
