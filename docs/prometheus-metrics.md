# Prometheus Metrics Catalog

This catalog describes the current Prometheus series produced by
`telemon-exporter` and the registry service-discovery labels Prometheus adds
during scraping.

## Scrape Jobs And Target Labels

Prometheus scrapes dynamic telemetry from `/metrics` through device-level
adaptive jobs:

- `telemon-15s`
- `telemon-10s`
- `telemon-5s`
- `telemon-1s`

Prometheus scrapes low-change metadata from `/metrics/static` through:

- `telemon-static`

When Steam Deck FPS telemetry is enabled, Prometheus scrapes aggregate game
metrics from `/fps` through:

- `telemon-fps`

The `/fps` `source` label can be `gamescope_wayland`, `mangohud_log`, or
`gamescope_mangoapp`. Steam Deck Gaming Mode prefers `gamescope_wayland`; the
other sources are fallbacks or diagnostics.

The registry adds these labels to discovered targets, so Prometheus stores them
on scraped series:

| Label | Source | Notes |
| --- | --- | --- |
| `job` | Prometheus scrape job | One of the dynamic jobs, `telemon-static`, or `telemon-fps`. |
| `instance` | Prometheus target | `<host>:<port>` returned by registry service discovery. |
| `device_uuid` | Registry | Opaque UUID assigned during enrollment. |
| `machine_uuid` | Client/registry | Stable physical-machine identity; can be shared across dual-boot installs. |
| `device_name` | Client/registry | Human device label. |
| `user_name` | Client/registry | Human user label. |
| `host` | Registry | Currently mirrors `device_name`. |
| `target_host` | Registry | Actual scrape host selected by the registry; blank advertised addresses use the observed source IP. |
| `os` | Client/registry | Rust target OS string such as `linux` or `windows`. |
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
| `info_build_info` | gauge | `/metrics/static` | exporter | `version`, `os`, `arch` | Build metadata; value is always `1`. |
| `info_device_id` | gauge | `/metrics/static` | registration | `device_uuid`, `machine_uuid`, `user_name`, `device_name`, `os`, `os_version`, `arch` | Emitted only when registration is enabled and a device UUID exists; value is always `1`. |
| `info_requested_scrape_interval_s` | gauge | `/metrics` | adaptive scheduler | none | Exporter-requested device-level scrape interval. |
| `info_snapshot_last_update_ts_s` | gauge | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `kind` | Unix timestamp of the last dynamic/static snapshot cache update. |
| `info_snapshot_age_s` | gauge | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `kind` | Age of the current dynamic/static snapshot cache. Low age with Grafana gaps points away from local collector lag. |
| `info_snapshot_updates_total` | counter | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `kind` | Total dynamic/static snapshot cache updates. |
| `info_scrape_requests_total` | counter | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `endpoint`, `status` | Total scrape requests observed by the exporter for metrics endpoints. |
| `info_scrape_last_request_ts_s` | gauge | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `endpoint` | Unix timestamp of the last scrape request observed for each metrics endpoint. |
| `info_scrape_gap_s` | gauge | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `endpoint` | Seconds between the two most recent scrape requests for each metrics endpoint. |
| `info_scrape_gaps_total` | counter | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `endpoint` | Total scrape request gaps above the configured diagnostics threshold. |
| `info_requested_scrape_interval_changes_total` | counter | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | `from`, `to` | Total requested scrape interval transitions between adaptive buckets. |
| `info_requested_scrape_interval_last_change_ts_s` | gauge | `/metrics`, `/metrics/static`, `/fps/debug` | exporter diagnostics | none | Unix timestamp of the last requested scrape interval transition. |
| `info_collector_up` | gauge | `/metrics` | all collectors | `collector` | `1` when collector is healthy in the last run, otherwise `0`. |
| `info_collector_supported` | gauge | `/metrics/static` | all collectors | `collector` | `1` when collector is supported on the host, otherwise `0`. |
| `info_collector_errors_total` | counter | `/metrics` | all collectors | `collector` | Total collector errors observed by the exporter process. |
| `info_collector_last_success_ts_s` | gauge | `/metrics` | all collectors | `collector` | Unix timestamp of last successful collector run. |
| `info_collector_samples` | gauge | `/metrics` | `linux_hwmon`, `macos_macmon` | `collector`, `kind` | Useful sample count from the last collection run; `linux_hwmon` uses `kind="temperature"` and `macos_macmon` uses `kind="dynamic"`. |
| `info_macmon_snapshot_age_s` | gauge | `/metrics` | `macos_macmon` | none | Age of the latest cached macmon snapshot in seconds. |
| `info_macmon_reinitializations_total` | counter | `/metrics` | `macos_macmon` | none | Total macmon sampler reinitializations. |
| `info_macmon_invalid_samples_total` | counter | `/metrics` | `macos_macmon` | `field` | Total macmon fields skipped during normalization. |
| `game_session_active` | gauge | `/fps` | `steam_deck_fps` | `state_source`, optional `appid`, `title` | `1` when a game session is active. |
| `game_session_focused` | gauge | `/fps` | `steam_deck_fps` | `state_source`, optional `appid`, `title` | `1` when the active game is focused and visible. |
| `game_session_info` | gauge | `/fps` | `steam_deck_fps` | `appid`, `title`, `identity_source` | Game identity resolved from local Steam metadata; value is always `1`. |
| `game_session_start_ts_s` | gauge | `/fps` | `steam_deck_fps` | `state_source`, optional `appid`, `title` | Unix timestamp for the current game session start. |
| `game_fps_source_selected` | gauge | `/fps` | `steam_deck_fps` | `source` | `1` for the FPS source currently selected by Telemon. |
| `game_fps_source_available` | gauge | `/fps` | `steam_deck_fps` | `source` | `1` when Telemon found or connected to a candidate FPS source. |
| `game_fps_source_healthy` | gauge | `/fps` | `steam_deck_fps` | `source` | `1` when valid frame samples were received recently while a game is active. |
| `game_fps_source_samples_total` | counter | `/fps` | `steam_deck_fps` | `source` | Total accepted frame timing samples from the active source. |
| `game_fps_source_sample_drops_total` | counter | `/fps` | `steam_deck_fps` | `source`, `reason` | Frame timing samples dropped by source sanity filters. Reasons: `invalid`, `too_large`, `stale`, `out_of_order`, `wrong_session`. |
| `game_fps_source_sample_last_ts_s` | gauge | `/fps` | `steam_deck_fps` | `source` | Unix timestamp of the last accepted frame timing sample. |
| `game_fps_source_sample_age_s` | gauge | `/fps` | `steam_deck_fps` | `source` | Age of the last accepted frame timing sample. |
| `game_fps_source_sample_interval_ms` | gauge | `/fps` | `steam_deck_fps` | `source` | Wall-clock interval since the previous accepted frame timing sample. |
| `game_frame_samples` | gauge | `/fps` | `steam_deck_fps` | `source`, `window`, optional `appid`, `title` | Frame samples in the rolling window. |
| `game_fps` | gauge | `/fps` | `steam_deck_fps` | `source`, `window`, `stat`, optional `appid`, `title` | FPS by rolling-window statistic. `stat` is `avg`, `low_1pct`, or `low_0_1pct`. |
| `game_frame_ms` | gauge | `/fps` | `steam_deck_fps` | `source`, `window`, `stat`, optional `appid`, `title` | Frame time in milliseconds by statistic. `stat` is `avg`, `min`, `max`, `p50`, `p95`, or `p99`. |
| `game_jitter_ms` | gauge | `/fps` | `steam_deck_fps` | `source`, `window`, `stat`, optional `appid`, `title` | Adjacent frame-time delta in milliseconds by statistic. `stat` is `avg`, `p95`, `p99`, or `max`. |
| `game_fps_source_backend_info` | gauge | `/fps/debug` | `steam_deck_fps` | `source`, `queue` | Backend-specific FPS source metadata; value is always `1`. |
| `game_fps_source_sample_payload_bytes` | gauge | `/fps/debug` | `steam_deck_fps` | `source`, `queue` | Payload bytes in the last accepted backend sample when available. |
| `game_fps_source_output_pixels` | gauge | `/fps/debug` | `steam_deck_fps` | `source`, `queue`, `axis` | Output width/height reported by the backend when available. |
| `hw_macmon_cpu_temp_c` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Average CPU temperature in Celsius from macmon. |
| `hw_macmon_gpu_temp_c` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Average GPU temperature in Celsius from macmon. |
| `hw_macmon_cpu_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | CPU power consumption in watts from macmon. |
| `hw_macmon_gpu_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | GPU power consumption in watts from macmon. |
| `hw_macmon_ane_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | ANE power consumption in watts from macmon. |
| `hw_macmon_soc_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Combined SoC power consumption in watts from macmon. |
| `hw_macmon_system_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | System power consumption in watts from macmon. |
| `hw_macmon_ram_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | RAM power consumption in watts from macmon. |
| `hw_macmon_gpu_ram_power_w` | gauge | `/metrics` | `macos_macmon` | optional `chip` | GPU RAM power consumption in watts from macmon. |
| `sys_macmon_cpu_usage_ratio` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Combined CPU utilization ratio from macmon. |
| `hw_macmon_ecpu_usage_ratio` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Efficiency CPU cluster utilization ratio from macmon. |
| `hw_macmon_pcpu_usage_ratio` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Performance CPU cluster utilization ratio from macmon. |
| `hw_macmon_gpu_usage_ratio` | gauge | `/metrics` | `macos_macmon` | optional `chip` | GPU utilization ratio from macmon. |
| `hw_macmon_ecpu_freq_mhz` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Efficiency CPU cluster frequency in MHz from macmon. |
| `hw_macmon_pcpu_freq_mhz` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Performance CPU cluster frequency in MHz from macmon. |
| `hw_macmon_gpu_freq_mhz` | gauge | `/metrics` | `macos_macmon` | optional `chip` | GPU frequency in MHz from macmon. |
| `sys_macmon_ram_used_mb` | gauge | `/metrics` | `macos_macmon` | optional `chip` | RAM usage in decimal MB from macmon. |
| `sys_macmon_ram_total_mb` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Total RAM in decimal MB from macmon. |
| `sys_macmon_swap_used_mb` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Swap usage in decimal MB from macmon. |
| `sys_macmon_swap_total_mb` | gauge | `/metrics` | `macos_macmon` | optional `chip` | Total swap in decimal MB from macmon. |
| `sys_uptime_s` | gauge | `/metrics` | `system`, `windows_baseline` | `source` | System uptime in seconds. |
| `sys_cpu_count` | gauge | `/metrics/static` | `system` | `source` | Logical CPU count. |
| `sys_cpu_usage_ratio` | gauge | `/metrics` | `system`, `windows_baseline`, `macos_macmon` | `source`, optional `component` | Total system CPU usage ratio from `0` to `1`; collectors may omit it until a reliable delta sample is available. |
| `sys_cpu_freq_mhz` | gauge | `/metrics` | `system`, `windows_baseline`, `macos_macmon` | `source`, `scope`, optional `cpu`, optional `cluster`, `state` | Current CPU frequency in decimal MHz. Linux and Windows emit `scope="logical_cpu"` with `cpu`; macOS macmon emits `scope="cluster"` with `cluster="efficiency"` or `cluster="performance"`. |
| `sys_mem_mb` | gauge | `/metrics`, `/metrics/static` | `system`, `windows_baseline`, `macos_macmon` | `source`, `kind`, `state` | Physical memory in decimal MB; `state` is `total`, `available`, or `used`. `total` is static. |
| `sys_mem_mb` | gauge | `/metrics`, `/metrics/static` | `macos_macmon` | `source`, `state` | Swap in decimal MB; `state` is `total` or `used`. `total` is static. |
| `sys_thermal_state` | gauge | `/metrics` | `macos_thermal_state` | `source`, `state` | macOS thermal pressure state as one-hot gauges for `nominal`, `fair`, `serious`, `critical`, and `unknown`; `source="macos_processinfo"`. |
| `sys_thermal_state_value` | gauge | `/metrics` | `macos_thermal_state` | `source` | macOS thermal pressure numeric state where `unknown=-1`, `nominal=0`, `fair=1`, `serious=2`, and `critical=3`; `source="macos_processinfo"`. |
| `sys_fs_space_mb` | gauge | `/metrics`, `/metrics/static` | `windows_baseline` | `source`, `volume`, `drive_type`, `state` | Filesystem space in decimal MB; `state` is `size`, `free`, or `available`. `size` is static. |
| `net_bytes_total` | counter | `/metrics` | `windows_baseline` | `source`, `if_index`, `interface`, `direction` | Total network bytes; `direction` is `receive` or `transmit`. |
| `sys_os_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `os`, optional `version`, `build`, optional `arch` | OS identity; value is always `1`. |
| `sys_cpu_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `model`, `architecture`, `logical_processors` | CPU identity and topology information; value is always `1`. |
| `sys_computer_info` | gauge | `/metrics/static` | `windows_inventory` | `source`, `computer_name`, `arch` | Windows computer identity information; value is always `1`. |
| `info_hwmon_chips_discovered` | gauge | `/metrics` | `linux_hwmon` | `collector` | Number of Linux hwmon chip directories discovered. |
| `info_hwmon_temp_inputs_discovered` | gauge | `/metrics` | `linux_hwmon` | `collector` | Number of Linux hwmon `temp*_input` files discovered before filtering. |
| `hw_temp_c` | gauge | `/metrics` | `linux_hwmon`, `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `windows_lhm_http`, `windows_lhm_wmi`, `macos_macmon` | `component`, `sensor`, `source`, optional `device_id`, optional `sensor_instance`, optional `source_driver`, optional source-specific labels | Dynamic hardware temperatures. Linux hwmon and Windows LHM use canonical sensor names such as `cpu_package_temp`, `cpu_core_temp`, `nvme_composite_temp`, `gpu_edge_temp`, and `vrm_temp`. `macos_macmon` emits average CPU/GPU temperatures with `source="macmon"`. |
| `hw_temp_limit_c` | gauge | `/metrics/static` | `linux_hwmon` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver`, `limit`, optional storage labels | Static-ish warning/critical temperature thresholds where available. |
| `hw_sensor_info` | gauge | `/metrics/static` | `linux_hwmon`, `windows_lhm_http` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver`, `raw_label`, `raw_channel`, `confidence` | Raw sensor mapping metadata; value is always `1`. Kept out of dynamic temperature labels to reduce churn. |
| `hw_device_info` | gauge | `/metrics/static` | `linux_hwmon`, `linux_power_supply`, `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `macos_macmon` | `component`, `source`, optional `device_id`, optional `source_driver`, source-specific identity labels | Hardware identity information for GPUs, NVMe storage, and Apple Silicon SoC identity; value is always `1`. Serial numbers are not exposed as Prometheus labels. |
| `hw_cpu_cluster_cores` | gauge | `/metrics/static` | `macos_macmon` | `cluster`, `source` | Apple Silicon CPU core count by efficiency/performance cluster. |
| `hw_gpu_cores` | gauge | `/metrics/static` | `macos_macmon` | `gpu_index`, `source` | Apple Silicon GPU core count. |
| `hw_freq_available_mhz` | gauge | `/metrics/static` | `macos_macmon` | `component`, `state`, `source`, optional `cluster`, optional `gpu_index` | Available Apple Silicon CPU/GPU clock states converted from MHz to hertz. |
| `storage_capacity_mb` | gauge | `/metrics/static` | `linux_hwmon` | `component`, `device_id`, `source`, `source_driver`, `storage_id`, `namespace` | Linux NVMe namespace capacity in decimal MB from sysfs block-sector counts. |
| `hw_voltage_v` | gauge | `/metrics` | `linux_drm`, `windows_lhm_http` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver` | Voltage readings from Linux DRM hwmon and LibreHardwareMonitor HTTP. |
| `hw_current_a` | gauge | `/metrics` | `windows_lhm_http` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver` | Current readings from LibreHardwareMonitor HTTP. |
| `batt_charge_ratio` | gauge | `/metrics` | `linux_power_supply` | `battery`, `source` | Battery charge as a ratio from `0` to `1`; uses `charge_now/charge_full` when available and falls back to `capacity`. |
| `batt_voltage_v` | gauge | `/metrics` | `linux_power_supply` | `battery`, `source` | Battery voltage converted from microvolts to volts. |
| `batt_current_a` | gauge | `/metrics` | `linux_power_supply` | `battery`, `source` | Battery current converted from microamps to amperes. |
| `batt_power_w` | gauge | `/metrics` | `linux_power_supply` | `battery`, `direction`, `derived`, `source` | Battery power in watts; `derived="true"` means volts multiplied by amps because `power_now` was absent. |
| `hw_power_w` | gauge | `/metrics` | `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `windows_lhm_http`, `macos_macmon` | `component`, `source`, optional `device_id`, `sensor`, `sensor_instance`, `source_driver`, `gpu_index` | Current hardware power usage in watts. `macos_macmon` emits CPU, GPU, ANE, SoC, system, RAM, and GPU RAM power when available. |
| `hw_power_limit_w` | gauge | `/metrics/static` | `linux_drm`, `nvidia_nvml`, `windows_lhm_http` | `component`, `device_id`, `sensor`, `source`, `source_driver`, `limit`, optional `gpu_index` | Static-ish enforced/current power limit in watts. |
| `hw_freq_mhz` | gauge | `/metrics` | `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `windows_lhm_http`, `macos_macmon` | `component`, `source`, optional `device_id`, `sensor`, `clock`, `unit`, `cluster`, `source_driver`, `gpu_index` | Hardware frequency in decimal MHz. `macos_macmon` converts CPU/GPU MHz values to hertz. |
| `hw_util_ratio` | gauge | `/metrics` | `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `windows_lhm_http`, `macos_macmon` | `component`, `source`, optional `device_id`, `sensor`, `engine`, `unit`, `cluster`, `source_driver`, `gpu_index` | Utilization ratio from `0` to `1`. |
| `hw_mem_mb` | gauge | `/metrics`, `/metrics/static` | `linux_amdgpu`, `linux_drm`, `nvidia_nvml`, `windows_lhm_http` | `component`, `device_id`, `memory`, `state`, `source`, `source_driver`, optional `gpu_index` | Hardware memory in decimal MB; GPU VRAM uses `memory="vram"` and `state` values `total`, `used`, and `free`. |
| `hw_fan_ratio` | gauge | `/metrics` | `nvidia_nvml`, `windows_lhm_http` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver`, optional `gpu_index` | Fan speed as a ratio from `0` to `1` where available. |
| `hw_fan_rpm` | gauge | `/metrics` | `linux_hwmon`, `linux_drm`, `windows_lhm_http` | `component`, `device_id`, `sensor`, `sensor_instance`, `source`, `source_driver` | Fan or pump speed in RPM; `0` can be a valid stopped reading. |
| `hw_state` | gauge | `/metrics` | `linux_amdgpu`, `linux_drm`, `nvidia_nvml` | `component`, `device_id`, `sensor`, `state`, `source`, `source_driver`, `gpu_index` | Numeric hardware state values such as NVIDIA P-state, GPU throttle reason flags, and Linux DRM throttle states. |

## Current Cardinality Notes

- Full dynamic scrapes include all enabled dynamic sensor metrics for a device.
- Hardware metric families use `component`, `device_id`, `sensor`, and
  `sensor_instance` as the common shape. Source-specific labels such as `gpu_index`,
  `storage_id`, `pci_bdf`, and `storage_model` are additive.
- `hw_sensor_info` carries raw sensor names, raw channels, and mapping
  confidence so dynamic metrics do not need those high-cardinality labels.
- `hw_device_info{name=...}` for NVIDIA GPUs is enabled by default; GPU
  UUID labels are opt-in through `collectors.nvidia_nvml.expose_gpu_uuid`.
- NVMe model labels are enabled by default through
  `collectors.linux_hwmon.expose_storage_model`; NVMe serial numbers stay
  local-only in `inspect-hardware`.
- Serial numbers, VBIOS versions, and other high-cardinality inspection fields
  stay local-only unless explicitly promoted to metrics later.
- Windows baseline metrics come from Win32 APIs and do not require LibreHardwareMonitor. CPU/motherboard/storage temperatures and other hardware sensor values use the optional `windows_lhm_http` collector when LibreHardwareMonitor exposes `http://127.0.0.1:8085/data.json`; `windows_lhm_wmi` is experimental and disabled by default.
- macOS baseline metrics use public APIs for uptime, memory, CPU count, and
  thermal pressure state. Exact macOS CPU/GPU sensor temperatures are not
  guaranteed and are not part of the stable baseline.
- The optional `macos_macmon` collector uses the `macmon` Rust library directly
  on Apple Silicon. It keeps `/metrics` fast by scraping cached snapshots from a
  background sampler thread and does not run a sidecar metrics server. It also
  exposes `/json` with a valid JSON snapshot shaped like macmon's native model.
- Windows network interface labels use interface aliases and indexes. Use `collectors.windows_baseline.network_interface_allowlist` or `network_interface_denylist` if a host exposes noisy virtual adapters.
- `info_device_id` and `info_build_info` can overlap with registry target
  labels. Prefer registry labels for dashboard filters.
- Steam Deck telemetry uses `linux_power_supply` for battery metrics and `linux_amdgpu` for AMD GPU utilization, clocks, VRAM/GTT memory, and APU `gpu_metrics` CPU temperature/power when the binary table is readable. Gamescope game-state detection only changes the requested scrape interval; it does not add per-game labels.
- Long-term storage optimization should be handled later through downsampling
  or retention policy, not per-sensor scrape scheduling.
