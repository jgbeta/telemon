# Uninstall And Reset

Use this guide when you want to remove Telemon cleanly before reinstalling or
when you want to delete all local state and start fresh.

There are two different levels of removal:

- **Uninstall** stops/removes the service or containers while preserving local
  config and UUID state.
- **Full reset** removes config and state too. The next install will enroll
  again and may receive a new `device_uuid`.

There is also a repair/migration case: remove a stale runtime without closing
Prometheus scrape access. Use the preserve-firewall options for that path.

## Linux `.deb`

For Debian/Ubuntu-like systems installed with `dist/linux/telemon-exporter_*.deb`:

```bash
sudo systemctl stop telemon-exporter.service
sudo apt remove telemon-exporter
```

`apt remove` removes the package and service but preserves the package
configuration file at `/etc/telemon/exporter.yml`.

To remove the package and its Debian conffiles:

```bash
sudo apt purge telemon-exporter
```

If you are using `dpkg` directly:

```bash
sudo dpkg --purge telemon-exporter
```

The package lifecycle scripts remove the Telemon-managed UFW rule recorded in
`/etc/telemon/prometheus-firewall-rule` or `/etc/telemon/prometheus-server-ip`.
Rules added manually must be removed manually.

When a `.deb` is installed over an older bootstrap/source install, package
`postinst` backs up stale `/etc/systemd/system/telemon-exporter.service` units
and stale `/usr/local/bin/telemon-exporter` binaries so the service uses
`/usr/bin/telemon-exporter`. It preserves or recreates the recorded scrape
firewall rule during that migration.

For a full local reset after package removal:

```bash
sudo rm -rf /etc/telemon /var/lib/telemon
```

Validate removal:

```bash
systemctl status telemon-exporter.service
command -v telemon-exporter
curl --connect-timeout 3 http://127.0.0.1:9185/healthz
```

The service should be missing or inactive, `command -v` should not find the
binary, and the curl command should fail to connect.

## Linux Bootstrap Or Source Script

This applies to the root `install.sh` bootstrap path and
`packaging/linux/install.sh`. These install to `/usr/local/bin` and use
`/etc/systemd/system/telemon-exporter.service` when systemd is available.

From the repository or installer source tree:

```bash
sudo bash packaging/linux/uninstall.sh
```

That script stops/disables the systemd service, removes
`/etc/systemd/system/telemon-exporter.service`, removes
`/usr/local/bin/telemon-exporter`, reloads systemd, and removes the
Telemon-managed UFW rule when it was recorded.

For package migration or repair, keep the recorded scrape firewall state:

```bash
sudo bash packaging/linux/uninstall.sh --preserve-firewall
```

For a full local reset:

```bash
sudo rm -rf /etc/telemon /var/lib/telemon
```

If the host does not use systemd and the exporter was started manually with
`nohup`, stop the process first:

```bash
pgrep -af 'telemon-exporter.*run'
```

Then stop the matching PID using your normal process management tools.

## Unraid Native Bootstrap

For Unraid native installs, the recommended startup path is the User Scripts
plugin running the `nohup ... run-telemon-exporter.sh ...` command printed by
`install.sh`.

To uninstall while preserving config and UUID state:

1. Disable or remove the Telemon startup script in the Unraid User Scripts UI.
2. Stop the running exporter process.
3. Remove the runtime binary:

```bash
rm -f /usr/local/bin/telemon-exporter
```

For a full Unraid reset, remove the persistent plugin directory:

```bash
rm -rf /boot/config/plugins/telemon
```

That deletes the persistent binary copy, generated config, runner scripts, and
UUID state.

## Docker Monitoring Stack

This is the central stack from `deploy/docker-compose.yml`: Telemon hub,
Prometheus, and Grafana.

Stop and remove the containers while preserving data volumes:

```bash
docker compose -f deploy/docker-compose.yml down
```

Full reset, including registry device records, Prometheus time series, and
Grafana state:

```bash
docker compose -f deploy/docker-compose.yml down -v --remove-orphans
```

Optional image cleanup:

```bash
docker image rm ghcr.io/<owner>/telemon-hub:edge
```

If you used the local build override, you can also remove the local development
image:

```bash
docker image rm telemon-hub:dev
```

Validate removal:

```bash
docker ps --filter name=telemon
docker volume ls | grep telemon
```

After a full reset, the Telemon compose volumes should be gone.

## Docker Exporter

This is the host-monitoring exporter container from
`deploy/exporter/docker-compose.production.yml`.

Before starting a replacement exporter, check for stale containers or host
processes still binding the scrape ports:

```bash
docker ps --filter name=telemon-exporter
ss -ltnp 'sport = :9185' || true
ss -ltnp 'sport = :9187' || true
```

Stop and remove the container while preserving `/config` state:

```bash
docker compose -f deploy/exporter/docker-compose.production.yml down
```

For a full reset, remove the configured host config directory after taking the
container down:

```bash
sudo rm -rf /srv/telemon/exporter
```

If you set `TELEMON_DOCKER_CONFIG_DIR`, remove that directory instead.
For Unraid Docker installs this is commonly:

```bash
rm -rf /boot/config/plugins/telemon-docker
```

For the development compose file, also remove the named volume:

```bash
docker compose -f deploy/exporter/docker-compose.yml down -v --remove-orphans
```

For the Unraid/OMV validation compose file:

```bash
docker compose -f deploy/exporter/docker-compose.unraid-test.yml down
```

Optional image cleanup:

```bash
docker image rm ghcr.io/<owner>/telemon-exporter:edge
docker image rm telemon-exporter:dev
docker image rm telemon-exporter:unraid-test
```

## Windows Service

Run from an Administrator PowerShell:

```powershell
.\packaging\windows\uninstall-service.ps1
```

The script removes the `TelemonExporter` Windows service and any firewall rules
matching `Telemon Exporter 9185*`. It preserves:

```text
C:\ProgramData\Telemon\exporter.yml
```

For service repair or migration where Prometheus should keep scrape access:

```powershell
.\packaging\windows\uninstall-service.ps1 -PreserveFirewall
```

For a full local reset:

```powershell
.\packaging\windows\uninstall-service.ps1 -RemoveFiles
```

Validate removal:

```powershell
Get-Service TelemonExporter -ErrorAction SilentlyContinue
Test-NetConnection 127.0.0.1 -Port 9185
```

The service lookup should return nothing and the port test should fail.

## macOS

macOS support is currently a LaunchDaemon skeleton. Thermal collection is still
deferred to a later phase.

Uninstall the LaunchDaemon and Telemon-managed `pf` rule:

```bash
sudo packaging/macos/uninstall.sh
```

The script preserves:

```text
/Library/Application Support/Telemon/exporter.yml
```

For service repair or migration where Prometheus should keep scrape access:

```bash
sudo packaging/macos/uninstall.sh --preserve-firewall
```

For a full local reset:

```bash
sudo packaging/macos/uninstall.sh --remove-files
```

Validate removal:

```bash
launchctl print system/com.telemon.exporter
curl --connect-timeout 3 http://127.0.0.1:9185/healthz
```

The LaunchDaemon should be missing and the curl command should fail to connect.
