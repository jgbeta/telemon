use std::time::Instant;

#[cfg(target_os = "windows")]
use anyhow::Result;
use serde::Serialize;

#[cfg(target_os = "windows")]
use crate::traits::unix_timestamp_seconds;
use crate::traits::{collector_status_metrics, Collector, CollectorResult};
use telemon_core::config::WindowsInventoryConfig;
#[cfg(any(target_os = "windows", test))]
use telemon_core::metrics::model::{labels, MetricSample};
#[cfg(any(target_os = "windows", test))]
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "windows_inventory";
pub const SOURCE: &str = "windows_api";

#[derive(Debug, Clone)]
pub struct WindowsInventoryCollector {
    errors_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowsInventoryInspection {
    pub enabled: bool,
    pub supported: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<WindowsOsInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<WindowsCpuInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<WindowsComputerInspection>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowsOsInspection {
    pub version: String,
    pub build: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowsCpuInspection {
    pub model: String,
    pub architecture: String,
    pub logical_processors: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowsComputerInspection {
    pub computer_name: String,
    pub arch: String,
}

impl WindowsInventoryCollector {
    pub fn new(_config: WindowsInventoryConfig) -> Self {
        Self { errors_total: 0 }
    }

    pub fn discover_summary(config: &WindowsInventoryConfig) -> String {
        if !config.enabled {
            return "disabled".to_string();
        }
        if cfg!(target_os = "windows") {
            "available".to_string()
        } else {
            "unsupported on this OS".to_string()
        }
    }
}

pub fn inspect_hardware(config: &WindowsInventoryConfig) -> WindowsInventoryInspection {
    if !config.enabled {
        return WindowsInventoryInspection {
            enabled: false,
            supported: false,
            status: "disabled".to_string(),
            message: None,
            os: None,
            cpu: None,
            computer: None,
        };
    }

    inspect_hardware_inner(config)
}

#[cfg(not(target_os = "windows"))]
fn inspect_hardware_inner(_config: &WindowsInventoryConfig) -> WindowsInventoryInspection {
    WindowsInventoryInspection {
        enabled: true,
        supported: false,
        status: "unsupported".to_string(),
        message: Some("windows_inventory is unsupported on this OS".to_string()),
        os: None,
        cpu: None,
        computer: None,
    }
}

#[cfg(target_os = "windows")]
fn inspect_hardware_inner(_config: &WindowsInventoryConfig) -> WindowsInventoryInspection {
    match collect_inventory() {
        Ok(inventory) => WindowsInventoryInspection {
            enabled: true,
            supported: true,
            status: "available".to_string(),
            message: None,
            os: Some(inventory.os),
            cpu: Some(inventory.cpu),
            computer: Some(inventory.computer),
        },
        Err(error) => WindowsInventoryInspection {
            enabled: true,
            supported: true,
            status: "error".to_string(),
            message: Some(error.to_string()),
            os: None,
            cpu: None,
            computer: None,
        },
    }
}

impl Collector for WindowsInventoryCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    #[cfg(not(target_os = "windows"))]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        CollectorResult {
            collector: COLLECTOR_NAME,
            success: true,
            metrics: collector_status_metrics(
                COLLECTOR_NAME,
                false,
                false,
                self.errors_total,
                None,
            ),
            error_message: Some("windows_inventory is unsupported on this OS".to_string()),
            duration: started_at.elapsed(),
        }
    }

    #[cfg(target_os = "windows")]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match collect_inventory().map(inventory_metrics) {
            Ok(mut inventory_metrics) => {
                let mut metrics = collector_status_metrics(
                    COLLECTOR_NAME,
                    true,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.append(&mut inventory_metrics);
                CollectorResult::success(COLLECTOR_NAME, metrics, started_at)
            }
            Err(error) => {
                self.errors_total += 1;
                CollectorResult {
                    collector: COLLECTOR_NAME,
                    success: false,
                    metrics: collector_status_metrics(
                        COLLECTOR_NAME,
                        true,
                        false,
                        self.errors_total,
                        None,
                    ),
                    error_message: Some(error.to_string()),
                    duration: started_at.elapsed(),
                }
            }
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsInventory {
    os: WindowsOsInspection,
    cpu: WindowsCpuInspection,
    computer: WindowsComputerInspection,
}

#[cfg(any(target_os = "windows", test))]
fn inventory_metrics(inventory: WindowsInventory) -> Vec<MetricSample> {
    vec![
        os_info_metric(&inventory.os),
        cpu_info_metric(&inventory.cpu),
        computer_system_info_metric(&inventory.computer),
    ]
}

#[cfg(any(target_os = "windows", test))]
fn os_info_metric(os: &WindowsOsInspection) -> MetricSample {
    MetricSample::gauge(
        names::WINDOWS_OS_INFO,
        "Windows operating system identity information.",
        labels(&[
            ("source", SOURCE),
            ("os", "windows"),
            ("version", os.version.as_str()),
            ("build", os.build.as_str()),
            ("arch", os.arch.as_str()),
        ]),
        1.0,
    )
}

#[cfg(any(target_os = "windows", test))]
fn cpu_info_metric(cpu: &WindowsCpuInspection) -> MetricSample {
    let logical_processors = cpu.logical_processors.to_string();
    MetricSample::gauge(
        names::CPU_INFO,
        "CPU identity and topology information.",
        labels(&[
            ("source", SOURCE),
            ("model", cpu.model.as_str()),
            ("architecture", cpu.architecture.as_str()),
            ("logical_processors", logical_processors.as_str()),
        ]),
        1.0,
    )
}

#[cfg(any(target_os = "windows", test))]
fn computer_system_info_metric(computer: &WindowsComputerInspection) -> MetricSample {
    MetricSample::gauge(
        names::COMPUTER_SYSTEM_INFO,
        "Windows computer system identity information.",
        labels(&[
            ("source", SOURCE),
            ("computer_name", computer.computer_name.as_str()),
            ("arch", computer.arch.as_str()),
        ]),
        1.0,
    )
}

#[cfg(target_os = "windows")]
fn collect_inventory() -> Result<WindowsInventory> {
    let (version, build) = windows_version();
    let logical_processors = logical_processor_count();
    let cpu_model = registry_string(
        r"HARDWARE\DESCRIPTION\System\CentralProcessor\0",
        "ProcessorNameString",
    )
    .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "unknown".to_string());
    let cpu_architecture = std::env::var("PROCESSOR_ARCHITECTURE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let computer_name = computer_name()
        .unwrap_or_else(|| std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string()));

    Ok(WindowsInventory {
        os: WindowsOsInspection {
            version,
            build,
            arch: std::env::consts::ARCH.to_string(),
        },
        cpu: WindowsCpuInspection {
            model: cpu_model,
            architecture: cpu_architecture,
            logical_processors,
        },
        computer: WindowsComputerInspection {
            computer_name,
            arch: std::env::consts::ARCH.to_string(),
        },
    })
}

#[cfg(target_os = "windows")]
fn registry_string(subkey: &str, value_name: &str) -> Option<String> {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let subkey = wide_null(subkey);
    let value_name = wide_null(value_name);
    let mut value_type = 0_u32;
    let mut byte_len = 0_u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            &mut value_type,
            null_mut(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS || byte_len < 2 {
        return None;
    }

    let mut buffer = vec![0_u16; byte_len as usize / 2 + 1];
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            &mut value_type,
            buffer.as_mut_ptr().cast::<c_void>(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }

    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..end]).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn logical_processor_count() -> u32 {
    use std::mem::zeroed;
    use windows_sys::Win32::System::SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO};

    unsafe {
        let mut info: SYSTEM_INFO = zeroed();
        GetNativeSystemInfo(&mut info);
        if info.dwNumberOfProcessors == 0 {
            1
        } else {
            info.dwNumberOfProcessors
        }
    }
}

#[cfg(target_os = "windows")]
fn computer_name() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;

    let mut buffer = [0_u16; 256];
    let mut size = buffer.len() as u32;
    let ok = unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) };
    if ok == 0 || size == 0 {
        return None;
    }

    Some(
        String::from_utf16_lossy(&buffer[..size as usize])
            .trim()
            .to_string(),
    )
}

#[cfg(target_os = "windows")]
fn windows_version() -> (String, String) {
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    struct OsVersionInfoW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(version_information: *mut OsVersionInfoW) -> i32;
    }

    unsafe {
        let mut info: OsVersionInfoW = zeroed();
        info.dw_os_version_info_size = size_of::<OsVersionInfoW>() as u32;
        if RtlGetVersion(&mut info) != 0 {
            return ("unknown".to_string(), "unknown".to_string());
        }

        (
            format!("{}.{}", info.dw_major_version, info.dw_minor_version),
            info.dw_build_number.to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_metric(metrics: &[MetricSample], name: &str) -> bool {
        metrics.iter().any(|metric| metric.name == name)
    }

    #[test]
    #[cfg(any(target_os = "windows", test))]
    fn inventory_metrics_emit_static_info_families() {
        let metrics = inventory_metrics(WindowsInventory {
            os: WindowsOsInspection {
                version: "10.0".to_string(),
                build: "22631".to_string(),
                arch: "x86_64".to_string(),
            },
            cpu: WindowsCpuInspection {
                model: "Example CPU".to_string(),
                architecture: "AMD64".to_string(),
                logical_processors: 16,
            },
            computer: WindowsComputerInspection {
                computer_name: "example-pc".to_string(),
                arch: "x86_64".to_string(),
            },
        });

        assert!(has_metric(&metrics, names::WINDOWS_OS_INFO));
        assert!(has_metric(&metrics, names::CPU_INFO));
        assert!(has_metric(&metrics, names::COMPUTER_SYSTEM_INFO));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_reports_unsupported_without_failing_process() {
        let mut collector =
            WindowsInventoryCollector::new(WindowsInventoryConfig { enabled: true });

        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::COLLECTOR_SUPPORTED
                && metric.labels.get("collector").map(String::as_str) == Some(COLLECTOR_NAME)
                && metric.value == 0.0
        }));
    }
}
