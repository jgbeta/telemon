pub const ALLOWED_PREFIXES: &[&str] = &[
    "hardware_",
    "system_",
    "filesystem_",
    "network_",
    "exporter_",
    "device_",
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
pub const MEMORY_TOTAL_BYTES: &str = "system_memory_bytes";
pub const MEMORY_AVAILABLE_BYTES: &str = "system_memory_bytes";
pub const MEMORY_USED_BYTES: &str = "system_memory_bytes";
pub const FILESYSTEM_SIZE_BYTES: &str = "filesystem_bytes";
pub const FILESYSTEM_FREE_BYTES: &str = "filesystem_bytes";
pub const FILESYSTEM_AVAILABLE_BYTES: &str = "filesystem_bytes";
pub const NETWORK_RECEIVE_BYTES_TOTAL: &str = "network_bytes_total";
pub const NETWORK_TRANSMIT_BYTES_TOTAL: &str = "network_bytes_total";
pub const WINDOWS_OS_INFO: &str = "system_os_info";
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
pub const STORAGE_DEVICE_INFO: &str = "hardware_device_info";
pub const STORAGE_NAMESPACE_CAPACITY_BYTES: &str = "hardware_storage_capacity_bytes";
pub const GPU_INFO: &str = "hardware_device_info";
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
