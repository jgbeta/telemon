# Prometheus Metrics Catalog

This catalog describes the current Prometheus series produced by
`telemon-exporter` and the registry service-discovery labels Prometheus
adds during scraping.

## Scrape Jobs And Target Labels

Prometheus scrapes dynamic telemetry from `/metrics` through device-level
adaptive jobs:

- `telemon-15s`
- `telemon-10s`
- `telemon-5s`
- `telemon-1s`

Prometheus scrapes low-change metadata from `/metrics/static` through:

- `telemon-static`

The registry adds these labels to discovered targets, so Prometheus stores them
on scraped series:

| Label | Source | Notes |
| --- | --- | --- |
| `job` | Prometheus scrape job | One of the dynamic jobs or `telemon-static`. |
| `instance` | Prometheus target | `<host>:<port>` returned by registry service discovery. |
| `device_uuid` | Registry | Opaque UUID assigned during enrollment. |
| `machine_uuid` | Client/registry | Stable physical-machine identity; can be shared across dual-boot installs. |
| `device_name` | Client/registry | Human device label. |
| `user_name` | Client/registry | Human user label. |
| `host` | Registry | Currently mirrors `device_name`. |
| `os` | Client/registry | Rust target OS string such as `linux`. |
| `os_version` | Client/registry | OS display string where available. |
| `arch` | Client/registry | Rust target architecture such as `x86_64`. |
| `requested_scrape_interval_seconds` | Client/registry | Current device-level adaptive scrape bucket. |

Prometheus target labels are the canonical labels for Grafana filtering. Some
static info metrics also emit overlapping labels directly. With Prometheus'
default `honor_labels: false`, conflicting scraped labels can be renamed to
`exported_<label>` while the registry target labels remain canonical.

## Metric Families

| Metric | Type | Endpoint | Source | Exporter labels | Notes |
| --- | --- | --- | --- | --- | --- |
| `up` | gauge | all telemon scrape jobs | Prometheus | target labels | Prometheus-generated scrape success metric; value is `1` when the last scrape succeeded. |
| `telemon_build_info` | gauge | `/metrics/static` | exporter | `version`, `os`, `arch` | Build metadata; value is always `1`. |
| `telemon_device_info` | gauge | `/metrics/static` | registration | `device_uuid`, `machine_uuid`, `user_name`, `device_name`, `os`, `os_version`, `arch` | Emitted only when registration is enabled and a device UUID exists; value is always `1`. |
| `telemon_requested_scrape_interval_seconds` | gauge | `/metrics` | adaptive scheduler | none | Exporter-requested device-level scrape interval. |
| `telemon_collector_up` | gauge | `/metrics` | all collectors | `collector` | `1` when collector is healthy in the last run, otherwise `0`. |
| `telemon_collector_supported` | gauge | `/metrics/static` | all collectors | `collector` | `1` when collector is supported on the host, otherwise `0`. |
| `telemon_collector_errors_total` | counter | `/metrics` | all collectors | `collector` | Total collector errors observed by the exporter process. |
| `telemon_collector_last_success_timestamp_seconds` | gauge | `/metrics` | all collectors | `collector` | Unix timestamp of last successful collector run. |
| `telemon_collector_samples` | gauge | `/metrics` | `linux_hwmon` | `collector`, `kind` | Useful sample count from the last collection run; currently `kind="temperature"`. |
| `telemon_uptime_seconds` | gauge | `/metrics` | `windows_baseline` | `source` | System uptime in seconds from Windows APIs. |
| `telemon_cpu_usage_ratio` | gauge | `/metrics` | `windows_baseline` | `component`, `source` | Total system CPU usage ratio from `0` to `1`; first sample appears after the second collection cycle. |
| `telemon_memory_total_bytes` | gauge | `/metrics/static` | `windows_baseline` | `source` | Total physical memory in bytes. |
| `telemon_memory_available_bytes` | gauge | `/metrics` | `windows_baseline` | `source` | Available physical memory in bytes. |
| `telemon_memory_used_bytes` | gauge | `/metrics` | `windows_baseline` | `source` | Used physical memory in bytes. |
| `telemon_filesystem_size_bytes` | gauge | `/metrics/static` | `windows_baseline` | `source`, `volume`, `drive_type` | Filesystem size for included Windows volumes. |
| `telemon_filesystem_free_bytes` | gauge | `/metrics` | `windows_baseline` | `source`, `volume`, `drive_type` | Free bytes for included Windows volumes. |
| `telemon_filesystem_available_bytes` | gauge | `/metrics` | `windows_baseline` | `source`, `volume`, `drive_type` | Bytes available to the exporter process for included Windows volumes. |
| `telemon_network_receive_bytes_total` | counter | `/metrics` | `windows_baseline` | `source`, `if_index`, `interface` | Total bytes received by a Windows network interface. |
| `telemon_network_transmit_bytes_total` | counter | `/metrics` | `windows_baseline` | `source`, `if_index`, `interface` | Total bytes transmitted by a Windows network interface. |
| `telemon_windows_os_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `os`, `version`, `build`, `arch` | Windows OS identity; value is always `1`. |
| `telemon_cpu_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `model`, `architecture`, `logical_processors` | CPU identity and topology information; value is always `1`. |
| `telemon_computer_system_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `computer_name`, `arch` | Windows computer identity information; value is always `1`. |
| `telemon_hwmon_chips_discovered` | gauge | `/metrics` | `linux_hwmon` | `collector` | Number of Linux hwmon chip directories discovered. |
| `telemon_hwmon_temperature_inputs_discovered` | gauge | `/metrics` | `linux_hwmon` | `collector` | Number of Linux hwmon `temp*_input` files discovered before filtering. |
| `telemon_temperature_celsius` | gauge | `/metrics` | `linux_hwmon`, `nvidia_nvml`, `windows_lhm_http`, `windows_lhm_wmi` | `component`, `sensor`, `source`, optional `gpu_index`, optional `storage_id`, optional `pci_bdf`, optional `storage_model` | Dynamic temperature readings. NVMe storage samples include stable drive labels when sysfs enrichment succeeds. Windows CPU/motherboard/storage samples use LibreHardwareMonitor HTTP by default; WMI is experimental. |
| `telemon_temperature_limit_celsius` | gauge | `/metrics/static` | `linux_hwmon` | `component`, `sensor`, `source`, `limit`, optional `storage_id`, optional `pci_bdf`, optional `storage_model` | Static-ish warning/critical temperature thresholds where available. |
| `telemon_storage_device_info` | gauge | `/metrics/static` | `linux_hwmon` | `source`, `storage_id`, `controller`, optional `pci_bdf`, optional `storage_model`, optional `firmware_rev`, optional `state` | Linux NVMe identity from sysfs; value is always `1`. Serial numbers are not exposed as Prometheus labels. |
| `telemon_storage_namespace_capacity_bytes` | gauge | `/metrics/static` | `linux_hwmon` | `source`, `storage_id`, `namespace` | Linux NVMe namespace capacity in bytes from sysfs block-sector counts. |
| `telemon_gpu_info` | gauge | `/metrics/static` | `nvidia_nvml` | `gpu_index`, `vendor`, `source`, optional `name`, optional `uuid` | NVIDIA GPU identity; value is always `1`. |
| `telemon_gpu_utilization_ratio` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source`, `engine` | Ratio from `0` to `1`; `engine` is `graphics` or `memory`. |
| `telemon_gpu_memory_total_bytes` | gauge | `/metrics/static` | `nvidia_nvml` | `gpu_index`, `source` | Static-ish total VRAM bytes. |
| `telemon_gpu_memory_used_bytes` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source` | Used VRAM bytes. |
| `telemon_gpu_memory_free_bytes` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source` | Free VRAM bytes. |
| `telemon_gpu_power_usage_watts` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source` | Current GPU power usage in watts. |
| `telemon_gpu_power_limit_watts` | gauge | `/metrics/static` | `nvidia_nvml` | `gpu_index`, `source` | Static-ish enforced GPU power limit in watts. |
| `telemon_gpu_clock_hertz` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source`, `clock` | GPU clock speed in hertz; `clock` is `graphics` or `memory`. |
| `telemon_gpu_performance_state` | gauge | `/metrics` | `nvidia_nvml` | `gpu_index`, `source` | Numeric NVIDIA P-state where `P0` is exported as `0`. |
| `telemon_fan_speed_ratio` | gauge | `/metrics` | `nvidia_nvml` | `component`, `gpu_index`, `source` | Ratio from `0` to `1`; emitted only when fan speed is available and enabled. |

## Current Cardinality Notes

- Full dynamic scrapes include all enabled dynamic sensor metrics for a device.
- Sensor-level cardinality is mostly driven by `component`, `sensor`, `source`,
  `gpu_index`, and NVMe `storage_id` when Linux sysfs enrichment is enabled.
- `telemon_gpu_info{name=...}` is enabled by default; GPU UUID labels are
  opt-in through `collectors.nvidia_nvml.expose_gpu_uuid`.
- NVMe model labels are enabled by default through
  `collectors.linux_hwmon.expose_storage_model`; NVMe serial numbers stay
  local-only in `inspect-hardware`.
- Serial numbers, VBIOS versions, and other high-cardinality inspection fields
  stay local-only unless explicitly promoted to metrics later.
- Windows baseline metrics come from Win32 APIs and do not require LibreHardwareMonitor. Generic Windows temperature classes are not used for production CPU temperature. CPU/motherboard/storage temperatures use the optional `windows_lhm_http` collector when LibreHardwareMonitor exposes `http://127.0.0.1:8085/data.json`; `windows_lhm_wmi` is experimental and disabled by default.
- Windows network interface labels use interface aliases and indexes. Use `collectors.windows_baseline.network_interface_allowlist` or `network_interface_denylist` if a host exposes noisy virtual adapters.
- `telemon_device_info` and `telemon_build_info` can overlap with
  registry target labels. Prefer registry labels for dashboard filters.
- Long-term storage optimization should be handled later through downsampling
  or retention policy, not per-sensor scrape scheduling.
