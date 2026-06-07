use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::temperature::model::{
    milli_celsius_to_celsius, normalize_sensor_label, Component, TemperatureReading,
};
use crate::traits::{
    collector_health_metrics, collector_status_metrics, unix_timestamp_seconds, Collector,
    CollectorResult,
};
use telemon_core::config::LinuxHwmonConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const SOURCE: &str = "linux_hwmon";

#[derive(Debug, Clone)]
pub struct LinuxHwmonCollector {
    config: LinuxHwmonConfig,
    errors_total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinuxHwmonDiscovery {
    pub root_exists: bool,
    pub chips_discovered: usize,
    pub temperature_inputs_discovered: usize,
    pub readings_emitted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonInspection {
    pub root: String,
    pub root_exists: bool,
    pub chips: Vec<LinuxHwmonChipInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonChipInspection {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub component: String,
    pub included_by_config: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvme: Option<LinuxHwmonNvmeInspection>,
    pub attributes: Vec<LinuxHwmonAttributeInspection>,
    pub temperatures: Vec<LinuxHwmonTemperatureInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonAttributeInspection {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonTemperatureInspection {
    pub index: String,
    pub input_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_label: Option<String>,
    pub normalized_sensor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emitted_sensor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_celsius: Option<f64>,
    pub emitted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonNvmeInspection {
    pub hwmon: String,
    pub hwmon_path: String,
    pub canonical_hwmon_path: String,
    pub controller: String,
    pub controller_sysfs_path: String,
    pub storage_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_bdf: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_device_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub namespaces: Vec<LinuxHwmonNvmeNamespaceInspection>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxHwmonNvmeNamespaceInspection {
    pub name: String,
    pub namespace: String,
    pub sysfs_path: String,
    pub dev_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sectors_512: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_block_size_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
struct LinuxHwmonScan {
    readings: Vec<TemperatureReading>,
    nvme_devices: Vec<LinuxHwmonNvmeInspection>,
    extra_metrics: Vec<MetricSample>,
    discovery: LinuxHwmonDiscovery,
}

impl LinuxHwmonCollector {
    pub fn new(config: LinuxHwmonConfig) -> Self {
        Self {
            config,
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &LinuxHwmonConfig) -> String {
        if !cfg!(target_os = "linux") && is_default_hwmon_root(&config.root) {
            return "unsupported on this OS".to_string();
        }

        match discover_details(config) {
            Ok(discovery) => format!(
                "available, root_exists={}, chips={}, temp_inputs={}, emitted_temperature_samples={}",
                discovery.root_exists,
                discovery.chips_discovered,
                discovery.temperature_inputs_discovered,
                discovery.readings_emitted
            ),
            Err(error) => format!("error: {error}"),
        }
    }
}

impl Collector for LinuxHwmonCollector {
    fn name(&self) -> &'static str {
        "linux_hwmon"
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        if !cfg!(target_os = "linux") && is_default_hwmon_root(&self.config.root) {
            self.errors_total += 1;
            return CollectorResult {
                collector: self.name(),
                success: false,
                metrics: collector_status_metrics(
                    self.name(),
                    false,
                    false,
                    self.errors_total,
                    None,
                ),
                error_message: Some("linux_hwmon is unsupported on this OS".to_string()),
                duration: started_at.elapsed(),
            };
        }

        match discover_scan(&self.config) {
            Ok(scan) => {
                let mut metrics = collector_health_metrics(
                    self.name(),
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.extend(discovery_to_metrics(self.name(), &scan.discovery));
                for nvme in &scan.nvme_devices {
                    metrics.extend(nvme_to_metrics(nvme, &self.config));
                }
                metrics.extend(scan.extra_metrics);
                for reading in scan.readings {
                    metrics.extend(reading_to_metrics(&reading));
                }
                CollectorResult::success(self.name(), metrics, started_at)
            }
            Err(error) => {
                self.errors_total += 1;
                CollectorResult::failure(
                    self.name(),
                    error.to_string(),
                    self.errors_total,
                    started_at,
                )
            }
        }
    }
}

fn is_default_hwmon_root(path: &Path) -> bool {
    path == Path::new("/sys/class/hwmon")
}

pub fn discover_readings(config: &LinuxHwmonConfig) -> Result<Vec<TemperatureReading>> {
    Ok(discover_scan(config)?.readings)
}

pub fn discover_details(config: &LinuxHwmonConfig) -> Result<LinuxHwmonDiscovery> {
    Ok(discover_scan(config)?.discovery)
}

pub fn inspect_hardware(config: &LinuxHwmonConfig) -> Result<LinuxHwmonInspection> {
    let mut inspection = LinuxHwmonInspection {
        root: config.root.display().to_string(),
        root_exists: config.root.exists(),
        chips: Vec::new(),
    };

    if !inspection.root_exists {
        return Ok(inspection);
    }

    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut hwmon_dirs = fs::read_dir(&config.root)
        .with_context(|| format!("failed to read hwmon root {}", config.root.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    hwmon_dirs.sort();

    for path in hwmon_dirs {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with("hwmon") {
            continue;
        }

        let name = read_optional_trimmed(path.join("name")).filter(|value| !value.is_empty());
        let component = component_for_chip(name.as_deref().unwrap_or("unknown"));
        let included_by_config = component != Component::Unknown || config.include_unknown_sensors;
        let skip_reason = if included_by_config {
            None
        } else {
            Some("unknown_component_disabled".to_string())
        };
        let canonical_path = fs::canonicalize(&path).ok();
        let nvme = if config.nvme_enrichment_enabled && name.as_deref() == Some("nvme") {
            inspect_nvme_hwmon(&path, canonical_path.as_deref(), config)
        } else {
            None
        };

        inspection.chips.push(LinuxHwmonChipInspection {
            path: path.display().to_string(),
            canonical_path: canonical_path
                .as_ref()
                .map(|path| path.display().to_string()),
            name,
            component: component.label_value().to_string(),
            included_by_config,
            skip_reason: skip_reason.clone(),
            nvme,
            attributes: inspect_hwmon_attributes(&path)?,
            temperatures: inspect_hwmon_temperatures(
                &path,
                included_by_config,
                &allowlist,
                &denylist,
            )?,
        });
    }

    disambiguate_inspection_sensors(&mut inspection.chips);
    Ok(inspection)
}

fn discover_scan(config: &LinuxHwmonConfig) -> Result<LinuxHwmonScan> {
    let mut discovery = LinuxHwmonDiscovery {
        root_exists: config.root.exists(),
        ..LinuxHwmonDiscovery::default()
    };

    if !config.root.exists() {
        return Ok(LinuxHwmonScan {
            readings: Vec::new(),
            nvme_devices: Vec::new(),
            extra_metrics: Vec::new(),
            discovery,
        });
    }

    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut readings = Vec::new();
    let mut nvme_devices = Vec::new();
    let mut extra_metrics = Vec::new();
    let mut hwmon_dirs = fs::read_dir(&config.root)
        .with_context(|| format!("failed to read hwmon root {}", config.root.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    hwmon_dirs.sort();

    for path in hwmon_dirs {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with("hwmon") {
            continue;
        }
        discovery.chips_discovered += 1;
        discovery.temperature_inputs_discovered += count_temp_inputs(&path)?;

        let chip_name =
            read_optional_trimmed(path.join("name")).unwrap_or_else(|| "unknown".to_string());
        let component = component_for_chip(&chip_name);
        if component == Component::Unknown && !config.include_unknown_sensors {
            continue;
        }

        let canonical_path = fs::canonicalize(&path).ok();
        let nvme = if config.nvme_enrichment_enabled && chip_name.trim() == "nvme" {
            inspect_nvme_hwmon(&path, canonical_path.as_deref(), config)
        } else {
            None
        };

        readings.extend(read_hwmon_dir(
            &path,
            &chip_name,
            component,
            &allowlist,
            &denylist,
            nvme.as_ref(),
            config,
        )?);
        extra_metrics.extend(read_hwmon_fan_metrics(&path, &chip_name, component)?);
        if let Some(nvme) = nvme {
            nvme_devices.push(nvme);
        }
    }

    disambiguate_duplicate_sensors(&mut readings);
    discovery.readings_emitted = readings.len();

    Ok(LinuxHwmonScan {
        readings,
        nvme_devices,
        extra_metrics,
        discovery,
    })
}

fn inspect_hwmon_attributes(path: &Path) -> Result<Vec<LinuxHwmonAttributeInspection>> {
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read hwmon directory {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    let mut attributes = Vec::new();
    for entry in entries {
        let entry_path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };

        match fs::read_to_string(&entry_path) {
            Ok(value) => attributes.push(LinuxHwmonAttributeInspection {
                name,
                path: entry_path.display().to_string(),
                value: Some(value.trim().to_string()),
                error: None,
            }),
            Err(error) => attributes.push(LinuxHwmonAttributeInspection {
                name,
                path: entry_path.display().to_string(),
                value: None,
                error: Some(error.to_string()),
            }),
        }
    }

    Ok(attributes)
}

fn inspect_hwmon_temperatures(
    path: &Path,
    included_by_config: bool,
    allowlist: &BTreeSet<String>,
    denylist: &BTreeSet<String>,
) -> Result<Vec<LinuxHwmonTemperatureInspection>> {
    let mut temperatures = Vec::new();
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read hwmon directory {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(index) = temp_input_index(file_name) else {
            continue;
        };

        let input_path = entry.path();
        let (raw_input, celsius, input_error) = inspect_temperature_input(&input_path);
        let raw_label = read_optional_trimmed(path.join(format!("temp{index}_label")))
            .filter(|value| !value.is_empty());
        let normalized_sensor = normalize_sensor_label(
            raw_label
                .as_deref()
                .unwrap_or_else(|| file_name.trim_end_matches("_input")),
        );
        let critical_celsius = read_optional_celsius(path.join(format!("temp{index}_crit")))?;
        let warning_celsius = read_optional_celsius(path.join(format!("temp{index}_max")))?;

        let skip_reason = inspection_skip_reason(
            included_by_config,
            &normalized_sensor,
            allowlist,
            denylist,
            celsius,
            input_error.as_deref(),
        );

        temperatures.push(LinuxHwmonTemperatureInspection {
            index,
            input_path: input_path.display().to_string(),
            raw_input,
            input_error,
            celsius,
            raw_label,
            normalized_sensor,
            emitted_sensor: None,
            critical_celsius,
            warning_celsius,
            emitted: skip_reason.is_none(),
            skip_reason,
        });
    }

    Ok(temperatures)
}

fn inspect_temperature_input(path: &Path) -> (Option<String>, Option<f64>, Option<String>) {
    let raw = match fs::read_to_string(path) {
        Ok(value) => value.trim().to_string(),
        Err(error) => return (None, None, Some(error.to_string())),
    };

    match raw.parse::<i64>() {
        Ok(value) => (Some(raw), Some(milli_celsius_to_celsius(value)), None),
        Err(error) => (Some(raw), None, Some(error.to_string())),
    }
}

fn inspection_skip_reason(
    included_by_config: bool,
    normalized_sensor: &str,
    allowlist: &BTreeSet<String>,
    denylist: &BTreeSet<String>,
    celsius: Option<f64>,
    input_error: Option<&str>,
) -> Option<String> {
    if let Some(error) = input_error {
        return Some(format!("invalid_or_unreadable_input: {error}"));
    }
    if celsius.is_none() {
        return Some("missing_temperature_value".to_string());
    }
    if !included_by_config {
        return Some("unknown_component_disabled".to_string());
    }
    if !allowlist.is_empty() && !allowlist.contains(normalized_sensor) {
        return Some("not_in_allowlist".to_string());
    }
    if denylist.contains(normalized_sensor) {
        return Some("in_denylist".to_string());
    }

    None
}

fn disambiguate_inspection_sensors(chips: &mut [LinuxHwmonChipInspection]) {
    let mut totals: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for chip in chips.iter() {
        let storage_id = chip
            .nvme
            .as_ref()
            .map(|nvme| nvme.storage_id.clone())
            .unwrap_or_default();
        for temperature in chip
            .temperatures
            .iter()
            .filter(|temperature| temperature.emitted)
        {
            *totals
                .entry((
                    chip.component.clone(),
                    temperature.normalized_sensor.clone(),
                    storage_id.clone(),
                ))
                .or_default() += 1;
        }
    }

    let mut seen: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for chip in chips.iter_mut() {
        let storage_id = chip
            .nvme
            .as_ref()
            .map(|nvme| nvme.storage_id.clone())
            .unwrap_or_default();
        for temperature in chip
            .temperatures
            .iter_mut()
            .filter(|temperature| temperature.emitted)
        {
            let key = (
                chip.component.clone(),
                temperature.normalized_sensor.clone(),
                storage_id.clone(),
            );
            let total = totals.get(&key).copied().unwrap_or_default();
            let index = seen.entry(key).or_default();
            *index += 1;
            temperature.emitted_sensor = if total > 1 && *index > 1 {
                Some(format!("{}_{}", temperature.normalized_sensor, *index))
            } else {
                Some(temperature.normalized_sensor.clone())
            };
        }
    }
}

fn read_hwmon_dir(
    path: &Path,
    chip_name: &str,
    component: Component,
    allowlist: &BTreeSet<String>,
    denylist: &BTreeSet<String>,
    nvme: Option<&LinuxHwmonNvmeInspection>,
    config: &LinuxHwmonConfig,
) -> Result<Vec<TemperatureReading>> {
    let mut readings = Vec::new();
    let labels = nvme
        .map(|nvme| nvme_temperature_labels(nvme, config))
        .unwrap_or_default();

    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read hwmon directory {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(index) = temp_input_index(file_name) else {
            continue;
        };

        let input_path = entry.path();
        let Some(raw_temperature) = read_optional_i64(&input_path)? else {
            continue;
        };

        let raw_label = read_optional_trimmed(path.join(format!("temp{index}_label")))
            .filter(|value| !value.is_empty());
        let normalized_sensor = normalize_sensor_label(
            raw_label
                .as_deref()
                .unwrap_or_else(|| file_name.trim_end_matches("_input")),
        );

        if !allowlist.is_empty() && !allowlist.contains(&normalized_sensor) {
            continue;
        }
        if denylist.contains(&normalized_sensor) {
            continue;
        }

        let temperature_celsius = milli_celsius_to_celsius(raw_temperature);
        if !valid_temperature_for_chip(chip_name, &normalized_sensor, temperature_celsius) {
            continue;
        }

        let mapping = temperature_mapping(chip_name, raw_label.as_deref(), &normalized_sensor);
        let mut reading_labels = labels.clone();
        enrich_hardware_labels(
            &mut reading_labels,
            component,
            chip_name,
            nvme,
            &mapping.instance,
        );

        let critical_celsius = read_optional_celsius(path.join(format!("temp{index}_crit")))?;
        let warning_celsius = read_optional_celsius(path.join(format!("temp{index}_max")))?;

        readings.push(TemperatureReading {
            component,
            sensor: mapping.sensor,
            source: SOURCE,
            labels: reading_labels,
            temperature_celsius,
            critical_celsius,
            warning_celsius,
            raw_label,
            raw_channel: Some(format!("temp{index}_input")),
            confidence: mapping.confidence,
        });
    }

    Ok(readings)
}

#[derive(Debug, Clone, PartialEq)]
struct TemperatureMapping {
    sensor: String,
    instance: String,
    confidence: f64,
}

fn temperature_mapping(
    chip_name: &str,
    raw_label: Option<&str>,
    normalized_sensor: &str,
) -> TemperatureMapping {
    let chip = chip_name.trim().to_ascii_lowercase();
    let raw = raw_label.unwrap_or(normalized_sensor).to_ascii_lowercase();

    let (sensor, instance, confidence) = match chip.as_str() {
        "coretemp" => {
            if normalized_sensor == "package" {
                ("cpu_package_temp", "package", 0.99)
            } else if normalized_sensor.starts_with("core_") {
                ("cpu_core_temp", normalized_sensor, 0.99)
            } else {
                ("cpu_temp", normalized_sensor, 0.85)
            }
        }
        "k10temp" | "zenpower" => {
            if normalized_sensor == "tctl"
                || normalized_sensor == "tdie"
                || normalized_sensor == "package"
            {
                ("cpu_package_temp", normalized_sensor, 0.95)
            } else if normalized_sensor.starts_with("ccd") {
                ("cpu_die_temp", normalized_sensor, 0.9)
            } else {
                ("cpu_temp", normalized_sensor, 0.8)
            }
        }
        "nvme" => {
            if normalized_sensor == "composite" {
                ("nvme_composite_temp", "composite", 0.99)
            } else {
                ("nvme_sensor_temp", normalized_sensor, 0.9)
            }
        }
        "acpitz" => ("acpi_thermal_zone_temp", normalized_sensor, 0.75),
        "amdgpu" => {
            if normalized_sensor.contains("junction") || normalized_sensor.contains("hotspot") {
                ("gpu_hotspot_temp", normalized_sensor, 0.9)
            } else if normalized_sensor.contains("mem") {
                ("gpu_memory_temp", normalized_sensor, 0.85)
            } else {
                ("gpu_edge_temp", normalized_sensor, 0.9)
            }
        }
        _ if is_network_chip(&chip) => {
            if raw.contains("phy") {
                ("network_phy_temp", "phy", 0.85)
            } else if raw.contains("mac") {
                ("network_mac_temp", "mac", 0.85)
            } else {
                ("network_temp", normalized_sensor, 0.65)
            }
        }
        _ if is_asus_ec_chip(&chip) => {
            if normalized_sensor.contains("vrm") {
                ("vrm_temp", normalized_sensor, 0.9)
            } else {
                ("motherboard_temp", normalized_sensor, 0.75)
            }
        }
        _ => (normalized_sensor, normalized_sensor, 0.5),
    };

    TemperatureMapping {
        sensor: sensor.to_string(),
        instance: instance.to_string(),
        confidence,
    }
}

fn enrich_hardware_labels(
    metric_labels: &mut BTreeMap<String, String>,
    component: Component,
    chip_name: &str,
    nvme: Option<&LinuxHwmonNvmeInspection>,
    instance: &str,
) {
    let chip = chip_name.trim();
    metric_labels.insert("source_driver".to_string(), chip.to_string());
    metric_labels.insert("sensor_instance".to_string(), instance.to_string());

    let device_id = match component {
        Component::Cpu => "cpu0".to_string(),
        Component::Gpu => "gpu0".to_string(),
        Component::Storage => nvme
            .map(|nvme| nvme.storage_id.clone())
            .unwrap_or_else(|| "storage".to_string()),
        Component::Motherboard => "board".to_string(),
        Component::Memory => "memory".to_string(),
        Component::Network => format!("net:{}", normalize_sensor_label(chip)),
        Component::Cooling => "cooling".to_string(),
        Component::System => "system".to_string(),
        Component::Battery => "battery".to_string(),
        Component::Unknown => format!("unknown:{}", normalize_sensor_label(chip)),
    };
    metric_labels.insert("device_id".to_string(), device_id);
}

fn valid_temperature_for_chip(chip_name: &str, normalized_sensor: &str, value: f64) -> bool {
    if !value.is_finite() || !(-20.0..=250.0).contains(&value) {
        return false;
    }

    let chip = chip_name.trim().to_ascii_lowercase();
    if chip == "nvme" && normalized_sensor.contains("limit") && value <= 0.0 {
        return false;
    }

    true
}

fn read_hwmon_fan_metrics(
    path: &Path,
    chip_name: &str,
    component: Component,
) -> Result<Vec<MetricSample>> {
    let mut metrics = Vec::new();
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("failed to read hwmon directory {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(index) = fan_input_index(file_name) else {
            continue;
        };
        let Some(raw_rpm) = read_optional_i64(entry.path())? else {
            continue;
        };
        if raw_rpm < 0 {
            continue;
        }

        let raw_label = read_optional_trimmed(path.join(format!("fan{index}_label")))
            .filter(|value| !value.is_empty());
        let normalized_sensor = normalize_sensor_label(
            raw_label
                .as_deref()
                .unwrap_or_else(|| file_name.trim_end_matches("_input")),
        );
        let fan_component = if normalized_sensor.contains("water_flow") {
            Component::Cooling
        } else {
            component
        };
        let sensor = if normalized_sensor.contains("water_flow") {
            "water_flow_rpm".to_string()
        } else {
            format!("{normalized_sensor}_rpm")
        };
        let instance = if normalized_sensor.contains("water_flow") {
            "water_flow".to_string()
        } else {
            normalized_sensor.clone()
        };

        let mut metric_labels = labels(&[
            ("component", fan_component.label_value()),
            ("sensor", sensor.as_str()),
            ("source", SOURCE),
        ]);
        enrich_hardware_labels(
            &mut metric_labels,
            fan_component,
            chip_name,
            None,
            &instance,
        );

        metrics.push(MetricSample::gauge(
            names::HARDWARE_FAN_SPEED_RPM,
            "Hardware fan or pump speed in revolutions per minute.",
            metric_labels.clone(),
            raw_rpm as f64,
        ));

        let mut info_labels = metric_labels;
        if let Some(raw_label) = raw_label {
            info_labels.insert("raw_label".to_string(), raw_label);
        }
        info_labels.insert("raw_channel".to_string(), format!("fan{index}_input"));
        info_labels.insert("confidence".to_string(), "0.8".to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_SENSOR_INFO,
            "Hardware sensor mapping information.",
            info_labels,
            1.0,
        ));
    }

    Ok(metrics)
}

fn disambiguate_duplicate_sensors(readings: &mut [TemperatureReading]) {
    let mut totals: BTreeMap<(String, String, String, String), usize> = BTreeMap::new();
    for reading in readings.iter() {
        *totals
            .entry((
                reading.component.label_value().to_string(),
                reading.sensor.clone(),
                reading.labels.get("device_id").cloned().unwrap_or_default(),
                reading
                    .labels
                    .get("sensor_instance")
                    .cloned()
                    .unwrap_or_default(),
            ))
            .or_default() += 1;
    }

    let mut seen: BTreeMap<(String, String, String, String), usize> = BTreeMap::new();
    for reading in readings.iter_mut() {
        let key = (
            reading.component.label_value().to_string(),
            reading.sensor.clone(),
            reading.labels.get("device_id").cloned().unwrap_or_default(),
            reading
                .labels
                .get("sensor_instance")
                .cloned()
                .unwrap_or_default(),
        );
        let total = totals.get(&key).copied().unwrap_or_default();
        if total <= 1 {
            continue;
        }

        let index = seen.entry(key).or_default();
        *index += 1;
        if *index > 1 {
            if let Some(instance) = reading.labels.get_mut("sensor_instance") {
                *instance = format!("{instance}_{index}");
            } else {
                reading.sensor = format!("{}_{}", reading.sensor, *index);
            }
        }
    }
}

fn reading_to_metrics(reading: &TemperatureReading) -> Vec<MetricSample> {
    let mut temperature_labels = labels(&[
        ("component", reading.component.label_value()),
        ("sensor", reading.sensor.as_str()),
        ("source", reading.source),
    ]);
    temperature_labels.extend(reading.labels.clone());

    let mut metrics = vec![MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Hardware temperature reading in degrees Celsius.",
        temperature_labels,
        reading.temperature_celsius,
    )];

    let mut info_labels = labels(&[
        ("component", reading.component.label_value()),
        ("sensor", reading.sensor.as_str()),
        ("source", reading.source),
    ]);
    info_labels.extend(reading.labels.clone());
    if let Some(raw_label) = &reading.raw_label {
        info_labels.insert("raw_label".to_string(), raw_label.clone());
    }
    if let Some(raw_channel) = &reading.raw_channel {
        info_labels.insert("raw_channel".to_string(), raw_channel.clone());
    }
    info_labels.insert(
        "confidence".to_string(),
        format_confidence(reading.confidence),
    );
    metrics.push(MetricSample::gauge(
        names::HARDWARE_SENSOR_INFO,
        "Hardware sensor mapping information.",
        info_labels,
        1.0,
    ));

    if let Some(critical) = reading.critical_celsius {
        let mut limit_labels = labels(&[
            ("component", reading.component.label_value()),
            ("sensor", reading.sensor.as_str()),
            ("source", reading.source),
            ("limit", "critical"),
        ]);
        limit_labels.extend(reading.labels.clone());
        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_LIMIT_CELSIUS,
            "Hardware temperature limit in degrees Celsius.",
            limit_labels,
            critical,
        ));
    }

    if let Some(warning) = reading.warning_celsius {
        let mut limit_labels = labels(&[
            ("component", reading.component.label_value()),
            ("sensor", reading.sensor.as_str()),
            ("source", reading.source),
            ("limit", "warning"),
        ]);
        limit_labels.extend(reading.labels.clone());
        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_LIMIT_CELSIUS,
            "Hardware temperature limit in degrees Celsius.",
            limit_labels,
            warning,
        ));
    }

    metrics
}

fn inspect_nvme_hwmon(
    hwmon_path: &Path,
    canonical_path: Option<&Path>,
    config: &LinuxHwmonConfig,
) -> Option<LinuxHwmonNvmeInspection> {
    let hwmon = hwmon_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let canonical = canonical_path
        .map(Path::to_path_buf)
        .or_else(|| fs::canonicalize(hwmon_path).ok())?;
    let controller = extract_nvme_controller_from_path(&canonical)?;
    let sys_root = infer_sys_root_from_hwmon_root(&config.root);
    let mut warnings = Vec::new();

    let controller_path = sys_root
        .as_ref()
        .map(|root| root.join("class/nvme").join(&controller));
    if controller_path.is_none() {
        warnings.push(format!(
            "could not infer sysfs root from hwmon root {}",
            config.root.display()
        ));
    }

    let model = controller_path
        .as_ref()
        .and_then(|path| read_optional_trimmed(path.join("model")))
        .map(clean_sysfs_string);
    let serial = controller_path
        .as_ref()
        .and_then(|path| read_optional_trimmed(path.join("serial")))
        .map(clean_sysfs_string);
    let firmware_rev = controller_path
        .as_ref()
        .and_then(|path| read_optional_trimmed(path.join("firmware_rev")))
        .map(clean_sysfs_string);
    let state = controller_path
        .as_ref()
        .and_then(|path| read_optional_trimmed(path.join("state")))
        .map(clean_sysfs_string);
    let pci_device_path = controller_path
        .as_ref()
        .and_then(|path| fs::canonicalize(path.join("device")).ok());
    let pci_bdf = pci_device_path
        .as_deref()
        .and_then(extract_pci_bdf_from_path);
    let storage_id = pci_bdf
        .as_ref()
        .map(|bdf| format!("pci-{bdf}"))
        .unwrap_or_else(|| format!("controller-{controller}"));
    let namespaces = sys_root
        .as_ref()
        .map(|root| collect_nvme_namespaces(root, &controller))
        .unwrap_or_default();

    Some(LinuxHwmonNvmeInspection {
        hwmon,
        hwmon_path: hwmon_path.display().to_string(),
        canonical_hwmon_path: canonical.display().to_string(),
        controller: controller.clone(),
        controller_sysfs_path: controller_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        storage_id,
        pci_bdf,
        pci_device_path: pci_device_path.map(|path| path.display().to_string()),
        model,
        serial,
        firmware_rev,
        state,
        namespaces,
        warnings,
    })
}

fn nvme_temperature_labels(
    nvme: &LinuxHwmonNvmeInspection,
    config: &LinuxHwmonConfig,
) -> BTreeMap<String, String> {
    let mut metric_labels = BTreeMap::new();
    metric_labels.insert("storage_id".to_string(), nvme.storage_id.clone());
    if let Some(pci_bdf) = &nvme.pci_bdf {
        metric_labels.insert("pci_bdf".to_string(), pci_bdf.clone());
    }
    if config.expose_storage_model {
        if let Some(model) = &nvme.model {
            metric_labels.insert("storage_model".to_string(), model.clone());
        }
    }
    metric_labels
}

fn nvme_to_metrics(
    nvme: &LinuxHwmonNvmeInspection,
    config: &LinuxHwmonConfig,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let mut device_labels = labels(&[
        ("component", "storage"),
        ("device_id", nvme.storage_id.as_str()),
        ("source", SOURCE),
        ("source_driver", "nvme"),
        ("storage_id", nvme.storage_id.as_str()),
        ("controller", nvme.controller.as_str()),
    ]);
    if let Some(pci_bdf) = &nvme.pci_bdf {
        device_labels.insert("pci_bdf".to_string(), pci_bdf.clone());
    }
    if config.expose_storage_model {
        if let Some(model) = &nvme.model {
            device_labels.insert("storage_model".to_string(), model.clone());
        }
    }
    if let Some(firmware_rev) = &nvme.firmware_rev {
        device_labels.insert("firmware_rev".to_string(), firmware_rev.clone());
    }
    if let Some(state) = &nvme.state {
        device_labels.insert("state".to_string(), state.clone());
    }

    metrics.push(MetricSample::gauge(
        names::STORAGE_DEVICE_INFO,
        "Hardware storage device identity information.",
        device_labels,
        1.0,
    ));

    for namespace in &nvme.namespaces {
        let Some(size_bytes) = namespace.size_bytes else {
            continue;
        };
        metrics.push(MetricSample::gauge(
            names::STORAGE_NAMESPACE_CAPACITY_BYTES,
            "Linux NVMe namespace capacity in decimal megabytes.",
            labels(&[
                ("component", "storage"),
                ("device_id", nvme.storage_id.as_str()),
                ("source", SOURCE),
                ("source_driver", "nvme"),
                ("storage_id", nvme.storage_id.as_str()),
                ("namespace", namespace.namespace.as_str()),
            ]),
            size_bytes as f64 / 1_000_000.0,
        ));
    }

    metrics
}

fn infer_sys_root_from_hwmon_root(root: &Path) -> Option<PathBuf> {
    let hwmon = root.file_name()?.to_str()?;
    let class = root.parent()?.file_name()?.to_str()?;
    if hwmon == "hwmon" && class == "class" {
        return root.parent()?.parent().map(Path::to_path_buf);
    }
    None
}

fn extract_nvme_controller_from_path(path: &Path) -> Option<String> {
    let mut previous_was_nvme_dir = false;
    for component in path.components() {
        let value = component.as_os_str().to_string_lossy();
        if previous_was_nvme_dir && is_nvme_controller_name(&value) {
            return Some(value.to_string());
        }
        previous_was_nvme_dir = value == "nvme";
    }
    None
}

fn is_nvme_controller_name(value: &str) -> bool {
    value
        .strip_prefix("nvme")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
}

fn extract_pci_bdf_from_path(path: &Path) -> Option<String> {
    path.components()
        .rev()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .find(|value| is_pci_bdf(value))
}

fn is_pci_bdf(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    bytes[4] == b':'
        && bytes[7] == b':'
        && bytes[10] == b'.'
        && bytes[..4].iter().all(u8::is_ascii_hexdigit)
        && bytes[5..7].iter().all(u8::is_ascii_hexdigit)
        && bytes[8..10].iter().all(u8::is_ascii_hexdigit)
        && bytes[11].is_ascii_digit()
}

fn collect_nvme_namespaces(
    sys_root: &Path,
    controller: &str,
) -> Vec<LinuxHwmonNvmeNamespaceInspection> {
    let mut namespaces = Vec::new();
    let block_root = sys_root.join("block");
    let Ok(entries) = fs::read_dir(&block_root) else {
        return namespaces;
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(namespace) = nvme_namespace_label(controller, &name) else {
            continue;
        };
        let sysfs_path = entry.path();
        let sectors_512 = read_optional_u64(sysfs_path.join("size"));
        let logical_block_size_bytes =
            read_optional_u64(sysfs_path.join("queue/logical_block_size"));
        let size_bytes = sectors_512.and_then(|sectors| sectors.checked_mul(512));

        namespaces.push(LinuxHwmonNvmeNamespaceInspection {
            name: name.clone(),
            namespace,
            sysfs_path: sysfs_path.display().to_string(),
            dev_path: format!("/dev/{name}"),
            size_bytes,
            sectors_512,
            logical_block_size_bytes,
        });
    }

    namespaces.sort_by(|left, right| left.name.cmp(&right.name));
    namespaces
}

fn nvme_namespace_label(controller: &str, name: &str) -> Option<String> {
    let suffix = name.strip_prefix(controller)?;
    if suffix.len() > 1
        && suffix.starts_with('n')
        && suffix[1..].chars().all(|ch| ch.is_ascii_digit())
    {
        Some(suffix.to_string())
    } else {
        None
    }
}

fn read_optional_u64(path: impl Into<PathBuf>) -> Option<u64> {
    fs::read_to_string(path.into())
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn clean_sysfs_string(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn discovery_to_metrics(
    collector: &'static str,
    discovery: &LinuxHwmonDiscovery,
) -> Vec<MetricSample> {
    vec![
        MetricSample::gauge(
            names::COLLECTOR_SAMPLES,
            "Useful samples emitted by a collector in the last collection run.",
            labels(&[("collector", collector), ("kind", "temperature")]),
            discovery.readings_emitted as f64,
        ),
        MetricSample::gauge(
            names::HWMON_CHIPS_DISCOVERED,
            "Linux hwmon chip directories discovered in the last collection run.",
            labels(&[("collector", collector)]),
            discovery.chips_discovered as f64,
        ),
        MetricSample::gauge(
            names::HWMON_TEMPERATURE_INPUTS_DISCOVERED,
            "Linux hwmon temp input files discovered in the last collection run.",
            labels(&[("collector", collector)]),
            discovery.temperature_inputs_discovered as f64,
        ),
    ]
}

fn component_for_chip(chip_name: &str) -> Component {
    let chip = chip_name.trim().to_ascii_lowercase();
    if matches!(chip.as_str(), "coretemp" | "k10temp" | "zenpower") {
        Component::Cpu
    } else if chip == "amdgpu" {
        Component::Gpu
    } else if chip == "nvme" || chip == "drivetemp" {
        Component::Storage
    } else if chip == "acpitz" {
        Component::System
    } else if is_asus_ec_chip(&chip) {
        Component::Motherboard
    } else if is_network_chip(&chip) {
        Component::Network
    } else {
        Component::Unknown
    }
}

fn is_asus_ec_chip(chip: &str) -> bool {
    matches!(chip, "asusec" | "asus_ec_sensors" | "asus")
}

fn is_network_chip(chip: &str) -> bool {
    chip.starts_with("en")
        || chip.contains("ethernet")
        || chip.contains("iwlwifi")
        || chip.contains("wifi")
        || chip.contains("phy")
}

fn format_confidence(value: f64) -> String {
    format!("{:.2}", value.clamp(0.0, 1.0))
}

fn count_temp_inputs(path: &Path) -> Result<usize> {
    let entries = fs::read_dir(path)
        .with_context(|| format!("failed to read hwmon directory {}", path.display()))?;
    Ok(entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|file_name| temp_input_index(file_name).is_some())
        .count())
}

fn temp_input_index(file_name: &str) -> Option<String> {
    file_name
        .strip_prefix("temp")
        .and_then(|value| value.strip_suffix("_input"))
        .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
}

fn fan_input_index(file_name: &str) -> Option<String> {
    file_name
        .strip_prefix("fan")
        .and_then(|value| value.strip_suffix("_input"))
        .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
}

fn read_optional_trimmed(path: impl Into<PathBuf>) -> Option<String> {
    fs::read_to_string(path.into())
        .ok()
        .map(|value| value.trim().to_string())
}

fn read_optional_i64(path: impl AsRef<Path>) -> Result<Option<i64>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let Ok(value) = raw.trim().parse::<i64>() else {
        return Ok(None);
    };
    Ok(Some(value))
}

fn read_optional_celsius(path: impl AsRef<Path>) -> Result<Option<f64>> {
    let Some(value) = read_optional_i64(path)? else {
        return Ok(None);
    };
    let celsius = milli_celsius_to_celsius(value);
    if (-100.0..=250.0).contains(&celsius) {
        Ok(Some(celsius))
    } else {
        Ok(None)
    }
}

fn normalized_filter_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| normalize_sensor_label(value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_config() -> LinuxHwmonConfig {
        LinuxHwmonConfig {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hwmon"),
            include_unknown_sensors: false,
            ..LinuxHwmonConfig::default()
        }
    }

    #[test]
    fn parses_fixture_temperatures() {
        let readings = discover_readings(&fixture_config()).unwrap();

        let package = readings
            .iter()
            .find(|reading| reading.sensor == "cpu_package_temp")
            .unwrap();

        assert_eq!(package.component, Component::Cpu);
        assert_eq!(package.temperature_celsius, 67.0);
        assert_eq!(package.critical_celsius, Some(100.0));
        assert_eq!(package.warning_celsius, Some(85.0));
        assert_eq!(package.source, SOURCE);
    }

    #[test]
    fn reports_discovery_counts_before_filtering() {
        let discovery = discover_details(&fixture_config()).unwrap();

        assert!(discovery.root_exists);
        assert_eq!(discovery.chips_discovered, 3);
        assert_eq!(discovery.temperature_inputs_discovered, 4);
        assert_eq!(discovery.readings_emitted, 3);
    }

    #[test]
    fn inspect_hardware_reports_raw_and_emitted_sensor_state() {
        let inspection = inspect_hardware(&fixture_config()).unwrap();

        assert!(inspection.root_exists);
        assert_eq!(inspection.chips.len(), 3);

        let cpu_chip = inspection
            .chips
            .iter()
            .find(|chip| chip.name.as_deref() == Some("coretemp"))
            .unwrap();
        assert!(cpu_chip
            .attributes
            .iter()
            .any(|attribute| attribute.name == "name"
                && attribute.value.as_deref() == Some("coretemp")));
        let package = cpu_chip
            .temperatures
            .iter()
            .find(|temperature| temperature.normalized_sensor == "package")
            .unwrap();
        assert!(package.emitted);
        assert_eq!(package.raw_input.as_deref(), Some("67000"));
        assert_eq!(package.celsius, Some(67.0));
        assert_eq!(package.emitted_sensor.as_deref(), Some("package"));

        let unknown_chip = inspection
            .chips
            .iter()
            .find(|chip| chip.name.as_deref() == Some("unknownchip"))
            .unwrap();
        assert!(!unknown_chip.included_by_config);
        assert_eq!(
            unknown_chip.temperatures[0].skip_reason.as_deref(),
            Some("unknown_component_disabled")
        );
    }

    #[test]
    fn emits_discovery_metrics() {
        let mut collector = LinuxHwmonCollector::new(fixture_config());
        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::COLLECTOR_SAMPLES
                && metric.labels.get("collector").map(String::as_str) == Some("linux_hwmon")
                && metric.labels.get("kind").map(String::as_str) == Some("temperature")
                && metric.value == 3.0
        }));
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::HWMON_CHIPS_DISCOVERED && metric.value == 3.0 }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HWMON_TEMPERATURE_INPUTS_DISCOVERED && metric.value == 4.0
        }));
    }

    #[test]
    fn skips_unknown_sources_by_default() {
        let readings = discover_readings(&fixture_config()).unwrap();

        assert!(!readings.iter().any(|reading| reading.sensor == "mystery"));
    }

    #[test]
    fn includes_unknown_sources_when_configured() {
        let mut config = fixture_config();
        config.include_unknown_sensors = true;

        let readings = discover_readings(&config).unwrap();

        assert!(readings.iter().any(|reading| reading.sensor == "mystery"));
    }

    #[test]
    fn applies_allowlist_and_denylist() {
        let mut config = fixture_config();
        config.sensor_allowlist = vec!["package".to_string(), "composite".to_string()];
        config.sensor_denylist = vec!["composite".to_string()];

        let readings = discover_readings(&config).unwrap();

        assert!(readings
            .iter()
            .any(|reading| reading.sensor == "cpu_package_temp"));
        assert!(!readings
            .iter()
            .any(|reading| reading.sensor == "nvme_composite_temp"));
    }

    #[test]
    fn disambiguates_duplicate_sensor_labels() {
        let mut readings = vec![
            TemperatureReading {
                component: Component::Storage,
                sensor: "composite".to_string(),
                source: SOURCE,
                labels: BTreeMap::new(),
                temperature_celsius: 40.0,
                critical_celsius: None,
                warning_celsius: None,
                raw_label: None,
                raw_channel: None,
                confidence: 0.5,
            },
            TemperatureReading {
                component: Component::Storage,
                sensor: "composite".to_string(),
                source: SOURCE,
                labels: BTreeMap::new(),
                temperature_celsius: 41.0,
                critical_celsius: None,
                warning_celsius: None,
                raw_label: None,
                raw_channel: None,
                confidence: 0.5,
            },
        ];

        disambiguate_duplicate_sensors(&mut readings);

        assert_eq!(readings[0].sensor, "composite");
        assert_eq!(readings[1].sensor, "composite_2");
    }

    #[cfg(unix)]
    struct TempFixture {
        root: PathBuf,
    }

    #[cfg(unix)]
    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    fn nvme_sysfs_fixture_config() -> (TempFixture, LinuxHwmonConfig) {
        let root = std::env::temp_dir().join(format!(
            "telemon-nvme-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sys_root = root.join("sys");
        let hwmon_root = sys_root.join("class/hwmon");
        let nvme_class = sys_root.join("class/nvme");
        fs::create_dir_all(&hwmon_root).unwrap();
        fs::create_dir_all(&nvme_class).unwrap();
        fs::create_dir_all(sys_root.join("block/nvme0n1/queue")).unwrap();
        fs::create_dir_all(sys_root.join("block/nvme1n1/queue")).unwrap();

        create_nvme_fixture_drive(
            &sys_root,
            "hwmon2",
            "nvme1",
            "0000:02:00.0",
            "Samsung SSD 990 PRO 2TB",
            "SERIAL-990",
            "4B2QJXD7",
            4_000,
            51_850,
        );
        create_nvme_fixture_drive(
            &sys_root,
            "hwmon3",
            "nvme0",
            "0000:71:00.0",
            "Samsung SSD 970 EVO Plus 2TB",
            "SERIAL-970",
            "2B2QEXM7",
            8_000,
            46_850,
        );

        (
            TempFixture { root },
            LinuxHwmonConfig {
                root: hwmon_root,
                ..LinuxHwmonConfig::default()
            },
        )
    }

    #[cfg(unix)]
    #[allow(clippy::too_many_arguments)]
    fn create_nvme_fixture_drive(
        sys_root: &Path,
        hwmon: &str,
        controller: &str,
        pci_bdf: &str,
        model: &str,
        serial: &str,
        firmware_rev: &str,
        sectors_512: u64,
        temp_milli_celsius: i64,
    ) {
        let pci_path = sys_root.join("devices/pci0000:00").join(pci_bdf);
        let controller_path = pci_path.join("nvme").join(controller);
        let hwmon_path = controller_path.join(hwmon);
        fs::create_dir_all(&hwmon_path).unwrap();
        fs::write(hwmon_path.join("name"), "nvme\n").unwrap();
        fs::write(hwmon_path.join("temp1_label"), "Composite\n").unwrap();
        fs::write(
            hwmon_path.join("temp1_input"),
            format!("{temp_milli_celsius}\n"),
        )
        .unwrap();
        fs::write(hwmon_path.join("temp1_crit"), "84850\n").unwrap();
        fs::write(hwmon_path.join("temp1_max"), "81850\n").unwrap();
        fs::write(controller_path.join("model"), format!("{model}     \n")).unwrap();
        fs::write(controller_path.join("serial"), format!("{serial}   \n")).unwrap();
        fs::write(
            controller_path.join("firmware_rev"),
            format!("{firmware_rev}\n"),
        )
        .unwrap();
        fs::write(controller_path.join("state"), "live\n").unwrap();
        std::os::unix::fs::symlink(&pci_path, controller_path.join("device")).unwrap();
        std::os::unix::fs::symlink(&hwmon_path, sys_root.join("class/hwmon").join(hwmon)).unwrap();
        std::os::unix::fs::symlink(
            &controller_path,
            sys_root.join("class/nvme").join(controller),
        )
        .unwrap();

        let namespace = format!("{controller}n1");
        let block_path = sys_root.join("block").join(&namespace);
        fs::write(block_path.join("size"), format!("{sectors_512}\n")).unwrap();
        fs::write(block_path.join("queue/logical_block_size"), "512\n").unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn maps_nvme_hwmon_to_drive_labels_and_static_metrics() {
        let (_fixture, config) = nvme_sysfs_fixture_config();
        let mut collector = LinuxHwmonCollector::new(config);
        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().all(|metric| {
            !metric.labels.contains_key("serial")
                && !metric
                    .labels
                    .values()
                    .any(|value| value.contains("SERIAL-"))
        }));

        let storage_temperatures = result
            .metrics
            .iter()
            .filter(|metric| {
                metric.name == names::TEMPERATURE_CELSIUS
                    && metric.labels.get("component").map(String::as_str) == Some("storage")
            })
            .collect::<Vec<_>>();
        assert_eq!(storage_temperatures.len(), 2);
        assert!(storage_temperatures.iter().all(|metric| {
            metric.labels.get("sensor").map(String::as_str) == Some("nvme_composite_temp")
        }));
        assert!(storage_temperatures.iter().any(|metric| {
            metric.labels.get("storage_id").map(String::as_str) == Some("pci-0000:02:00.0")
                && metric.labels.get("pci_bdf").map(String::as_str) == Some("0000:02:00.0")
                && metric.labels.get("storage_model").map(String::as_str)
                    == Some("Samsung SSD 990 PRO 2TB")
                && metric.value == 51.85
        }));

        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::STORAGE_DEVICE_INFO
                && metric.labels.get("storage_id").map(String::as_str) == Some("pci-0000:71:00.0")
                && metric.labels.get("firmware_rev").map(String::as_str) == Some("2B2QEXM7")
                && metric.labels.get("state").map(String::as_str) == Some("live")
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::STORAGE_NAMESPACE_CAPACITY_BYTES
                && metric.labels.get("storage_id").map(String::as_str) == Some("pci-0000:02:00.0")
                && metric.labels.get("namespace").map(String::as_str) == Some("n1")
                && metric.value == 4_000.0 * 512.0 / 1_000_000.0
        }));
    }

    #[test]
    #[cfg(unix)]
    fn inspect_hardware_includes_local_only_nvme_serials() {
        let (_fixture, config) = nvme_sysfs_fixture_config();
        let inspection = inspect_hardware(&config).unwrap();

        let nvme = inspection
            .chips
            .iter()
            .filter_map(|chip| chip.nvme.as_ref())
            .find(|nvme| nvme.storage_id == "pci-0000:02:00.0")
            .unwrap();

        assert_eq!(nvme.controller, "nvme1");
        assert_eq!(nvme.serial.as_deref(), Some("SERIAL-990"));
        assert_eq!(nvme.namespaces[0].name, "nvme1n1");
        assert_eq!(nvme.namespaces[0].namespace, "n1");
        assert_eq!(nvme.namespaces[0].size_bytes, Some(4_000 * 512));
    }
}
