use serde::Serialize;
use telemon_collectors::gpu::nvidia::collector::{
    inspect_hardware as inspect_nvidia_nvml, NvidiaNvmlInspection,
};
use telemon_collectors::temperature::linux_hwmon::{
    inspect_hardware as inspect_linux_hwmon, LinuxHwmonInspection,
};
use telemon_collectors::windows::baseline::{
    inspect_hardware as inspect_windows_baseline, WindowsBaselineInspection,
};
use telemon_collectors::windows::inventory::{
    inspect_hardware as inspect_windows_inventory, WindowsInventoryInspection,
};
use telemon_core::config::AppConfig;

#[derive(Debug, Serialize)]
struct HardwareInspection {
    linux_hwmon: LinuxHwmonInspection,
    nvidia_nvml: NvidiaNvmlInspection,
    windows_baseline: WindowsBaselineInspection,
    windows_inventory: WindowsInventoryInspection,
}

pub fn inspect_hardware_json(config: &AppConfig) -> anyhow::Result<String> {
    let inspection = HardwareInspection {
        linux_hwmon: inspect_linux_hwmon(&config.collectors.linux_hwmon)?,
        nvidia_nvml: inspect_nvidia_nvml(&config.collectors.nvidia_nvml),
        windows_baseline: inspect_windows_baseline(&config.collectors.windows_baseline),
        windows_inventory: inspect_windows_inventory(&config.collectors.windows_inventory),
    };

    Ok(serde_json::to_string_pretty(&inspection)?)
}
