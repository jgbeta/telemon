# Troubleshooting

## Basic Checks

```bash
cargo run -p telemon-cli -- exporter check --config config.example.yml
cargo run -p telemon-cli -- exporter discover --config config.example.yml
cargo run -p telemon-cli -- exporter print-metrics --config config.example.yml
curl -v http://127.0.0.1:9185/healthz
curl -v http://127.0.0.1:9185/readyz
curl -v http://127.0.0.1:9185/metrics
curl -v http://127.0.0.1:9185/metrics/static
```

## Debug Bundle

```bash
bash scripts/collect-debug-info.sh
```

The script writes a timestamped directory under `debug-bundles/`. It is read-only and continues when optional commands fail.

## Common Failures

`cargo run -p telemon-cli -- exporter check` fails:

- YAML indentation error.
- Invalid `server.listen`.
- `server.metrics_path` does not start with `/`.
- Zero collection interval.
- Invalid logging level.

`/readyz` returns `503`:

- Scheduler has not produced the first snapshot.
- All collectors are disabled.
- A collector is failing before the cache update.

Prometheus target is down:

- Exporter is not running.
- Exporter is bound to `127.0.0.1` while Prometheus runs in Docker.
- Device is missing or stale in the registry `/prometheus/sd` response.
- Device is in a different adaptive bucket than expected; check
  `/prometheus/sd/15s`, `/prometheus/sd/10s`, `/prometheus/sd/5s`, and
  `/prometheus/sd/1s`.
- Firewall blocks port `9185`.

Exporter fails with `Address already in use`:

- Another exporter is already listening on the same host port.
- With Docker `network_mode: host`, `0.0.0.0:9185` is the host port, not an
  isolated container port.
- Use `ss -ltnp | grep -E ':9185|:9187'` to find the owner.
- For side-by-side validation, keep native on `9185` and use the Docker test
  compose on `9187`.

Linux hwmon reports no sensors:

- No supported chips found.
- `include_unknown_sensors` is false.
- The host is a VM.
- Kernel driver is not loaded.
- Files are unreadable by the service user.
- Filters excluded the normalized sensor labels.
- Docker exporter mounts only `/sys/class/hwmon` instead of full `/sys` at
  `/host/sys`.

Safe hwmon listing:

```bash
find /sys/class/hwmon -maxdepth 2 -type f \( -name "name" -o -name "temp*_input" -o -name "temp*_label" -o -name "temp*_crit" -o -name "temp*_max" \) -print
```

Do not write to sysfs or hardware control paths.

Unraid startup:

- For the native bootstrap path, add the installer's printed `nohup ...
  run-telemon-exporter.sh` command to the Unraid User Scripts plugin and
  run it at array start.
- Verify locally with
  `curl http://127.0.0.1:9185/metrics | grep 'source="linux_hwmon"'`.
- If Docker shows fewer sensors than native, check that `/sys` is mounted
  read-only at `/host/sys` and set
  `TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN=true`.
- Check `telemon_collector_samples{collector="linux_hwmon",kind="temperature"}`,
  `telemon_hwmon_chips_discovered`, and
  `telemon_hwmon_temperature_inputs_discovered` to distinguish zero host
  sensors from filters or classification gaps.
- For NVMe drive identity, verify that `/sys/class/hwmon/hwmon*/name` reports
  `nvme` and that the canonical hwmon path contains `/nvme/nvmeN/`. Use
  `telemon exporter inspect-hardware --config <config> --format json` to see
  local-only serials, firmware, PCI BDFs, and namespace capacity.

NVIDIA NVML collector reports `library_missing`:

- NVIDIA drivers are not installed on the host.
- NVML is not in the platform loader path.
- `collectors.nvidia_nvml.library_paths` points to the wrong file.

NVIDIA NVML collector reports `no_devices`:

- NVML loaded successfully, but no NVIDIA GPU is visible.
- The host is a VM or container without direct GPU access.
- The driver is installed, but the GPU is not initialized or is hidden from the service user.

NVIDIA GPU metrics are partial:

- Fan speed is not available on every GPU and is skipped when unsupported.
- Temperature/utilization/memory calls are collected independently, so one failed call does not suppress the other GPU metrics.
- `telemon_collector_errors_total{collector="nvidia_nvml"}` increments when individual NVML calls fail.

Useful NVIDIA checks:

```bash
cargo run -p telemon-cli -- exporter discover --config config.example.yml
cargo run -p telemon-cli -- exporter print-metrics --config config.example.yml
```

For raw local hardware discovery, use the inspection command. It prints JSON
with readable hwmon attributes, emitted/skipped hwmon temperature inputs, NVML
load status, GPU identity, and best-effort NVML fields such as serial, VBIOS,
power, clocks, and performance state. This output is local debug data; serial numbers, VBIOS versions, and other
identity/debug fields are not exported as Prometheus labels by default.

```bash
cargo run -p telemon-cli -- exporter inspect-hardware --config config.example.yml --format json
telemon-exporter inspect-hardware --config /etc/telemon/exporter.yml --format json
```

If you know where NVML is installed, add it explicitly:

```yaml
collectors:
  nvidia_nvml:
    library_paths:
      - "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1"
```

## Packaging Checks

Validate packaging files locally:

```bash
bash scripts/check-packaging-files.sh
```

Build a Linux package:

```bash
bash packaging/linux/package-deb.sh
```

Common packaging failures:

- `dpkg-deb is required`: install Debian packaging tools or build on Debian/Ubuntu.
- Service starts but `/metrics` is unavailable: check bind address, firewall, and service logs.
- Bootstrap artifact path confusion: run `bash install.sh --dry-run --artifact <artifact.tar.gz> ...` before installing.
- Windows service install fails: run PowerShell as Administrator and verify the release `.exe` path.
- macOS LaunchDaemon does not load: check plist ownership `root:wheel`, mode `644`, and `launchctl print system/com.telemon.exporter`.
