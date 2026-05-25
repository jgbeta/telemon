use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "windows")]
use std::io::{Read, Write};
#[cfg(target_os = "windows")]
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(target_os = "windows")]
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
#[cfg(any(target_os = "windows", test))]
use anyhow::{bail, Context};
#[cfg(any(target_os = "windows", test))]
use serde::Deserialize;
use serde::Serialize;

use crate::temperature::model::{normalize_sensor_label, Component};
use crate::traits::{collector_status_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::WindowsLhmHttpConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "windows_lhm_http";
pub const SOURCE: &str = "windows_lhm_http";

pub struct WindowsLhmHttpCollector {
    config: WindowsLhmHttpConfig,
    provider: Box<dyn LhmHttpProvider>,
    errors_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowsLhmHttpInspection {
    pub enabled: bool,
    pub supported: bool,
    pub status: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub sensor_count: usize,
    pub emitted_temperature_count: usize,
    pub temperatures: Vec<WindowsLhmHttpTemperatureInspection>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowsLhmHttpTemperatureInspection {
    pub component: String,
    pub sensor: String,
    pub value_celsius: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsLhmHttpDiscovery {
    pub enabled: bool,
    pub supported: bool,
    pub status: &'static str,
    pub url: String,
    pub sensor_count: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct LhmHttpQuery {
    provider_available: bool,
    sensors: Vec<LhmHttpSensor>,
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct LhmHttpSensor {
    path: Vec<String>,
    name: String,
    value: String,
}

trait LhmHttpProvider: Send + Sync {
    fn query(&mut self, config: &WindowsLhmHttpConfig) -> Result<LhmHttpQuery>;
}

impl WindowsLhmHttpCollector {
    pub fn new(config: WindowsLhmHttpConfig) -> Self {
        Self {
            config,
            provider: default_provider(),
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &WindowsLhmHttpConfig) -> WindowsLhmHttpDiscovery {
        if !config.enabled {
            return WindowsLhmHttpDiscovery {
                enabled: false,
                supported: false,
                status: "disabled",
                url: config.url.clone(),
                sensor_count: 0,
                message: None,
            };
        }

        let mut provider = default_provider();
        discovery_from_query_result(config, provider.query(config))
    }

    #[cfg(test)]
    fn with_provider(
        config: WindowsLhmHttpConfig,
        provider: impl LhmHttpProvider + 'static,
    ) -> Self {
        Self {
            config,
            provider: Box::new(provider),
            errors_total: 0,
        }
    }
}

pub fn inspect_hardware(config: &WindowsLhmHttpConfig) -> WindowsLhmHttpInspection {
    if !config.enabled {
        return WindowsLhmHttpInspection {
            enabled: false,
            supported: false,
            status: "disabled".to_string(),
            url: config.url.clone(),
            message: None,
            sensor_count: 0,
            emitted_temperature_count: 0,
            temperatures: Vec::new(),
        };
    }

    let mut provider = default_provider();
    match provider.query(config) {
        Ok(query) if !query.provider_available => WindowsLhmHttpInspection {
            enabled: true,
            supported: false,
            status: missing_provider_status(config).to_string(),
            url: config.url.clone(),
            message: query.message,
            sensor_count: 0,
            emitted_temperature_count: 0,
            temperatures: Vec::new(),
        },
        Ok(query) => {
            let sensor_count = query.sensors.len();
            let readings = temperature_readings(config, query.sensors);
            let temperatures = readings
                .iter()
                .map(|reading| WindowsLhmHttpTemperatureInspection {
                    component: reading.component.label_value().to_string(),
                    sensor: reading.sensor.clone(),
                    value_celsius: reading.value_celsius,
                })
                .collect::<Vec<_>>();

            WindowsLhmHttpInspection {
                enabled: true,
                supported: true,
                status: "available".to_string(),
                url: config.url.clone(),
                message: query.message,
                sensor_count,
                emitted_temperature_count: temperatures.len(),
                temperatures,
            }
        }
        Err(error) => WindowsLhmHttpInspection {
            enabled: true,
            supported: true,
            status: "error".to_string(),
            url: config.url.clone(),
            message: Some(error.to_string()),
            sensor_count: 0,
            emitted_temperature_count: 0,
            temperatures: Vec::new(),
        },
    }
}

impl Collector for WindowsLhmHttpCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match self.provider.query(&self.config) {
            Ok(query) if !query.provider_available => {
                if self.config.require_provider {
                    self.errors_total += 1;
                }
                unsupported_result(&self.config, self.errors_total, query.message, started_at)
            }
            Ok(query) => {
                let mut metrics = collector_status_metrics(
                    COLLECTOR_NAME,
                    true,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.extend(sensor_metrics(&self.config, query.sensors));
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

fn discovery_from_query_result(
    config: &WindowsLhmHttpConfig,
    result: Result<LhmHttpQuery>,
) -> WindowsLhmHttpDiscovery {
    match result {
        Ok(query) if !query.provider_available => WindowsLhmHttpDiscovery {
            enabled: true,
            supported: false,
            status: missing_provider_status(config),
            url: config.url.clone(),
            sensor_count: 0,
            message: query.message,
        },
        Ok(query) => WindowsLhmHttpDiscovery {
            enabled: true,
            supported: true,
            status: "available",
            url: config.url.clone(),
            sensor_count: query.sensors.len(),
            message: query.message,
        },
        Err(error) => WindowsLhmHttpDiscovery {
            enabled: true,
            supported: true,
            status: "error",
            url: config.url.clone(),
            sensor_count: 0,
            message: Some(error.to_string()),
        },
    }
}

fn unsupported_result(
    config: &WindowsLhmHttpConfig,
    errors_total: u64,
    message: Option<String>,
    started_at: Instant,
) -> CollectorResult {
    CollectorResult {
        collector: COLLECTOR_NAME,
        success: !config.require_provider,
        metrics: collector_status_metrics(COLLECTOR_NAME, false, false, errors_total, None),
        error_message: message,
        duration: started_at.elapsed(),
    }
}

fn missing_provider_status(config: &WindowsLhmHttpConfig) -> &'static str {
    if config.require_provider {
        "missing_required_provider"
    } else {
        "missing_provider"
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TemperatureReading {
    component: Component,
    sensor: String,
    instance: String,
    value_celsius: f64,
}

fn temperature_readings(
    config: &WindowsLhmHttpConfig,
    sensors: Vec<LhmHttpSensor>,
) -> Vec<TemperatureReading> {
    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut readings = Vec::new();
    let mut duplicate_counts: BTreeMap<(String, String, String), usize> = BTreeMap::new();

    for sensor in sensors {
        let Some(value_celsius) = parse_temperature_celsius(&sensor.value) else {
            continue;
        };
        if !valid_temperature(value_celsius) {
            continue;
        }

        let component = map_component(&sensor.path);
        if should_skip_zero_temperature(component, value_celsius) {
            continue;
        }
        if component == Component::Unknown && !config.include_unknown_sensors {
            continue;
        }

        let normalized_sensor = normalize_lhm_sensor_label(&sensor.name);
        if !allowlist.is_empty() && !allowlist.contains(&normalized_sensor) {
            continue;
        }
        if denylist.contains(&normalized_sensor) {
            continue;
        }

        let sensor_name = canonical_temperature_sensor(component, &sensor.path, &normalized_sensor);
        let mut instance = normalized_sensor;
        let duplicate_key = (
            component.label_value().to_string(),
            sensor_name.clone(),
            instance.clone(),
        );
        let count = duplicate_counts.entry(duplicate_key).or_insert(0);
        *count += 1;
        if *count > 1 {
            instance = format!("{instance}_{count}");
        }

        readings.push(TemperatureReading {
            component,
            sensor: sensor_name,
            instance,
            value_celsius,
        });
    }

    readings
}

fn sensor_metrics(config: &WindowsLhmHttpConfig, sensors: Vec<LhmHttpSensor>) -> Vec<MetricSample> {
    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut metrics = Vec::new();
    let mut duplicate_counts: BTreeMap<(String, String, String, String), usize> = BTreeMap::new();

    for sensor in sensors {
        let Some(parsed) = parse_lhm_value(&sensor.value, &sensor) else {
            continue;
        };
        if matches!(parsed.kind, LhmValueKind::TemperatureCelsius)
            && !valid_temperature(parsed.value)
        {
            continue;
        }

        let component = map_component(&sensor.path);
        if matches!(parsed.kind, LhmValueKind::TemperatureCelsius)
            && should_skip_zero_temperature(component, parsed.value)
        {
            continue;
        }
        if component == Component::Unknown && !config.include_unknown_sensors {
            continue;
        }

        let normalized_sensor = normalize_lhm_sensor_label(&sensor.name);
        if !allowlist.is_empty() && !allowlist.contains(&normalized_sensor) {
            continue;
        }
        if denylist.contains(&normalized_sensor) {
            continue;
        }

        let sensor_name =
            canonical_lhm_sensor_name(parsed.kind, component, &sensor.path, &normalized_sensor);
        let mut instance = normalized_sensor.clone();
        let mut metric_labels = labels(&[
            ("component", component.label_value()),
            ("sensor", sensor_name.as_str()),
            ("source", SOURCE),
            ("source_driver", "librehardwaremonitor"),
        ]);
        metric_labels.insert(
            "device_id".to_string(),
            lhm_device_id(component, &sensor.path),
        );
        apply_lhm_kind_labels(parsed.kind, &normalized_sensor, &mut metric_labels);

        let duplicate_key = (
            parsed.metric_name.to_string(),
            component.label_value().to_string(),
            sensor_name.clone(),
            instance.clone(),
        );
        let count = duplicate_counts.entry(duplicate_key).or_insert(0);
        *count += 1;
        if *count > 1 {
            instance = format!("{instance}_{count}");
        }
        metric_labels.insert("sensor_instance".to_string(), instance.clone());

        metrics.push(MetricSample::gauge(
            parsed.metric_name,
            parsed.help,
            metric_labels.clone(),
            parsed.value,
        ));

        let mut info_labels = metric_labels;
        info_labels.insert("raw_label".to_string(), sensor.name.clone());
        info_labels.insert("raw_channel".to_string(), sensor.path.join("/"));
        info_labels.insert("confidence".to_string(), "0.70".to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_SENSOR_INFO,
            "Hardware sensor mapping information.",
            info_labels,
            1.0,
        ));
    }

    metrics
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LhmValueKind {
    TemperatureCelsius,
    VoltageVolts,
    CurrentAmperes,
    PowerWatts,
    PowerLimitWatts,
    ClockHertz,
    UtilizationRatio,
    FanSpeedRpm,
    FanSpeedRatio,
    MemoryBytes,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ParsedLhmValue {
    kind: LhmValueKind,
    metric_name: &'static str,
    help: &'static str,
    value: f64,
}

fn parse_lhm_value(value: &str, sensor: &LhmHttpSensor) -> Option<ParsedLhmValue> {
    if let Some(value) = parse_temperature_celsius(value) {
        return Some(ParsedLhmValue {
            kind: LhmValueKind::TemperatureCelsius,
            metric_name: names::TEMPERATURE_CELSIUS,
            help: "Hardware temperature reading in degrees Celsius.",
            value,
        });
    }

    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let normalized_name = normalize_lhm_sensor_label(&sensor.name);

    if lower.ends_with("rpm") {
        return parse_number_before_unit(trimmed, "rpm").map(|value| ParsedLhmValue {
            kind: LhmValueKind::FanSpeedRpm,
            metric_name: names::HARDWARE_FAN_SPEED_RPM,
            help: "Hardware fan or pump speed in revolutions per minute.",
            value,
        });
    }

    if lower.ends_with("mhz") {
        return parse_number_before_unit(trimmed, "mhz").map(|value| ParsedLhmValue {
            kind: LhmValueKind::ClockHertz,
            metric_name: names::HARDWARE_CLOCK_HERTZ,
            help: "Hardware clock speed in hertz.",
            value: value * 1_000_000.0,
        });
    }

    if lower.ends_with('%') {
        let kind = if lhm_path_contains(sensor, "fan") {
            LhmValueKind::FanSpeedRatio
        } else {
            LhmValueKind::UtilizationRatio
        };
        return parse_number_before_unit(trimmed, "%").map(|value| ParsedLhmValue {
            kind,
            metric_name: if kind == LhmValueKind::FanSpeedRatio {
                names::FAN_SPEED_RATIO
            } else {
                names::HARDWARE_UTILIZATION_RATIO
            },
            help: if kind == LhmValueKind::FanSpeedRatio {
                "Hardware fan speed as a ratio from 0 to 1."
            } else {
                "Hardware utilization as a ratio from 0 to 1."
            },
            value: (value / 100.0).clamp(0.0, 1.0),
        });
    }

    if lower.ends_with(" v") || lower.ends_with('v') {
        return parse_number_before_unit(trimmed, "v").map(|value| ParsedLhmValue {
            kind: LhmValueKind::VoltageVolts,
            metric_name: names::HARDWARE_VOLTAGE_VOLTS,
            help: "Hardware voltage in volts.",
            value,
        });
    }

    if lower.ends_with(" a") || lower.ends_with('a') {
        return parse_number_before_unit(trimmed, "a").map(|value| ParsedLhmValue {
            kind: LhmValueKind::CurrentAmperes,
            metric_name: names::HARDWARE_CURRENT_AMPERES,
            help: "Hardware current in amperes.",
            value,
        });
    }

    if lower.ends_with(" w") || lower.ends_with('w') {
        let is_limit = normalized_name.contains("limit");
        return parse_number_before_unit(trimmed, "w").map(|value| ParsedLhmValue {
            kind: if is_limit {
                LhmValueKind::PowerLimitWatts
            } else {
                LhmValueKind::PowerWatts
            },
            metric_name: if is_limit {
                names::HARDWARE_POWER_LIMIT_WATTS
            } else {
                names::HARDWARE_POWER_WATTS
            },
            help: if is_limit {
                "Hardware power limit in watts."
            } else {
                "Hardware power in watts."
            },
            value,
        });
    }

    parse_memory_bytes(trimmed).map(|value| ParsedLhmValue {
        kind: LhmValueKind::MemoryBytes,
        metric_name: names::HARDWARE_MEMORY_BYTES,
        help: "Hardware memory bytes by state.",
        value,
    })
}

fn parse_number_before_unit(value: &str, unit: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    let marker = lower.rfind(unit)?;
    let number_text = value[..marker].trim().replace(',', ".");
    let token = number_text
        .split_whitespace()
        .last()
        .unwrap_or(&number_text);
    token.parse::<f64>().ok()
}

fn parse_memory_bytes(value: &str) -> Option<f64> {
    let lower = value.trim().to_ascii_lowercase();
    let units = [
        ("gib", 1024.0_f64.powi(3)),
        ("gb", 1000.0_f64.powi(3)),
        ("mib", 1024.0_f64.powi(2)),
        ("mb", 1000.0_f64.powi(2)),
        ("kib", 1024.0),
        ("kb", 1000.0),
    ];
    for (unit, multiplier) in units {
        if lower.ends_with(unit) {
            return parse_number_before_unit(value, unit).map(|number| number * multiplier);
        }
    }
    None
}

fn canonical_lhm_sensor_name(
    kind: LhmValueKind,
    component: Component,
    path: &[String],
    normalized_sensor: &str,
) -> String {
    match kind {
        LhmValueKind::TemperatureCelsius => {
            canonical_temperature_sensor(component, path, normalized_sensor)
        }
        LhmValueKind::VoltageVolts => format!("{}_voltage", normalized_sensor),
        LhmValueKind::CurrentAmperes => format!("{}_current", normalized_sensor),
        LhmValueKind::PowerWatts => format!("{}_power", normalized_sensor),
        LhmValueKind::PowerLimitWatts => format!("{}_power_limit", normalized_sensor),
        LhmValueKind::ClockHertz => format!("{}_clock", normalized_sensor),
        LhmValueKind::UtilizationRatio => format!("{}_utilization", normalized_sensor),
        LhmValueKind::FanSpeedRpm => {
            if normalized_sensor.ends_with("_rpm") {
                normalized_sensor.to_string()
            } else {
                format!("{}_rpm", normalized_sensor)
            }
        }
        LhmValueKind::FanSpeedRatio => {
            if normalized_sensor.ends_with("_fan") {
                format!("{}_percent", normalized_sensor)
            } else {
                format!("{}_fan_percent", normalized_sensor)
            }
        }
        LhmValueKind::MemoryBytes => format!("{}_memory", normalized_sensor),
    }
}

fn canonical_temperature_sensor(
    component: Component,
    path: &[String],
    normalized_sensor: &str,
) -> String {
    match component {
        Component::Cpu => {
            if normalized_sensor == "package"
                || normalized_sensor.contains("tctl")
                || normalized_sensor.contains("tdie")
            {
                "cpu_package_temp".to_string()
            } else if normalized_sensor.starts_with("core") {
                "cpu_core_temp".to_string()
            } else {
                "cpu_temp".to_string()
            }
        }
        Component::Gpu => {
            if normalized_sensor.contains("hotspot") || normalized_sensor.contains("hot_spot") {
                "gpu_hotspot_temp".to_string()
            } else if normalized_sensor.contains("memory") {
                "gpu_memory_temp".to_string()
            } else {
                "gpu_edge_temp".to_string()
            }
        }
        Component::Storage => {
            if normalized_sensor == "composite" && lhm_path_contains_text(path, "nvme") {
                "nvme_composite_temp".to_string()
            } else if normalized_sensor == "composite" {
                "storage_composite_temp".to_string()
            } else {
                "storage_sensor_temp".to_string()
            }
        }
        Component::Motherboard => {
            if normalized_sensor.contains("vrm") {
                "vrm_temp".to_string()
            } else {
                "motherboard_temp".to_string()
            }
        }
        Component::Memory => "memory_temp".to_string(),
        Component::Network => "network_temp".to_string(),
        Component::Cooling => "cooling_temp".to_string(),
        Component::System => "system_temp".to_string(),
        Component::Battery => "battery_temp".to_string(),
        Component::Unknown => normalized_sensor.to_string(),
    }
}

fn apply_lhm_kind_labels(
    kind: LhmValueKind,
    normalized_sensor: &str,
    metric_labels: &mut BTreeMap<String, String>,
) {
    match kind {
        LhmValueKind::ClockHertz => {
            let clock = if normalized_sensor.contains("memory") {
                "memory"
            } else if normalized_sensor.contains("bus") {
                "bus"
            } else {
                "core"
            };
            metric_labels.insert("clock".to_string(), clock.to_string());
        }
        LhmValueKind::UtilizationRatio => {
            let engine = if normalized_sensor.contains("memory") {
                "memory"
            } else if normalized_sensor.contains("video") {
                "video"
            } else {
                "total"
            };
            metric_labels.insert("engine".to_string(), engine.to_string());
        }
        LhmValueKind::PowerLimitWatts => {
            metric_labels.insert("limit".to_string(), "current".to_string());
        }
        LhmValueKind::MemoryBytes => {
            metric_labels.insert("memory".to_string(), "ram".to_string());
            let state = if normalized_sensor.contains("free") {
                "free"
            } else if normalized_sensor.contains("used") {
                "used"
            } else {
                "total"
            };
            metric_labels.insert("state".to_string(), state.to_string());
        }
        _ => {}
    }
}

fn lhm_device_id(component: Component, path: &[String]) -> String {
    match component {
        Component::Cpu => "cpu0".to_string(),
        Component::Gpu => "gpu0".to_string(),
        Component::Storage => path
            .iter()
            .find(|part| {
                let lower = part.to_ascii_lowercase();
                lower.contains("nvme")
                    || lower.contains("ssd")
                    || lower.contains("hdd")
                    || lower.contains("disk")
            })
            .map(|part| format!("storage:{}", normalize_sensor_label(part)))
            .unwrap_or_else(|| "storage".to_string()),
        Component::Motherboard => "board".to_string(),
        Component::Memory => "memory".to_string(),
        Component::Network => "network".to_string(),
        Component::Cooling => "cooling".to_string(),
        Component::System => "system".to_string(),
        Component::Battery => "battery".to_string(),
        Component::Unknown => "unknown".to_string(),
    }
}

fn lhm_path_contains(sensor: &LhmHttpSensor, needle: &str) -> bool {
    sensor.name.to_ascii_lowercase().contains(needle)
        || lhm_path_contains_text(&sensor.path, needle)
}

fn lhm_path_contains_text(path: &[String], needle: &str) -> bool {
    path.iter()
        .any(|part| part.to_ascii_lowercase().contains(needle))
}

fn parse_temperature_celsius(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    let marker = trimmed
        .find("\u{00b0}C")
        .or_else(|| trimmed.find("\u{00b0}c"))
        .or_else(|| trimmed.find(" C"));
    let marker = marker?;
    let number_text = trimmed[..marker].trim().replace(',', ".");
    let token = number_text
        .split_whitespace()
        .last()
        .unwrap_or(&number_text);
    token.parse::<f64>().ok()
}

fn map_component(path: &[String]) -> Component {
    let combined = path.join(" ").to_ascii_lowercase();

    if combined.contains("gpu")
        || combined.contains("radeon")
        || combined.contains("geforce")
        || combined.contains("nvidia")
    {
        Component::Gpu
    } else if combined.contains("cpu")
        || combined.contains("processor")
        || combined.contains("ryzen")
        || combined.contains("intel core")
    {
        Component::Cpu
    } else if combined.contains("storage")
        || combined.contains("hdd")
        || combined.contains("ssd")
        || combined.contains("nvme")
        || combined.contains("drive")
        || combined.contains("disk")
    {
        Component::Storage
    } else if combined.contains("motherboard")
        || combined.contains("superio")
        || combined.contains("super i/o")
        || combined.contains("mainboard")
        || combined.contains("vrm")
        || combined.contains("pch")
        || combined.contains("m2 #")
        || combined.contains("asus")
        || combined.contains("embedded controller")
    {
        Component::Motherboard
    } else if combined.contains("memory") || combined.contains("ram") {
        Component::Memory
    } else if combined.contains("network")
        || combined.contains("ethernet")
        || combined.contains("wifi")
    {
        Component::Network
    } else if combined.contains("fan") || combined.contains("pump") || combined.contains("cooler") {
        Component::Cooling
    } else if combined.contains("battery") {
        Component::Battery
    } else if combined.contains("acpi") || combined.contains("thermal zone") {
        Component::System
    } else {
        Component::Unknown
    }
}

fn normalize_lhm_sensor_label(raw: &str) -> String {
    let normalized = normalize_sensor_label(raw);

    if normalized == "cpu_package" || normalized.starts_with("cpu_package_") {
        "package".to_string()
    } else if normalized == "gpu_core" {
        "core".to_string()
    } else if normalized == "gpu_hot_spot" {
        "hotspot".to_string()
    } else if normalized == "gpu_memory_junction" {
        "memory_junction".to_string()
    } else if normalized == "composite_temperature" {
        "composite".to_string()
    } else if let Some(suffix) = normalized.strip_prefix("temperature_") {
        format!("temp{suffix}")
    } else if normalized == "temperature" {
        "temp".to_string()
    } else {
        normalized
    }
}

fn normalized_filter_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| normalize_lhm_sensor_label(value))
        .collect()
}

fn valid_temperature(value: f64) -> bool {
    value.is_finite() && (-100.0..=250.0).contains(&value)
}

fn should_skip_zero_temperature(component: Component, value: f64) -> bool {
    value == 0.0
        && matches!(
            component,
            Component::Storage | Component::Motherboard | Component::Unknown
        )
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Deserialize)]
struct LhmNode {
    #[serde(default, alias = "Text")]
    text: Option<String>,
    #[serde(default, alias = "Value")]
    value: Option<serde_json::Value>,
    #[serde(default, alias = "Children")]
    children: Vec<LhmNode>,
}

#[cfg(any(target_os = "windows", test))]
impl LhmNode {
    fn text_value(&self) -> String {
        self.text.as_deref().unwrap_or_default().trim().to_string()
    }

    fn sensor_value(&self) -> Option<String> {
        let value = self.value.as_ref()?;
        match value {
            serde_json::Value::String(text) => {
                let text = text.trim();
                (!text.is_empty()).then(|| text.to_string())
            }
            serde_json::Value::Number(number) => Some(number.to_string()),
            _ => None,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn parse_lhm_data_json(text: &str) -> Result<LhmHttpQuery> {
    let root: LhmNode = serde_json::from_str(text.trim())
        .with_context(|| "failed to parse LibreHardwareMonitor HTTP JSON response")?;
    let mut sensors = Vec::new();
    collect_lhm_sensors(&root, &[], &mut sensors);

    Ok(LhmHttpQuery {
        provider_available: true,
        sensors,
        message: None,
    })
}

#[cfg(any(target_os = "windows", test))]
fn collect_lhm_sensors(node: &LhmNode, ancestors: &[String], sensors: &mut Vec<LhmHttpSensor>) {
    let name = node.text_value();
    let mut path = ancestors.to_vec();
    if !name.is_empty() {
        path.push(name.clone());
    }

    if let Some(value) = node.sensor_value() {
        sensors.push(LhmHttpSensor {
            path: path.clone(),
            name: if name.is_empty() {
                "sensor".to_string()
            } else {
                name.clone()
            },
            value,
        });
    }

    for child in &node.children {
        collect_lhm_sensors(child, &path, sensors);
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpUrl {
    host: String,
    port: u16,
    path: String,
}

#[cfg(any(target_os = "windows", test))]
fn parse_http_url(raw: &str) -> Result<HttpUrl> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .with_context(|| "LibreHardwareMonitor HTTP URL must start with http://")?;
    let (authority, path) = match without_scheme.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (without_scheme, "/".to_string()),
    };
    if authority.trim().is_empty() {
        bail!("LibreHardwareMonitor HTTP URL host must not be empty");
    }

    let (host, port) = parse_http_authority(authority)?;
    Ok(HttpUrl { host, port, path })
}

#[cfg(any(target_os = "windows", test))]
fn parse_http_authority(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, after_host)) = rest.split_once(']') else {
            bail!("invalid bracketed IPv6 host in LibreHardwareMonitor HTTP URL");
        };
        let port = if after_host.is_empty() {
            80
        } else {
            after_host
                .strip_prefix(':')
                .with_context(|| "invalid port separator in LibreHardwareMonitor HTTP URL")?
                .parse::<u16>()
                .with_context(|| "invalid LibreHardwareMonitor HTTP URL port")?
        };
        return Ok((host.to_string(), port));
    }

    if let Some((host, port)) = authority.rsplit_once(':') {
        if port.chars().all(|ch| ch.is_ascii_digit()) {
            return Ok((
                host.to_string(),
                port.parse::<u16>()
                    .with_context(|| "invalid LibreHardwareMonitor HTTP URL port")?,
            ));
        }
    }

    Ok((authority.to_string(), 80))
}

#[cfg(target_os = "windows")]
enum HttpFetchFailure {
    ProviderUnavailable(String),
    Error(anyhow::Error),
}

#[cfg(target_os = "windows")]
struct HttpLhmProvider;

#[cfg(target_os = "windows")]
impl LhmHttpProvider for HttpLhmProvider {
    fn query(&mut self, config: &WindowsLhmHttpConfig) -> Result<LhmHttpQuery> {
        let timeout = Duration::from_millis(config.timeout_ms);
        match fetch_lhm_json(&config.url, timeout) {
            Ok(body) => parse_lhm_data_json(&body),
            Err(HttpFetchFailure::ProviderUnavailable(message)) => Ok(LhmHttpQuery {
                provider_available: false,
                sensors: Vec::new(),
                message: Some(message),
            }),
            Err(HttpFetchFailure::Error(error)) => Err(error),
        }
    }
}

#[cfg(target_os = "windows")]
fn fetch_lhm_json(url: &str, timeout: Duration) -> std::result::Result<String, HttpFetchFailure> {
    let parsed = parse_http_url(url).map_err(HttpFetchFailure::Error)?;
    let addr = format!("{}:{}", parsed.host, parsed.port);
    let mut socket_addrs = addr.to_socket_addrs().map_err(|error| {
        HttpFetchFailure::Error(anyhow::anyhow!("failed to resolve {addr}: {error}"))
    })?;
    let Some(socket_addr) = socket_addrs.next() else {
        return Err(HttpFetchFailure::Error(anyhow::anyhow!(
            "failed to resolve {addr}: no socket addresses returned"
        )));
    };

    let mut stream = match TcpStream::connect_timeout(&socket_addr, timeout) {
        Ok(stream) => stream,
        Err(error) if is_provider_unavailable_error(&error) => {
            return Err(HttpFetchFailure::ProviderUnavailable(format!(
                "LibreHardwareMonitor HTTP endpoint is not reachable at {url}: {error}"
            )));
        }
        Err(error) => {
            return Err(HttpFetchFailure::Error(anyhow::anyhow!(
                "failed to connect to LibreHardwareMonitor HTTP endpoint {url}: {error}"
            )));
        }
    };

    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| HttpFetchFailure::Error(error.into()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| HttpFetchFailure::Error(error.into()))?;

    let host_header = if parsed.port == 80 {
        parsed.host.clone()
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\nUser-Agent: telemon/{}\r\n\r\n",
        parsed.path,
        host_header,
        env!("CARGO_PKG_VERSION")
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| HttpFetchFailure::Error(error.into()))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| HttpFetchFailure::Error(error.into()))?;
    parse_http_response(&response, url).map_err(HttpFetchFailure::Error)
}

#[cfg(target_os = "windows")]
fn is_provider_unavailable_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::TimedOut
    )
}

#[cfg(any(target_os = "windows", test))]
fn parse_http_response(bytes: &[u8], url: &str) -> Result<String> {
    let Some((headers, body)) = split_http_headers_body(bytes) else {
        bail!("LibreHardwareMonitor HTTP response from {url} did not contain headers");
    };
    let header_text = String::from_utf8_lossy(headers);
    let status_line = header_text
        .lines()
        .next()
        .with_context(|| "LibreHardwareMonitor HTTP response was empty")?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .with_context(|| "LibreHardwareMonitor HTTP response was missing status code")?
        .parse::<u16>()
        .with_context(|| "LibreHardwareMonitor HTTP response status code was invalid")?;
    if !(200..300).contains(&status_code) {
        bail!("LibreHardwareMonitor HTTP endpoint {url} returned status {status_code}");
    }

    let body = if header_text
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked_body(body)?
    } else {
        body.to_vec()
    };

    String::from_utf8(body)
        .with_context(|| "LibreHardwareMonitor HTTP response body was not valid UTF-8")
}

#[cfg(any(target_os = "windows", test))]
fn split_http_headers_body(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    find_bytes(bytes, b"\r\n\r\n")
        .map(|index| (&bytes[..index], &bytes[index + 4..]))
        .or_else(|| find_bytes(bytes, b"\n\n").map(|index| (&bytes[..index], &bytes[index + 2..])))
}

#[cfg(any(target_os = "windows", test))]
fn decode_chunked_body(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut index = 0;

    loop {
        let line_end = find_bytes(&bytes[index..], b"\r\n")
            .map(|offset| index + offset)
            .with_context(|| "invalid chunked HTTP body: missing chunk size terminator")?;
        let line = std::str::from_utf8(&bytes[index..line_end])
            .with_context(|| "invalid chunked HTTP body: chunk size was not UTF-8")?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16)
            .with_context(|| "invalid chunked HTTP body: chunk size was not hex")?;
        index = line_end + 2;

        if size == 0 {
            break;
        }
        if index + size > bytes.len() {
            bail!("invalid chunked HTTP body: chunk exceeded response length");
        }
        output.extend_from_slice(&bytes[index..index + size]);
        index += size;

        if bytes.get(index..index + 2) == Some(b"\r\n") {
            index += 2;
        } else if bytes.get(index) == Some(&b'\n') {
            index += 1;
        } else {
            bail!("invalid chunked HTTP body: missing chunk terminator");
        }
    }

    Ok(output)
}

#[cfg(any(target_os = "windows", test))]
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(not(target_os = "windows"))]
struct UnsupportedPlatformProvider;

#[cfg(not(target_os = "windows"))]
impl LhmHttpProvider for UnsupportedPlatformProvider {
    fn query(&mut self, _config: &WindowsLhmHttpConfig) -> Result<LhmHttpQuery> {
        Ok(LhmHttpQuery {
            provider_available: false,
            sensors: Vec::new(),
            message: Some("windows_lhm_http is unsupported on this OS".to_string()),
        })
    }
}

fn default_provider() -> Box<dyn LhmHttpProvider> {
    #[cfg(target_os = "windows")]
    {
        Box::new(HttpLhmProvider)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Box::new(UnsupportedPlatformProvider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct FakeProvider {
        result: Result<LhmHttpQuery, String>,
    }

    impl LhmHttpProvider for FakeProvider {
        fn query(&mut self, _config: &WindowsLhmHttpConfig) -> Result<LhmHttpQuery> {
            self.result.clone().map_err(anyhow::Error::msg)
        }
    }

    fn available_query(sensors: Vec<LhmHttpSensor>) -> LhmHttpQuery {
        LhmHttpQuery {
            provider_available: true,
            sensors,
            message: None,
        }
    }

    fn sensor(path: &[&str], value: &str) -> LhmHttpSensor {
        LhmHttpSensor {
            path: path.iter().map(|value| value.to_string()).collect(),
            name: path.last().copied().unwrap_or("sensor").to_string(),
            value: value.to_string(),
        }
    }

    fn metric_value(result: &CollectorResult, sensor: &str) -> f64 {
        result
            .metrics
            .iter()
            .find(|metric| {
                metric.name == names::TEMPERATURE_CELSIUS
                    && metric.labels.get("sensor").map(String::as_str) == Some(sensor)
            })
            .map(|metric| metric.value)
            .expect("temperature metric should exist")
    }

    #[test]
    fn parses_lhm_tree_and_emits_expected_temperature_readings() {
        let query = parse_lhm_data_json(
            r#"{
                "Text": "CFC",
                "Children": [
                  {"Text": "AMD Ryzen 5 7600X", "Children": [
                    {"Text": "Temperatures", "Children": [
                      {"Text": "Core (Tctl/Tdie)", "Value": "47.6 °C"},
                      {"Text": "Package", "Value": "43.8 °C"},
                      {"Text": "IOD Hotspot", "Value": "-359627300.0 °C"}
                    ]}
                  ]},
                  {"Text": "NVIDIA GeForce RTX 4070 Ti SUPER", "Children": [
                    {"Text": "Temperatures", "Children": [
                      {"Text": "GPU Core", "Value": "34.0 °C"},
                      {"Text": "GPU Hot Spot", "Value": "43.7 °C"}
                    ]}
                  ]},
                  {"Text": "NVMe Samsung SSD", "Children": [
                    {"Text": "Temperatures", "Children": [
                      {"Text": "Composite Temperature", "Value": "46.0 °C"},
                      {"Text": "Thermal Sensor Low Limit", "Value": "0.0 °C"}
                    ]}
                  ]}
                ]
            }"#,
        )
        .unwrap();

        let readings = temperature_readings(&WindowsLhmHttpConfig::default(), query.sensors);

        assert_eq!(readings.len(), 5);
        assert!(readings.iter().any(|reading| {
            reading.component == Component::Cpu
                && reading.sensor == "cpu_package_temp"
                && reading.instance == "core_tctl_tdie"
                && reading.value_celsius == 47.6
        }));
        assert!(readings.iter().any(|reading| {
            reading.component == Component::Cpu
                && reading.sensor == "cpu_package_temp"
                && reading.value_celsius == 43.8
        }));
        assert!(readings.iter().any(|reading| {
            reading.component == Component::Gpu
                && reading.sensor == "gpu_hotspot_temp"
                && reading.value_celsius == 43.7
        }));
        assert!(readings.iter().any(|reading| {
            reading.component == Component::Storage
                && reading.sensor == "nvme_composite_temp"
                && reading.value_celsius == 46.0
        }));
    }

    #[test]
    fn collect_emits_metrics_from_available_http_provider() {
        let config = WindowsLhmHttpConfig::default();
        let provider = FakeProvider {
            result: Ok(available_query(vec![sensor(
                &["CFC", "AMD Ryzen 5 7600X", "Temperatures", "Package"],
                "67.0 °C",
            )])),
        };
        let mut collector = WindowsLhmHttpCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(result.success);
        assert_eq!(metric_value(&result, "cpu_package_temp"), 67.0);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::TEMPERATURE_CELSIUS
                && metric.labels.get("component").map(String::as_str) == Some("cpu")
                && metric.labels.get("source").map(String::as_str) == Some(SOURCE)
        }));
    }

    #[test]
    fn disambiguates_duplicate_sensor_names_per_component() {
        let config = WindowsLhmHttpConfig::default();
        let readings = temperature_readings(
            &config,
            vec![
                sensor(&["AMD Ryzen", "Temperatures", "Core #1"], "61.0 °C"),
                sensor(&["AMD Ryzen", "Temperatures", "Core #1"], "62.0 °C"),
                sensor(&["NVIDIA GeForce", "Temperatures", "GPU Core"], "40.0 °C"),
            ],
        );

        assert_eq!(readings[0].sensor, "cpu_core_temp");
        assert_eq!(readings[0].instance, "core_1");
        assert_eq!(readings[1].sensor, "cpu_core_temp");
        assert_eq!(readings[1].instance, "core_1_2");
        assert_eq!(readings[2].sensor, "gpu_edge_temp");
    }

    #[test]
    fn skips_unknown_sensors_by_default() {
        let config = WindowsLhmHttpConfig::default();
        let readings = temperature_readings(
            &config,
            vec![sensor(&["Mystery Device", "Temperature #1"], "35.0 °C")],
        );

        assert!(readings.is_empty());
    }

    #[test]
    fn applies_allowlist_and_denylist() {
        let config = WindowsLhmHttpConfig {
            sensor_allowlist: vec!["package".to_string(), "core_1".to_string()],
            sensor_denylist: vec!["core_1".to_string()],
            ..WindowsLhmHttpConfig::default()
        };
        let readings = temperature_readings(
            &config,
            vec![
                sensor(&["AMD Ryzen", "Temperatures", "Package"], "67.0 °C"),
                sensor(&["AMD Ryzen", "Temperatures", "Core #1"], "61.0 °C"),
                sensor(&["AMD Ryzen", "Temperatures", "Core #2"], "62.0 °C"),
            ],
        );

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].sensor, "cpu_package_temp");
    }

    #[test]
    fn filters_non_temperature_implausible_and_zero_placeholder_values() {
        let config = WindowsLhmHttpConfig::default();
        let readings = temperature_readings(
            &config,
            vec![
                sensor(&["AMD Ryzen", "Load", "CPU Total"], "50.0 %"),
                sensor(&["AMD Ryzen", "Temperatures", "Package"], "300.0 °C"),
                sensor(&["NVMe Samsung", "Temperatures", "Thermal Low"], "0.0 °C"),
                sensor(&["AMD Ryzen", "Temperatures", "Core #1"], "61.0 °C"),
            ],
        );

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].sensor, "cpu_core_temp");
        assert_eq!(readings[0].instance, "core_1");
    }

    #[test]
    fn collect_emits_generic_lhm_hardware_metric_families() {
        let config = WindowsLhmHttpConfig::default();
        let provider = FakeProvider {
            result: Ok(available_query(vec![
                sensor(&["NVIDIA GeForce", "Powers", "GPU Power"], "57.6 W"),
                sensor(&["NVIDIA GeForce", "Clocks", "GPU Core"], "2520.0 MHz"),
                sensor(&["NVIDIA GeForce", "Load", "GPU Core"], "31.0 %"),
                sensor(&["NVIDIA GeForce", "Fans", "GPU Fan"], "42.0 %"),
                sensor(&["ASUS EC", "Fans", "Water Flow"], "1200 RPM"),
                sensor(&["ASUS EC", "Voltages", "+12V"], "12.048 V"),
                sensor(&["AMD Ryzen", "Currents", "CPU TDC"], "9.5 A"),
            ])),
        };
        let mut collector = WindowsLhmHttpCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_POWER_WATTS
                && metric.labels.get("component").map(String::as_str) == Some("gpu")
                && metric.value == 57.6
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_CLOCK_HERTZ
                && metric.labels.get("clock").map(String::as_str) == Some("core")
                && metric.value == 2_520_000_000.0
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_UTILIZATION_RATIO
                && metric.labels.get("engine").map(String::as_str) == Some("total")
                && metric.value == 0.31
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::FAN_SPEED_RATIO
                && metric.labels.get("sensor").map(String::as_str) == Some("gpu_fan_percent")
                && metric.value == 0.42
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_FAN_SPEED_RPM
                && metric.labels.get("sensor").map(String::as_str) == Some("water_flow_rpm")
                && metric.value == 1200.0
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_VOLTAGE_VOLTS
                && metric.labels.get("sensor").map(String::as_str) == Some("12v_voltage")
                && metric.value == 12.048
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_CURRENT_AMPERES
                && metric.labels.get("sensor").map(String::as_str) == Some("cpu_tdc_current")
                && metric.value == 9.5
        }));
    }

    #[test]
    fn missing_provider_is_non_fatal_by_default() {
        let config = WindowsLhmHttpConfig::default();
        let provider = FakeProvider {
            result: Ok(LhmHttpQuery {
                provider_available: false,
                sensors: Vec::new(),
                message: Some("connection refused".to_string()),
            }),
        };
        let mut collector = WindowsLhmHttpCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(result.success);
        assert!(result.error_message.is_some());
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_SUPPORTED && metric.value == 0.0 }));
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_UP && metric.value == 0.0 }));
    }

    #[test]
    fn require_provider_marks_missing_provider_as_failure() {
        let config = WindowsLhmHttpConfig {
            require_provider: true,
            ..WindowsLhmHttpConfig::default()
        };
        let provider = FakeProvider {
            result: Ok(LhmHttpQuery {
                provider_available: false,
                sensors: Vec::new(),
                message: Some("connection refused".to_string()),
            }),
        };
        let mut collector = WindowsLhmHttpCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(!result.success);
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_ERRORS_TOTAL && metric.value == 1.0 }));
    }

    #[test]
    fn parses_http_url_with_default_and_explicit_ports() {
        assert_eq!(
            parse_http_url("http://127.0.0.1/data.json").unwrap(),
            HttpUrl {
                host: "127.0.0.1".to_string(),
                port: 80,
                path: "/data.json".to_string(),
            }
        );
        assert_eq!(
            parse_http_url("http://localhost:8085/data.json").unwrap(),
            HttpUrl {
                host: "localhost".to_string(),
                port: 8085,
                path: "/data.json".to_string(),
            }
        );
    }

    #[test]
    fn parses_chunked_http_response() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n";

        let body = parse_http_response(response, "http://localhost:8085/data.json").unwrap();

        assert_eq!(body, r#"{"a":1}"#);
    }
}
