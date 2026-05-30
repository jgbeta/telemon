use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::linux::gpu_discovery::{discover_drm_gpus, LinuxGpuDevice};
use crate::traits::{
    collector_health_metrics, collector_status_metrics, unix_timestamp_seconds, Collector,
    CollectorResult,
};
use telemon_core::config::LinuxDrmConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "linux_drm";
pub const SOURCE: &str = "linux_drm";

#[derive(Debug, Clone)]
pub struct LinuxDrmCollector {
    config: LinuxDrmConfig,
    errors_total: u64,
    previous_energy_uj: BTreeMap<String, (u64, Instant)>,
    previous_engine_ns: BTreeMap<String, (u64, Instant)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxDrmInspection {
    pub drm_root: String,
    pub root_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_error: Option<String>,
    pub devices: Vec<LinuxDrmDeviceInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxDrmDeviceInspection {
    pub node_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub device_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_bdf: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    pub supported: bool,
    pub hwmon_dirs: Vec<String>,
}

impl LinuxDrmCollector {
    pub fn new(config: LinuxDrmConfig) -> Self {
        Self {
            config,
            errors_total: 0,
            previous_energy_uj: BTreeMap::new(),
            previous_engine_ns: BTreeMap::new(),
        }
    }

    pub fn discover_summary(config: &LinuxDrmConfig) -> String {
        let inspection = inspect_hardware(config);
        if !inspection.root_exists {
            return "unavailable, root missing".to_string();
        }
        if let Some(error) = inspection.root_error {
            return format!("error: {error}");
        }
        let supported = inspection
            .devices
            .iter()
            .filter(|device| device.supported)
            .count();
        format!(
            "available, devices={}, supported_devices={}",
            inspection.devices.len(),
            supported
        )
    }
}

impl Collector for LinuxDrmCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        match discover_drm_gpus(&self.config.drm_root) {
            Ok(devices) => {
                let supported_devices = devices
                    .iter()
                    .filter(|device| supported_driver(device.backend_driver()))
                    .collect::<Vec<_>>();
                let supported = self.config.drm_root.exists() && !supported_devices.is_empty();
                let mut metrics = if supported {
                    collector_health_metrics(
                        COLLECTOR_NAME,
                        true,
                        self.errors_total,
                        Some(unix_timestamp_seconds()),
                    )
                } else {
                    collector_status_metrics(COLLECTOR_NAME, false, false, self.errors_total, None)
                };

                for (gpu_index, device) in supported_devices.into_iter().enumerate() {
                    metrics.extend(self.device_metrics(device, gpu_index));
                }

                CollectorResult::success(COLLECTOR_NAME, metrics, started_at)
            }
            Err(error) => {
                self.errors_total += 1;
                CollectorResult::failure(
                    COLLECTOR_NAME,
                    error.to_string(),
                    self.errors_total,
                    started_at,
                )
            }
        }
    }
}

pub fn inspect_hardware(config: &LinuxDrmConfig) -> LinuxDrmInspection {
    let mut inspection = LinuxDrmInspection {
        drm_root: config.drm_root.display().to_string(),
        root_exists: config.drm_root.exists(),
        root_error: None,
        devices: Vec::new(),
    };
    if !inspection.root_exists {
        return inspection;
    }

    match discover_drm_gpus(&config.drm_root) {
        Ok(devices) => {
            inspection.devices = devices
                .into_iter()
                .map(|device| {
                    let supported = supported_driver(device.backend_driver());
                    let hwmon_dirs = hwmon_dirs(&device.device_path)
                        .into_iter()
                        .map(|path| path.display().to_string())
                        .collect();
                    LinuxDrmDeviceInspection {
                        node_name: device.node_name,
                        render_node: device.render_node,
                        card: device.card,
                        device_path: device.device_path.display().to_string(),
                        pci_bdf: device.pci_bdf,
                        vendor_id: device.vendor_id,
                        device_id: device.device_id,
                        supported,
                        hwmon_dirs,
                        driver: device.driver,
                    }
                })
                .collect();
        }
        Err(error) => inspection.root_error = Some(error.to_string()),
    }

    inspection
}

impl LinuxDrmCollector {
    fn device_metrics(&mut self, device: &LinuxGpuDevice, gpu_index: usize) -> Vec<MetricSample> {
        let mut metrics = Vec::new();
        metrics.push(device_info_metric(device, gpu_index));
        let driver = device.backend_driver();

        if self.config.include_hwmon {
            for hwmon in hwmon_dirs(&device.device_path) {
                metrics.extend(hwmon_metrics(
                    &mut self.previous_energy_uj,
                    device,
                    gpu_index,
                    driver,
                    &hwmon,
                ));
            }
        }

        metrics.extend(driver_clock_metrics(device, gpu_index, driver));
        metrics.extend(driver_throttle_metrics(device, gpu_index, driver));

        if self.config.include_fdinfo {
            if let Some(pid) = self.config.target_pid {
                metrics.extend(fdinfo_metrics(
                    &mut self.previous_engine_ns,
                    device,
                    gpu_index,
                    driver,
                    &self.config.proc_root,
                    pid,
                ));
            }
        }

        metrics
    }
}

fn supported_driver(driver: &str) -> bool {
    matches!(
        driver,
        "i915" | "xe" | "panfrost" | "panthor" | "msm_dpu" | "msm_drm"
    )
}

fn device_info_metric(device: &LinuxGpuDevice, gpu_index: usize) -> MetricSample {
    let mut metric_labels = base_labels(device, gpu_index, device.backend_driver());
    metric_labels.insert(
        "vendor".to_string(),
        vendor_label(device.vendor_id.as_deref()).to_string(),
    );
    if let Some(value) = &device.vendor_id {
        metric_labels.insert("pci_vendor_id".to_string(), value.clone());
    }
    if let Some(value) = &device.device_id {
        metric_labels.insert("pci_device_id".to_string(), value.clone());
    }
    if let Some(value) = &device.pci_bdf {
        metric_labels.insert("pci_bdf".to_string(), value.clone());
    }
    if let Some(value) = &device.render_node {
        metric_labels.insert("render_node".to_string(), value.clone());
    }
    if let Some(value) = &device.card {
        metric_labels.insert("card".to_string(), value.clone());
    }

    MetricSample::gauge(
        names::HARDWARE_DEVICE_INFO,
        "Hardware device identity information.",
        metric_labels,
        1.0,
    )
}

fn base_labels(
    device: &LinuxGpuDevice,
    gpu_index: usize,
    driver: &str,
) -> BTreeMap<String, String> {
    let device_id = device.stable_id();
    let gpu_index = gpu_index.to_string();
    labels(&[
        ("component", "gpu"),
        ("device_id", device_id.as_str()),
        ("gpu_index", gpu_index.as_str()),
        ("source", SOURCE),
        ("source_driver", driver),
    ])
}

fn vendor_label(vendor_id: Option<&str>) -> &'static str {
    match vendor_id {
        Some("0x8086") => "intel",
        Some("0x1002") => "amd",
        Some("0x10de") => "nvidia",
        _ => "unknown",
    }
}

fn hwmon_dirs(device_path: &Path) -> Vec<PathBuf> {
    let root = device_path.join("hwmon");
    let mut dirs = fs::read_dir(root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("hwmon"))
        })
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

fn hwmon_metrics(
    previous_energy_uj: &mut BTreeMap<String, (u64, Instant)>,
    device: &LinuxGpuDevice,
    gpu_index: usize,
    driver: &str,
    hwmon: &Path,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let base = base_labels(device, gpu_index, driver);

    match driver {
        "i915" => {
            push_temp(
                &mut metrics,
                &base,
                hwmon.join("temp1_input"),
                "gpu_edge_temp",
                "edge",
            );
            push_voltage(
                &mut metrics,
                &base,
                hwmon.join("in0_input"),
                "gpu_voltage",
                "core",
            );
            push_fan(
                &mut metrics,
                &base,
                hwmon.join("fan1_input"),
                "gpu_fan_rpm",
                "fan1",
            );
            push_power_limit(
                &mut metrics,
                &base,
                hwmon.join("power1_max"),
                "gpu_power_limit",
                "current",
            );
            push_power(
                &mut metrics,
                &base,
                hwmon.join("power1_input"),
                "gpu_power",
                "current",
            );
            push_power(
                &mut metrics,
                &base,
                hwmon.join("power1_average"),
                "gpu_power",
                "average",
            );
            push_energy_power(
                previous_energy_uj,
                &mut metrics,
                &base,
                hwmon.join("energy1_input"),
                "gpu_power",
                "energy",
            );
        }
        "xe" => {
            push_temp(
                &mut metrics,
                &base,
                hwmon.join("temp2_input"),
                "gpu_edge_temp",
                "edge",
            );
            push_temp(
                &mut metrics,
                &base,
                hwmon.join("temp3_input"),
                "gpu_memory_temp",
                "memory",
            );
            push_voltage(
                &mut metrics,
                &base,
                hwmon.join("in1_input"),
                "gpu_voltage",
                "core",
            );
            push_fan(
                &mut metrics,
                &base,
                hwmon.join("fan1_input"),
                "gpu_fan_rpm",
                "fan1",
            );
            push_power_limit(
                &mut metrics,
                &base,
                hwmon.join("power2_max"),
                "gpu_power_limit",
                "current",
            );
            push_power(
                &mut metrics,
                &base,
                hwmon.join("power2_input"),
                "gpu_power",
                "current",
            );
            push_power(
                &mut metrics,
                &base,
                hwmon.join("power2_average"),
                "gpu_power",
                "average",
            );
            push_energy_power(
                previous_energy_uj,
                &mut metrics,
                &base,
                hwmon.join("energy2_input"),
                "gpu_power",
                "energy",
            );
        }
        _ => {
            if let Some(path) = lowest_matching_file(hwmon, "temp", "_input") {
                push_temp(&mut metrics, &base, path, "gpu_edge_temp", "edge");
            }
            if let Some(path) = lowest_matching_file(hwmon, "in", "_input") {
                push_voltage(&mut metrics, &base, path, "gpu_voltage", "core");
            }
            if let Some(path) = lowest_matching_file(hwmon, "fan", "_input") {
                push_fan(&mut metrics, &base, path, "gpu_fan_rpm", "fan");
            }
            if let Some(path) = lowest_matching_file(hwmon, "power", "_input") {
                push_power(&mut metrics, &base, path, "gpu_power", "current");
            }
            if let Some(path) = lowest_matching_file(hwmon, "power", "_average") {
                push_power(&mut metrics, &base, path, "gpu_power", "average");
            }
            if let Some(path) = lowest_matching_file(hwmon, "energy", "_input") {
                push_energy_power(
                    previous_energy_uj,
                    &mut metrics,
                    &base,
                    path,
                    "gpu_power",
                    "energy",
                );
            }
        }
    }

    metrics
}

fn push_temp(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    instance: &str,
) {
    if let Some(value) = read_f64(&path)
        .map(|value| value / 1_000.0)
        .filter(valid_temperature)
    {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("sensor_instance".to_string(), instance.to_string());
        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_CELSIUS,
            "Hardware temperature reading in degrees Celsius.",
            metric_labels,
            value,
        ));
    }
}

fn push_voltage(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    instance: &str,
) {
    if let Some(value) = read_f64(&path)
        .map(|value| value / 1_000.0)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("sensor_instance".to_string(), instance.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_VOLTAGE_VOLTS,
            "Hardware voltage in volts.",
            metric_labels,
            value,
        ));
    }
}

fn push_fan(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    instance: &str,
) {
    if let Some(value) = read_f64(&path).filter(|value| value.is_finite() && *value >= 0.0) {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("sensor_instance".to_string(), instance.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_FAN_SPEED_RPM,
            "Hardware fan or pump speed in revolutions per minute.",
            metric_labels,
            value,
        ));
    }
}

fn push_power(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    instance: &str,
) {
    if let Some(value) = read_f64(&path)
        .map(|value| value / 1_000_000.0)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("sensor_instance".to_string(), instance.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_POWER_WATTS,
            "Hardware power in watts.",
            metric_labels,
            value,
        ));
    }
}

fn push_power_limit(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    limit: &str,
) {
    if let Some(value) = read_f64(&path)
        .map(|value| value / 1_000_000.0)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("limit".to_string(), limit.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_POWER_LIMIT_WATTS,
            "Hardware power limit in watts.",
            metric_labels,
            value,
        ));
    }
}

fn push_energy_power(
    previous_energy_uj: &mut BTreeMap<String, (u64, Instant)>,
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    instance: &str,
) {
    let Some(current) = read_u64(path.clone()) else {
        return;
    };
    let now = Instant::now();
    let key = path.display().to_string();
    let value = previous_energy_uj
        .get(&key)
        .and_then(|(previous, previous_at)| {
            let elapsed = now.duration_since(*previous_at).as_secs_f64();
            if current < *previous || elapsed <= 0.0 {
                return None;
            }
            Some((current - *previous) as f64 / elapsed / 1_000_000.0)
        });
    previous_energy_uj.insert(key, (current, now));

    if let Some(value) = value.filter(|value| value.is_finite() && *value >= 0.0) {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("sensor_instance".to_string(), instance.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_POWER_WATTS,
            "Hardware power in watts.",
            metric_labels,
            value,
        ));
    }
}

fn driver_clock_metrics(
    device: &LinuxGpuDevice,
    gpu_index: usize,
    driver: &str,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let base = base_labels(device, gpu_index, driver);
    match driver {
        "i915" => {
            push_clock_mhz(
                &mut metrics,
                &base,
                device
                    .device_path
                    .join("drm")
                    .join(device.card.as_deref().unwrap_or("card0"))
                    .join("gt_act_freq_mhz"),
                "gpu_core_clock",
                "graphics",
            );
        }
        "xe" => {
            for path in xe_clock_paths(&device.device_path) {
                push_clock_mhz(&mut metrics, &base, path, "gpu_core_clock", "graphics");
            }
        }
        _ => {}
    }
    metrics
}

fn push_clock_mhz(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    path: PathBuf,
    sensor: &str,
    clock: &str,
) {
    if let Some(value) = read_f64(&path)
        .map(|mhz| mhz * 1_000_000.0)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), sensor.to_string());
        metric_labels.insert("clock".to_string(), clock.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_CLOCK_HERTZ,
            "Hardware clock speed in hertz.",
            metric_labels,
            value,
        ));
    }
}

fn xe_clock_paths(device_path: &Path) -> Vec<PathBuf> {
    let tile0 = device_path.join("tile0");
    let mut paths = fs::read_dir(tile0)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gt"))
        })
        .map(|path| path.join("freq0/act_freq"))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn driver_throttle_metrics(
    device: &LinuxGpuDevice,
    gpu_index: usize,
    driver: &str,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let base = base_labels(device, gpu_index, driver);
    match driver {
        "i915" => push_throttle_reason_files(
            &mut metrics,
            &base,
            &device
                .device_path
                .join("drm")
                .join(device.card.as_deref().unwrap_or("card0"))
                .join("gt/gt0"),
        ),
        "xe" => push_throttle_reason_files(
            &mut metrics,
            &base,
            &device.device_path.join("tile0/gt0/freq0/throttle"),
        ),
        _ => {}
    }
    metrics
}

fn push_throttle_reason_files(
    metrics: &mut Vec<MetricSample>,
    base: &BTreeMap<String, String>,
    root: &Path,
) {
    for (reason, files) in [
        ("power", ["reason_pl1", "reason_pl2"].as_slice()),
        ("current", ["reason_pl4", "reason_vr_tdc"].as_slice()),
        (
            "thermal",
            [
                "reason_prochot",
                "reason_ratl",
                "reason_thermal",
                "reason_vr_thermalert",
            ]
            .as_slice(),
        ),
    ] {
        let active = files
            .iter()
            .any(|file| read_u64(root.join(file)).unwrap_or(0) > 0);
        let mut metric_labels = base.clone();
        metric_labels.insert("sensor".to_string(), "gpu_throttle".to_string());
        metric_labels.insert("state".to_string(), reason.to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_STATE,
            "Hardware numeric state value.",
            metric_labels,
            if active { 1.0 } else { 0.0 },
        ));
    }
}

fn fdinfo_metrics(
    previous_engine_ns: &mut BTreeMap<String, (u64, Instant)>,
    device: &LinuxGpuDevice,
    gpu_index: usize,
    driver: &str,
    proc_root: &Path,
    pid: u32,
) -> Vec<MetricSample> {
    let fdinfo_root = proc_root.join(pid.to_string()).join("fdinfo");
    let mut metrics = Vec::new();
    let mut seen_clients = BTreeSet::new();
    let mut engine_ns: u64 = 0;
    let mut memory_bytes: BTreeMap<String, u64> = BTreeMap::new();

    let entries = match fs::read_dir(&fdinfo_root) {
        Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
        Err(_) => return metrics,
    };

    for entry in entries {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let parsed = parse_fdinfo(&text);
        if parsed.get("drm-driver").map(String::as_str) != Some(driver) {
            continue;
        }
        if let (Some(expected), Some(actual)) = (
            device.pci_bdf.as_deref(),
            parsed.get("drm-pdev").map(String::as_str),
        ) {
            if actual != expected {
                continue;
            }
        }
        if let Some(client_id) = parsed.get("drm-client-id") {
            if !seen_clients.insert(client_id.clone()) {
                continue;
            }
        }

        for (key, value) in &parsed {
            if key.starts_with("drm-engine-") || key.starts_with("drm-cycles-") {
                engine_ns = engine_ns.saturating_add(value_number(value));
            }
            if key.starts_with("drm-resident-") {
                let memory = if key.contains("local") || key.contains("vram") {
                    "vram"
                } else {
                    "system"
                };
                *memory_bytes.entry(memory.to_string()).or_default() += parse_bytes(value);
            }
        }
    }

    let base = base_labels(device, gpu_index, driver);
    if engine_ns > 0 {
        let now = Instant::now();
        let key = format!("{}:{pid}:{driver}:engine", device.stable_id());
        let value = previous_engine_ns
            .get(&key)
            .and_then(|(previous, previous_at)| {
                let elapsed_ns = now.duration_since(*previous_at).as_nanos() as f64;
                if engine_ns < *previous || elapsed_ns <= 0.0 {
                    return None;
                }
                Some(((engine_ns - *previous) as f64 / elapsed_ns).clamp(0.0, 1.0))
            });
        previous_engine_ns.insert(key, (engine_ns, now));
        if let Some(value) = value {
            let mut metric_labels = base.clone();
            metric_labels.insert("sensor".to_string(), "gpu_fdinfo_busy".to_string());
            metric_labels.insert("engine".to_string(), "graphics".to_string());
            metrics.push(MetricSample::gauge(
                names::HARDWARE_UTILIZATION_RATIO,
                "Hardware utilization as a ratio from 0 to 1.",
                metric_labels,
                value,
            ));
        }
    }

    for (memory, bytes) in memory_bytes {
        let mut metric_labels = base.clone();
        metric_labels.insert("memory".to_string(), memory);
        metric_labels.insert("state".to_string(), "used".to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_MEMORY_BYTES,
            "Hardware memory bytes by state.",
            metric_labels,
            bytes as f64,
        ));
    }

    metrics
}

fn parse_fdinfo(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn value_number(value: &str) -> u64 {
    value
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn parse_bytes(value: &str) -> u64 {
    let mut parts = value.split_whitespace();
    let Some(number) = parts.next().and_then(|value| value.parse::<f64>().ok()) else {
        return 0;
    };
    let multiplier = match parts.next().unwrap_or("bytes") {
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (number * multiplier) as u64
}

fn lowest_matching_file(root: &Path, prefix: &str, suffix: &str) -> Option<PathBuf> {
    let mut paths = fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.into_iter().next()
}

fn valid_temperature(value: &f64) -> bool {
    value.is_finite() && (-20.0..=250.0).contains(value)
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_u64(path: PathBuf) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn read_f64(path: &Path) -> Option<f64> {
    read_trimmed(path.to_path_buf())?.parse().ok()
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "telemon-linux-drm-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write(path: &Path, value: &str) {
        fs::write(path, value).unwrap();
    }

    #[test]
    fn emits_intel_hwmon_and_clock_metrics() {
        let root = temp_dir("i915");
        let class = root.join("class");
        let pci = root.join("pci/0000:00:02.0");
        let driver = root.join("drivers/i915");
        fs::create_dir_all(&class).unwrap();
        fs::create_dir_all(&pci).unwrap();
        fs::create_dir_all(&driver).unwrap();
        fs::create_dir_all(pci.join("hwmon/hwmon0")).unwrap();
        fs::create_dir_all(pci.join("drm/card0")).unwrap();
        write(&pci.join("vendor"), "0x8086\n");
        write(&pci.join("device"), "0x1234\n");
        write(&pci.join("hwmon/hwmon0/temp1_input"), "42000\n");
        write(&pci.join("hwmon/hwmon0/fan1_input"), "1200\n");
        write(&pci.join("drm/card0/gt_act_freq_mhz"), "900\n");
        symlink(&pci, class.join("card0").join("device")).unwrap_err();
        fs::create_dir_all(class.join("card0")).unwrap();
        symlink(&pci, class.join("card0/device")).unwrap();
        symlink(&driver, pci.join("driver")).unwrap();

        let mut collector = LinuxDrmCollector::new(LinuxDrmConfig {
            enabled: true,
            drm_root: class.clone(),
            ..LinuxDrmConfig::default()
        });
        let result = collector.collect();
        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::TEMPERATURE_CELSIUS
                && metric.labels.get("sensor").map(String::as_str) == Some("gpu_edge_temp")
                && (metric.value - 42.0).abs() < 0.001
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_CLOCK_HERTZ && metric.value == 900_000_000.0
        }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fdinfo_utilization_is_delta_based() {
        let root = temp_dir("fdinfo");
        let class = root.join("class");
        let proc = root.join("proc");
        let pci = root.join("pci/0000:00:02.0");
        let driver = root.join("drivers/i915");
        fs::create_dir_all(class.join("card0")).unwrap();
        fs::create_dir_all(&pci).unwrap();
        fs::create_dir_all(&driver).unwrap();
        fs::create_dir_all(proc.join("123/fdinfo")).unwrap();
        write(&pci.join("vendor"), "0x8086\n");
        write(&pci.join("device"), "0x1234\n");
        symlink(&pci, class.join("card0/device")).unwrap();
        symlink(&driver, pci.join("driver")).unwrap();
        write(
            &proc.join("123/fdinfo/1"),
            "drm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t7\ndrm-engine-render:\t100000000\ndrm-resident-local0:\t128 MiB\n",
        );

        let mut collector = LinuxDrmCollector::new(LinuxDrmConfig {
            enabled: true,
            drm_root: class.clone(),
            proc_root: proc.clone(),
            target_pid: Some(123),
            include_fdinfo: true,
            ..LinuxDrmConfig::default()
        });
        assert!(collector.collect().success);
        thread::sleep(Duration::from_millis(10));
        write(
            &proc.join("123/fdinfo/1"),
            "drm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t7\ndrm-engine-render:\t110000000\ndrm-resident-local0:\t128 MiB\n",
        );
        let result = collector.collect();
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_UTILIZATION_RATIO
                && metric.labels.get("sensor").map(String::as_str) == Some("gpu_fdinfo_busy")
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_MEMORY_BYTES
                && metric.labels.get("memory").map(String::as_str) == Some("vram")
        }));

        fs::remove_dir_all(root).unwrap();
    }
}
