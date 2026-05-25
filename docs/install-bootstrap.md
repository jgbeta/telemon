# Bootstrap Linux Install

Use the root `install.sh` when you need a quick Linux install path for testing,
homelab machines, unsupported distributions, or Unraid-like systems. This is the
compatibility bootstrap path, not the long-term package format. Prefer the
`.deb` package on Debian/Ubuntu when possible. On Unraid, this native bootstrap
path plus the User Scripts plugin is the validated path for host sensor access.
Use the Docker exporter when you specifically want a container-based Linux
server or NAS install.

## What It Does

- Installs a prebuilt `tar.gz` artifact or raw `telemon-exporter` binary.
- Preserves existing config unless `--force-config` is passed.
- Writes or updates registry enrollment settings when provided.
- Installs and restarts a systemd service when systemd is available.
- Prints exact manual run commands when systemd is unavailable.
- Uses `/boot/config/plugins/telemon` for persistent Unraid config/state,
  then copies the binary to `/usr/local/bin` before launch.

## Prerequisites

Start the monitoring stack first and confirm the client can reach the registry:

```bash
curl http://<server-ip>:9186/healthz
```

Build the release artifact on a development/build machine:

```bash
scripts/build-release.sh
```

Copy the generated artifact to the target machine:

```text
dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz
```

The target machine does not need Cargo. The normal path is to run from the
repository root or copied installer bundle:

```bash
sudo bash install.sh --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz
```

You can also extract the bundle and run its included installer. In that mode,
`install.sh` uses the bundled `telemon-exporter` binary automatically:

```bash
tar -xzf telemon-exporter-v<version>-linux-<arch>.tar.gz
cd telemon-exporter-v<version>-linux-<arch>
sudo bash install.sh
```

For fast local testing, you can still pass a raw binary path:

```bash
sudo bash install.sh /path/to/telemon-exporter
```

## Install With Registry Enrollment

```bash
sudo bash install.sh \
  --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name linux-desktop \
  --advertised-addr exporter.example.local
```

If you omit registry values in an interactive terminal, the script prompts for
them. Blank user and device values default to the installing user and hostname.
Blank registry server disables registration. Blank advertised address tells the
registry to use the connection source IP as the Prometheus scrape target. Blank
machine UUID lets the exporter create or reuse its local physical-machine UUID.

Useful options:

```text
--registry-server HOST:PORT
--enrollment-token TOKEN
--user-name NAME
--device-name NAME
--machine-uuid UUID
--advertised-addr HOST_OR_IP
--prometheus-server-ip IP
--force-config
--dry-run
```

Use `--machine-uuid` only when you intentionally want several OS installs to
represent the same physical machine, such as a dual-boot system. Each OS keeps
its own registry `device_uuid`; the shared `machine_uuid` lets Grafana group
them while the `os` and `device_uuid` labels still distinguish sessions.

Use `--dry-run` to validate artifact resolution and planned install paths
without writing files:

```bash
bash install.sh \
  --dry-run \
  --artifact dist/current/telemon-exporter-v<version>-linux-<arch>.tar.gz \
  --registry-server registry.example.local:9186 \
  --enrollment-token change-me \
  --user-name example-user \
  --device-name linux-desktop
```

## Installed Paths

Normal Linux:

```text
Binary:  /usr/local/bin/telemon-exporter
Config:  /etc/telemon/exporter.yml
State:   /var/lib/telemon/exporter/device-id
Service: /etc/systemd/system/telemon-exporter.service
```

Unraid-like systems:

```text
Runtime binary:     /usr/local/bin/telemon-exporter
Persistent binary:  /boot/config/plugins/telemon/telemon-exporter
Config:  /boot/config/plugins/telemon/exporter.yml
State:   /boot/config/plugins/telemon/state/device-id
Runner:  /boot/config/plugins/telemon/run-telemon-exporter.sh
Alias:   /boot/config/plugins/telemon/exporter.sh
```

## Non-Systemd And Unraid

When systemd is not available, the installer does not fail after installing the
binary and config. It prints a command like:

```bash
nohup /usr/local/bin/telemon-exporter run --config /etc/telemon/exporter.yml >> /var/log/telemon-exporter.log 2>&1 &
```

On Unraid-like systems, it prints a persistent command using
`run-telemon-exporter.sh`. It also writes `exporter.sh` as a shorter alias.
Add the printed `nohup ...` line through the Unraid User Scripts plugin and set
the script to run at array start if you want the exporter to start on boot.

The installer does not edit `/boot/config/go` automatically. User Scripts is
preferred for this bootstrap path because it keeps the startup command visible
in the Unraid UI and avoids relying on manual boot-file edits.

## Verify

```bash
curl http://127.0.0.1:9185/healthz
curl http://127.0.0.1:9185/metrics
curl http://127.0.0.1:9185/metrics/static
curl http://127.0.0.1:9185/metrics | grep 'source="linux_hwmon"'
```

On Unraid, seeing `hardware_temperature_celsius` values with
`source="linux_hwmon"` confirms the host hwmon collector can read CPU, board, or
storage sensors exposed by the kernel. NVIDIA metrics are separate; if
`exporter_collector_supported{collector="nvidia_nvml"} 0` appears, NVML is
not available to the exporter yet.

From the monitoring server:

```bash
curl http://<exporter-lan-ip>:9185/metrics
curl http://<exporter-lan-ip>:9185/metrics/static
curl http://<server-ip>:9186/prometheus/sd
```

For systemd:

```bash
systemctl status telemon-exporter
journalctl -u telemon-exporter -n 100
```

## Reruns

Running `install.sh` again is safe for the bootstrap use case:

- The binary is updated.
- Existing config is preserved.
- Registry fields are updated only when registry options are provided.
- The systemd service is replaced and restarted when systemd is available.

Use `--force-config` only when you intentionally want to replace
`exporter.yml`. The old file is backed up first.

## Troubleshooting

Missing artifact:

```text
provide a prebuilt artifact or binary path; target installs should not require Cargo
```

Build `dist/current/*.tar.gz` on a build machine, download a GitHub Release
artifact, or pass an explicit binary path:

```bash
sudo bash install.sh --artifact /path/to/telemon-exporter-v<version>-linux-<arch>.tar.gz
```

Registry token error:

```text
registry server requires an enrollment token
```

Pass `--enrollment-token` when `--registry-server` is set.

Prometheus cannot scrape:

- Confirm the exporter listens on TCP `9185`.
- Confirm the client firewall allows the monitoring server.
- Use `--prometheus-server-ip <server-ip>` on UFW-based systems. Installer-managed UFW rules are recorded under `/etc/telemon` and removed by the Linux uninstall path; manually-created firewall rules must be removed manually.
- Check `/prometheus/sd` on the registry for `device_uuid`, `user_name`, and
  `device_name` labels.
- Check `/prometheus/sd/15s`, `/prometheus/sd/5s`, and `/prometheus/sd/1s` if
  adaptive sampling does not appear to move a target between scrape buckets.
