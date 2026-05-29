pub const ALLOWED_PREFIXES: &[&str] = &[
    "hardware_",
    "system_",
    "filesystem_",
    "network_",
    "exporter_",
    "device_",
    "macmon_",
];

pub const BUILD_INFO: &str = "exporter_build_info";
pub const DEVICE_INFO: &str = "device_info";
pub const REQUESTED_SCRAPE_INTERVAL_SECONDS: &str = "exporter_requested_scrape_interval_seconds";
pub const COLLECTOR_UP: &str = "exporter_collector_up";
pub const COLLECTOR_SUPPORTED: &str = "exporter_collector_supported";
pub const COLLECTOR_ERRORS_TOTAL: &str = "exporter_collector_errors_total";
pub const COLLECTOR_LAST_SUCCESS_TIMESTAMP_SECONDS: &str =
    "exporter_collector_last_success_timestamp_seconds";
pub const COLLECTOR_SAMPLES: &str = "exporter_collector_samples";
pub const UPTIME_SECONDS: &str = "system_uptime_seconds";
pub const CPU_USAGE_RATIO: &str = "system_cpu_usage_ratio";
pub const SYSTEM_CPU_COUNT: &str = "system_cpu_count";
pub const SYSTEM_THERMAL_STATE: &str = "system_thermal_state";
pub const SYSTEM_THERMAL_STATE_VALUE: &str = "system_thermal_state_value";
pub const SYSTEM_OS_INFO: &str = "system_os_info";
pub const MEMORY_TOTAL_BYTES: &str = "system_memory_bytes";
pub const MEMORY_AVAILABLE_BYTES: &str = "system_memory_bytes";
pub const MEMORY_USED_BYTES: &str = "system_memory_bytes";
pub const SYSTEM_SWAP_BYTES: &str = "system_swap_bytes";
pub const FILESYSTEM_SIZE_BYTES: &str = "filesystem_bytes";
pub const FILESYSTEM_FREE_BYTES: &str = "filesystem_bytes";
pub const FILESYSTEM_AVAILABLE_BYTES: &str = "filesystem_bytes";
pub const NETWORK_RECEIVE_BYTES_TOTAL: &str = "network_bytes_total";
pub const NETWORK_TRANSMIT_BYTES_TOTAL: &str = "network_bytes_total";
pub const WINDOWS_OS_INFO: &str = SYSTEM_OS_INFO;
pub const CPU_INFO: &str = "hardware_cpu_info";
pub const COMPUTER_SYSTEM_INFO: &str = "system_computer_info";
pub const HWMON_CHIPS_DISCOVERED: &str = "hardware_hwmon_chips_discovered";
pub const HWMON_TEMPERATURE_INPUTS_DISCOVERED: &str =
    "hardware_hwmon_temperature_inputs_discovered";
pub const TEMPERATURE_CELSIUS: &str = "hardware_temperature_celsius";
pub const TEMPERATURE_LIMIT_CELSIUS: &str = "hardware_temperature_limit_celsius";
pub const HARDWARE_VOLTAGE_VOLTS: &str = "hardware_voltage_volts";
pub const HARDWARE_CURRENT_AMPERES: &str = "hardware_current_amperes";
pub const HARDWARE_POWER_WATTS: &str = "hardware_power_watts";
pub const HARDWARE_POWER_LIMIT_WATTS: &str = "hardware_power_limit_watts";
pub const HARDWARE_CLOCK_HERTZ: &str = "hardware_clock_hertz";
pub const HARDWARE_UTILIZATION_RATIO: &str = "hardware_utilization_ratio";
pub const HARDWARE_MEMORY_BYTES: &str = "hardware_memory_bytes";
pub const HARDWARE_SENSOR_INFO: &str = "hardware_sensor_info";
pub const HARDWARE_DEVICE_INFO: &str = "hardware_device_info";
pub const HARDWARE_CPU_CLUSTER_CORES: &str = "hardware_cpu_cluster_cores";
pub const HARDWARE_GPU_CORES: &str = "hardware_gpu_cores";
pub const HARDWARE_CLOCK_AVAILABLE_HERTZ: &str = "hardware_clock_available_hertz";
pub const STORAGE_DEVICE_INFO: &str = HARDWARE_DEVICE_INFO;
pub const STORAGE_NAMESPACE_CAPACITY_BYTES: &str = "hardware_storage_capacity_bytes";
pub const GPU_INFO: &str = HARDWARE_DEVICE_INFO;
pub const GPU_UTILIZATION_RATIO: &str = "hardware_utilization_ratio";
pub const GPU_MEMORY_TOTAL_BYTES: &str = "hardware_memory_bytes";
pub const GPU_MEMORY_USED_BYTES: &str = "hardware_memory_bytes";
pub const GPU_MEMORY_FREE_BYTES: &str = "hardware_memory_bytes";
pub const GPU_POWER_USAGE_WATTS: &str = "hardware_power_watts";
pub const GPU_POWER_LIMIT_WATTS: &str = "hardware_power_limit_watts";
pub const GPU_CLOCK_HERTZ: &str = "hardware_clock_hertz";
pub const GPU_PERFORMANCE_STATE: &str = "hardware_state";
pub const FAN_SPEED_RATIO: &str = "hardware_fan_speed_ratio";
pub const HARDWARE_FAN_SPEED_RPM: &str = "hardware_fan_speed_rpm";
pub const EXPORTER_MACOS_MACMON_SNAPSHOT_AGE_SECONDS: &str =
    "exporter_macos_macmon_snapshot_age_seconds";
pub const EXPORTER_MACOS_MACMON_REINITIALIZATIONS_TOTAL: &str =
    "exporter_macos_macmon_reinitializations_total";
pub const EXPORTER_MACOS_MACMON_INVALID_SAMPLES_TOTAL: &str =
    "exporter_macos_macmon_invalid_samples_total";
pub const MACMON_CPU_TEMP_CELSIUS: &str = "macmon_cpu_temp_celsius";
pub const MACMON_GPU_TEMP_CELSIUS: &str = "macmon_gpu_temp_celsius";
pub const MACMON_CPU_POWER_WATTS: &str = "macmon_cpu_power_watts";
pub const MACMON_GPU_POWER_WATTS: &str = "macmon_gpu_power_watts";
pub const MACMON_ANE_POWER_WATTS: &str = "macmon_ane_power_watts";
pub const MACMON_ALL_POWER_WATTS: &str = "macmon_all_power_watts";
pub const MACMON_SYS_POWER_WATTS: &str = "macmon_sys_power_watts";
pub const MACMON_RAM_POWER_WATTS: &str = "macmon_ram_power_watts";
pub const MACMON_GPU_RAM_POWER_WATTS: &str = "macmon_gpu_ram_power_watts";
pub const MACMON_CPU_USAGE_RATIO: &str = "macmon_cpu_usage_ratio";
pub const MACMON_ECPU_USAGE_RATIO: &str = "macmon_ecpu_usage_ratio";
pub const MACMON_PCPU_USAGE_RATIO: &str = "macmon_pcpu_usage_ratio";
pub const MACMON_GPU_USAGE_RATIO: &str = "macmon_gpu_usage_ratio";
pub const MACMON_ECPU_FREQUENCY_MHZ: &str = "macmon_ecpu_frequency_mhz";
pub const MACMON_PCPU_FREQUENCY_MHZ: &str = "macmon_pcpu_frequency_mhz";
pub const MACMON_GPU_FREQUENCY_MHZ: &str = "macmon_gpu_frequency_mhz";
pub const MACMON_MEMORY_RAM_USED_BYTES: &str = "macmon_memory_ram_used_bytes";
pub const MACMON_MEMORY_RAM_TOTAL_BYTES: &str = "macmon_memory_ram_total_bytes";
pub const MACMON_MEMORY_SWAP_USED_BYTES: &str = "macmon_memory_swap_used_bytes";
pub const MACMON_MEMORY_SWAP_TOTAL_BYTES: &str = "macmon_memory_swap_total_bytes";
