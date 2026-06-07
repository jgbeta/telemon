use std::time::Instant;

#[cfg(target_os = "windows")]
use anyhow::{anyhow, Result};
use serde::Serialize;

#[cfg(target_os = "windows")]
use crate::traits::unix_timestamp_seconds;
use crate::traits::{collector_status_metrics, Collector, CollectorResult};
use telemon_core::config::WindowsBaselineConfig;
#[cfg(any(target_os = "windows", test))]
use telemon_core::metrics::model::{labels, MetricSample};
#[cfg(any(target_os = "windows", test))]
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "windows_baseline";
pub const SOURCE: &str = "windows_api";

#[derive(Debug, Clone)]
pub struct WindowsBaselineCollector {
    config: WindowsBaselineConfig,
    #[cfg(target_os = "windows")]
    previous_cpu: Option<CpuTimes>,
    errors_total: u64,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy)]
struct CpuTimes {
    idle: u64,
    kernel: u64,
    user: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowsBaselineInspection {
    pub enabled: bool,
    pub supported: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl WindowsBaselineCollector {
    pub fn new(config: WindowsBaselineConfig) -> Self {
        Self {
            config,
            #[cfg(target_os = "windows")]
            previous_cpu: None,
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &WindowsBaselineConfig) -> String {
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

pub fn inspect_hardware(config: &WindowsBaselineConfig) -> WindowsBaselineInspection {
    if !config.enabled {
        return WindowsBaselineInspection {
            enabled: false,
            supported: false,
            status: "disabled".to_string(),
            message: None,
        };
    }

    if cfg!(target_os = "windows") {
        WindowsBaselineInspection {
            enabled: true,
            supported: true,
            status: "available".to_string(),
            message: None,
        }
    } else {
        WindowsBaselineInspection {
            enabled: true,
            supported: false,
            status: "unsupported".to_string(),
            message: Some("windows_baseline is unsupported on this OS".to_string()),
        }
    }
}

impl Collector for WindowsBaselineCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    #[cfg(not(target_os = "windows"))]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        let _ = &self.config;
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
            error_message: Some("windows_baseline is unsupported on this OS".to_string()),
            duration: started_at.elapsed(),
        }
    }

    #[cfg(target_os = "windows")]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match collect_windows_metrics(&self.config, &mut self.previous_cpu) {
            Ok(mut baseline_metrics) => {
                let mut metrics = collector_status_metrics(
                    COLLECTOR_NAME,
                    true,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.append(&mut baseline_metrics);
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

#[cfg(target_os = "windows")]
fn collect_windows_metrics(
    config: &WindowsBaselineConfig,
    previous_cpu: &mut Option<CpuTimes>,
) -> Result<Vec<MetricSample>> {
    let mut metrics = Vec::new();
    metrics.push(uptime_metric(uptime_seconds()));

    if let Some(cpu_usage) = cpu_usage_ratio(previous_cpu)? {
        metrics.push(cpu_usage_metric(cpu_usage));
    }

    let memory = memory_status()?;
    metrics.extend(memory_metrics(memory));
    metrics.extend(filesystem_metrics(config)?);
    metrics.extend(network_metrics(config)?);

    Ok(metrics)
}

#[cfg(target_os = "windows")]
fn uptime_metric(seconds: f64) -> MetricSample {
    MetricSample::gauge(
        names::UPTIME_SECONDS,
        "System uptime in seconds.",
        labels(&[("source", SOURCE)]),
        seconds,
    )
}

#[cfg(target_os = "windows")]
fn cpu_usage_metric(value: f64) -> MetricSample {
    MetricSample::gauge(
        names::CPU_USAGE_RATIO,
        "Total system CPU usage as a ratio from 0 to 1.",
        labels(&[("component", "cpu"), ("source", SOURCE)]),
        value.clamp(0.0, 1.0),
    )
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct MemoryStatus {
    total_bytes: u64,
    available_bytes: u64,
}

#[cfg(any(target_os = "windows", test))]
fn memory_metrics(memory: MemoryStatus) -> Vec<MetricSample> {
    let used = memory.total_bytes.saturating_sub(memory.available_bytes);
    vec![
        MetricSample::gauge(
            names::MEMORY_TOTAL_BYTES,
            "Physical memory in decimal megabytes by kind and state.",
            labels(&[("source", SOURCE), ("kind", "ram"), ("state", "total")]),
            bytes_to_mb(memory.total_bytes),
        ),
        MetricSample::gauge(
            names::MEMORY_AVAILABLE_BYTES,
            "Physical memory in decimal megabytes by kind and state.",
            labels(&[("source", SOURCE), ("kind", "ram"), ("state", "available")]),
            bytes_to_mb(memory.available_bytes),
        ),
        MetricSample::gauge(
            names::MEMORY_USED_BYTES,
            "Physical memory in decimal megabytes by kind and state.",
            labels(&[("source", SOURCE), ("kind", "ram"), ("state", "used")]),
            bytes_to_mb(used),
        ),
    ]
}

#[cfg(any(target_os = "windows", test))]
fn bytes_to_mb(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, PartialEq)]
struct FilesystemStatus {
    volume: String,
    drive_type: &'static str,
    size_bytes: u64,
    free_bytes: u64,
    available_bytes: u64,
}

#[cfg(target_os = "windows")]
fn filesystem_status_to_metrics(filesystem: &FilesystemStatus) -> Vec<MetricSample> {
    let base_labels = labels(&[
        ("source", SOURCE),
        ("volume", filesystem.volume.as_str()),
        ("drive_type", filesystem.drive_type),
    ]);

    vec![
        filesystem_bytes_metric(
            names::FILESYSTEM_SIZE_BYTES,
            base_labels.clone(),
            "size",
            filesystem.size_bytes,
        ),
        filesystem_bytes_metric(
            names::FILESYSTEM_FREE_BYTES,
            base_labels.clone(),
            "free",
            filesystem.free_bytes,
        ),
        filesystem_bytes_metric(
            names::FILESYSTEM_AVAILABLE_BYTES,
            base_labels,
            "available",
            filesystem.available_bytes,
        ),
    ]
}

#[cfg(target_os = "windows")]
fn filesystem_bytes_metric(
    name: &str,
    mut metric_labels: std::collections::BTreeMap<String, String>,
    state: &str,
    value: u64,
) -> MetricSample {
    metric_labels.insert("state".to_string(), state.to_string());
    MetricSample::gauge(
        name,
        "Filesystem space in decimal megabytes by state.",
        metric_labels,
        bytes_to_mb(value),
    )
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, PartialEq)]
struct NetworkStatus {
    if_index: u32,
    interface: String,
    receive_bytes: u64,
    transmit_bytes: u64,
}

#[cfg(target_os = "windows")]
fn network_status_to_metrics(network: &NetworkStatus) -> Vec<MetricSample> {
    let if_index = network.if_index.to_string();
    let metric_labels = labels(&[
        ("source", SOURCE),
        ("if_index", if_index.as_str()),
        ("interface", network.interface.as_str()),
    ]);

    vec![
        network_bytes_metric(
            names::NETWORK_RECEIVE_BYTES_TOTAL,
            metric_labels.clone(),
            "receive",
            network.receive_bytes,
        ),
        network_bytes_metric(
            names::NETWORK_TRANSMIT_BYTES_TOTAL,
            metric_labels,
            "transmit",
            network.transmit_bytes,
        ),
    ]
}

#[cfg(target_os = "windows")]
fn network_bytes_metric(
    name: &str,
    mut metric_labels: std::collections::BTreeMap<String, String>,
    direction: &str,
    value: u64,
) -> MetricSample {
    metric_labels.insert("direction".to_string(), direction.to_string());
    MetricSample::counter(
        name,
        "Total network bytes by direction.",
        metric_labels,
        value as f64,
    )
}

#[cfg(any(target_os = "windows", test))]
fn interface_allowed(config: &WindowsBaselineConfig, name: &str) -> bool {
    let normalized_name = name.to_ascii_lowercase();
    let allowlist = normalized_patterns(&config.network_interface_allowlist);
    let denylist = normalized_patterns(&config.network_interface_denylist);

    if !allowlist.is_empty()
        && !allowlist
            .iter()
            .any(|pattern| normalized_name.contains(pattern))
    {
        return false;
    }

    !denylist
        .iter()
        .any(|pattern| normalized_name.contains(pattern))
}

#[cfg(any(target_os = "windows", test))]
fn normalized_patterns(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

#[cfg(target_os = "windows")]
fn uptime_seconds() -> f64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;

    unsafe { GetTickCount64() as f64 / 1_000.0 }
}

#[cfg(target_os = "windows")]
fn cpu_usage_ratio(previous: &mut Option<CpuTimes>) -> Result<Option<f64>> {
    let current = read_cpu_times()?;
    let Some(previous_times) = previous.replace(current) else {
        return Ok(None);
    };

    let idle_delta = current.idle.saturating_sub(previous_times.idle);
    let kernel_delta = current.kernel.saturating_sub(previous_times.kernel);
    let user_delta = current.user.saturating_sub(previous_times.user);
    let total_delta = kernel_delta.saturating_add(user_delta);
    if total_delta == 0 || idle_delta > total_delta {
        return Ok(None);
    }

    let busy_delta = total_delta - idle_delta;
    Ok(Some(
        (busy_delta as f64 / total_delta as f64).clamp(0.0, 1.0),
    ))
}

#[cfg(target_os = "windows")]
fn read_cpu_times() -> Result<CpuTimes> {
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    unsafe {
        let mut idle: FILETIME = zeroed();
        let mut kernel: FILETIME = zeroed();
        let mut user: FILETIME = zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return Err(anyhow!("GetSystemTimes failed"));
        }

        Ok(CpuTimes {
            idle: filetime_to_u64(idle),
            kernel: filetime_to_u64(kernel),
            user: filetime_to_u64(user),
        })
    }
}

#[cfg(target_os = "windows")]
fn filetime_to_u64(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

#[cfg(target_os = "windows")]
fn memory_status() -> Result<MemoryStatus> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    unsafe {
        let mut status: MEMORYSTATUSEX = zeroed();
        status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return Err(anyhow!("GlobalMemoryStatusEx failed"));
        }

        Ok(MemoryStatus {
            total_bytes: status.ullTotalPhys,
            available_bytes: status.ullAvailPhys,
        })
    }
}

#[cfg(target_os = "windows")]
fn filesystem_metrics(config: &WindowsBaselineConfig) -> Result<Vec<MetricSample>> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    };
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE,
    };

    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(anyhow!("GetLogicalDrives failed"));
    }

    let mut metrics = Vec::new();
    for index in 0..26 {
        if mask & (1 << index) == 0 {
            continue;
        }

        let letter = (b'A' + index as u8) as char;
        let volume = format!("{letter}:");
        let root = [letter as u16, ':' as u16, '\\' as u16, 0];
        let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
        let Some(drive_type_label) = drive_type_label(drive_type) else {
            continue;
        };

        if drive_type == DRIVE_REMOVABLE && !config.include_removable_drives {
            continue;
        }
        if drive_type == DRIVE_REMOTE && !config.include_remote_drives {
            continue;
        }
        if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE && drive_type != DRIVE_REMOTE
        {
            continue;
        }

        let mut available_bytes = 0_u64;
        let mut size_bytes = 0_u64;
        let mut free_bytes = 0_u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                root.as_ptr(),
                &mut available_bytes,
                &mut size_bytes,
                &mut free_bytes,
            )
        };
        if ok == 0 {
            continue;
        }

        metrics.extend(filesystem_status_to_metrics(&FilesystemStatus {
            volume,
            drive_type: drive_type_label,
            size_bytes,
            free_bytes,
            available_bytes,
        }));
    }

    Ok(metrics)
}

#[cfg(target_os = "windows")]
fn drive_type_label(value: u32) -> Option<&'static str> {
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
        DRIVE_UNKNOWN,
    };

    match value {
        DRIVE_FIXED => Some("fixed"),
        DRIVE_REMOVABLE => Some("removable"),
        DRIVE_REMOTE => Some("remote"),
        DRIVE_RAMDISK => Some("ramdisk"),
        DRIVE_CDROM | DRIVE_NO_ROOT_DIR | DRIVE_UNKNOWN => None,
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn network_metrics(config: &WindowsBaselineConfig) -> Result<Vec<MetricSample>> {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use std::slice;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = null_mut();
        let result = GetIfTable2(&mut table);
        if result != 0 {
            return Err(anyhow!("GetIfTable2 failed with code {result}"));
        }
        if table.is_null() {
            return Ok(Vec::new());
        }

        let rows = slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let mut metrics = Vec::new();
        for row in rows {
            let mut interface = wide_array_to_string(&row.Alias);
            if interface.is_empty() {
                interface = format!("if{}", row.InterfaceIndex);
            }
            if !interface_allowed(config, &interface) {
                continue;
            }

            metrics.extend(network_status_to_metrics(&NetworkStatus {
                if_index: row.InterfaceIndex,
                interface,
                receive_bytes: row.InOctets,
                transmit_bytes: row.OutOctets,
            }));
        }

        FreeMibTable(table.cast::<c_void>());
        Ok(metrics)
    }
}

#[cfg(target_os = "windows")]
fn wide_array_to_string(values: &[u16]) -> String {
    let end = values
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(values.len());
    String::from_utf16_lossy(&values[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_value(metrics: &[MetricSample], name: &str, labels: &[(&str, &str)]) -> Option<f64> {
        metrics
            .iter()
            .find(|metric| {
                metric.name == name
                    && labels.iter().all(|label| {
                        metric
                            .labels
                            .get(label.0)
                            .map(|actual| actual == label.1)
                            .unwrap_or(false)
                    })
            })
            .map(|metric| metric.value)
    }

    #[test]
    #[cfg(any(target_os = "windows", test))]
    fn memory_metrics_emit_total_available_and_used_bytes() {
        let metrics = memory_metrics(MemoryStatus {
            total_bytes: 16_000_000,
            available_bytes: 6_000_000,
        });

        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_TOTAL_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "total")]
            ),
            Some(16.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_AVAILABLE_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "available")]
            ),
            Some(6.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_USED_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "used")]
            ),
            Some(10.0)
        );
    }

    #[test]
    fn network_interface_filters_use_case_insensitive_substrings() {
        let config = WindowsBaselineConfig {
            network_interface_allowlist: vec!["ether".to_string()],
            network_interface_denylist: vec!["test".to_string()],
            ..WindowsBaselineConfig::default()
        };

        assert!(interface_allowed(&config, "Ethernet 2"));
        assert!(!interface_allowed(&config, "Wi-Fi"));
        assert!(!interface_allowed(&config, "Ethernet Test"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_reports_unsupported_without_failing_process() {
        let mut collector = WindowsBaselineCollector::new(WindowsBaselineConfig {
            enabled: true,
            ..WindowsBaselineConfig::default()
        });

        let result = collector.collect();

        assert!(result.success);
        assert_eq!(
            metric_value(
                &result.metrics,
                names::COLLECTOR_SUPPORTED,
                &[("collector", COLLECTOR_NAME)]
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(
                &result.metrics,
                names::COLLECTOR_UP,
                &[("collector", COLLECTOR_NAME)]
            ),
            Some(0.0)
        );
    }
}
