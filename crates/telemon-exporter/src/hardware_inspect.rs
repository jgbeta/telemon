use serde::Serialize;
use telemon_collectors::gpu::nvidia::collector::{
    inspect_hardware as inspect_nvidia_nvml, NvidiaNvmlInspection,
};
use telemon_collectors::temperature::linux_hwmon::{
    inspect_hardware as inspect_linux_hwmon, LinuxHwmonInspection,
};
use telemon_core::config::AppConfig;

#[derive(Debug, Serialize)]
struct HardwareInspection {
    linux_hwmon: LinuxHwmonInspection,
    nvidia_nvml: NvidiaNvmlInspection,
}

pub fn inspect_hardware_json(config: &AppConfig) -> anyhow::Result<String> {
    let inspection = HardwareInspection {
        linux_hwmon: inspect_linux_hwmon(&config.collectors.linux_hwmon)?,
        nvidia_nvml: inspect_nvidia_nvml(&config.collectors.nvidia_nvml),
    };

    Ok(serde_json::to_string_pretty(&inspection)?)
}
