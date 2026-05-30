use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::traits::{
    collector_health_metrics, collector_status_metrics, unix_timestamp_seconds, Collector,
    CollectorResult,
};
use telemon_core::config::LinuxPowerSupplyConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "linux_power_supply";
pub const SOURCE: &str = "linux_power_supply";

#[derive(Debug, Clone)]
pub struct LinuxPowerSupplyCollector {
    config: LinuxPowerSupplyConfig,
    errors_total: u64,
}

#[derive(Debug, Clone, Default)]
struct PowerSupplyScan {
    root_exists: bool,
    batteries: Vec<BatterySnapshot>,
}

#[derive(Debug, Clone, Default)]
struct BatterySnapshot {
    name: String,
    status: Option<String>,
    capacity_percent: Option<f64>,
    voltage_volts: Option<f64>,
    current_amperes: Option<f64>,
    power_watts: Option<PowerReading>,
    charge_ratio: Option<f64>,
    manufacturer: Option<String>,
    model_name: Option<String>,
    technology: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PowerReading {
    watts: f64,
    derived: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxPowerSupplyInspection {
    pub root: String,
    pub root_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_error: Option<String>,
    pub batteries: Vec<LinuxPowerSupplyBatteryInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxPowerSupplyBatteryInspection {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_volts: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_amperes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_watts: Option<f64>,
    pub power_derived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technology: Option<String>,
}

impl LinuxPowerSupplyCollector {
    pub fn new(config: LinuxPowerSupplyConfig) -> Self {
        Self {
            config,
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &LinuxPowerSupplyConfig) -> String {
        let inspection = inspect_hardware(config);
        if !inspection.root_exists {
            return "unavailable, root missing".to_string();
        }
        if let Some(error) = inspection.root_error {
            return format!("error: {error}");
        }
        format!("available, batteries={}", inspection.batteries.len())
    }
}

impl Collector for LinuxPowerSupplyCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match scan_power_supplies(&self.config) {
            Ok(scan) => {
                let supported = scan.root_exists && !scan.batteries.is_empty();
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
                for battery in scan.batteries {
                    metrics.extend(battery_to_metrics(&battery));
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

pub fn inspect_hardware(config: &LinuxPowerSupplyConfig) -> LinuxPowerSupplyInspection {
    let mut inspection = LinuxPowerSupplyInspection {
        root: config.root.display().to_string(),
        root_exists: config.root.exists(),
        root_error: None,
        batteries: Vec::new(),
    };

    if !inspection.root_exists {
        return inspection;
    }

    match scan_power_supplies(config) {
        Ok(scan) => {
            inspection.batteries = scan
                .batteries
                .into_iter()
                .map(|battery| LinuxPowerSupplyBatteryInspection {
                    path: config.root.join(&battery.name).display().to_string(),
                    power_watts: battery.power_watts.map(|power| power.watts),
                    power_derived: battery
                        .power_watts
                        .map(|power| power.derived)
                        .unwrap_or(false),
                    name: battery.name,
                    status: battery.status,
                    capacity_percent: battery.capacity_percent,
                    voltage_volts: battery.voltage_volts,
                    current_amperes: battery.current_amperes,
                    manufacturer: battery.manufacturer,
                    model_name: battery.model_name,
                    technology: battery.technology,
                })
                .collect();
        }
        Err(error) => inspection.root_error = Some(error.to_string()),
    }

    inspection
}

fn scan_power_supplies(config: &LinuxPowerSupplyConfig) -> Result<PowerSupplyScan> {
    if !config.root.exists() {
        return Ok(PowerSupplyScan {
            root_exists: false,
            batteries: Vec::new(),
        });
    }

    let mut entries = fs::read_dir(&config.root)
        .with_context(|| format!("failed to read power supply root {}", config.root.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    let mut batteries = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let supply_type = read_trimmed(path.join("type"));
        if supply_type.as_deref() != Some("Battery") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            batteries.push(read_battery(config, name.to_string(), &path));
        }
    }

    Ok(PowerSupplyScan {
        root_exists: true,
        batteries,
    })
}

fn read_battery(config: &LinuxPowerSupplyConfig, name: String, path: &Path) -> BatterySnapshot {
    let status = read_trimmed(path.join("status"));
    let capacity_percent =
        read_f64(path.join("capacity")).filter(|value| (0.0..=100.0).contains(value));
    let voltage_volts = read_f64(path.join("voltage_now")).map(|value| value / 1_000_000.0);
    let current_amperes = read_f64(path.join("current_now")).map(|value| value / 1_000_000.0);
    let mut power_watts = read_f64(path.join("power_now")).map(|value| PowerReading {
        watts: value / 1_000_000.0,
        derived: false,
    });
    if power_watts.is_none() && config.derive_power_when_missing {
        if let (Some(voltage), Some(current)) = (voltage_volts, current_amperes) {
            power_watts = Some(PowerReading {
                watts: voltage * current,
                derived: true,
            });
        }
    }

    let charge_ratio = match (
        read_f64(path.join("charge_now")),
        read_f64(path.join("charge_full")),
    ) {
        (Some(now), Some(full)) if full > 0.0 && now >= 0.0 => Some((now / full).clamp(0.0, 1.0)),
        _ => capacity_percent.map(|value| value / 100.0),
    };

    BatterySnapshot {
        name,
        status,
        capacity_percent,
        voltage_volts,
        current_amperes,
        power_watts,
        charge_ratio,
        manufacturer: read_optional_label(path.join("manufacturer")),
        model_name: read_optional_label(path.join("model_name")),
        technology: read_optional_label(path.join("technology")),
    }
}

fn battery_to_metrics(battery: &BatterySnapshot) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    if let Some(value) = battery.charge_ratio.filter(|value| value.is_finite()) {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_BATTERY_CHARGE_RATIO,
            "Battery charge as a ratio from 0 to 1.",
            labels(&[("battery", &battery.name), ("source", SOURCE)]),
            value,
        ));
    }
    if let Some(value) = battery
        .voltage_volts
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_BATTERY_VOLTAGE_VOLTS,
            "Battery voltage in volts.",
            labels(&[("battery", &battery.name), ("source", SOURCE)]),
            value,
        ));
    }
    if let Some(value) = battery
        .current_amperes
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_BATTERY_CURRENT_AMPERES,
            "Battery current in amperes.",
            labels(&[("battery", &battery.name), ("source", SOURCE)]),
            value,
        ));
    }
    if let Some(power) = battery
        .power_watts
        .filter(|power| power.watts.is_finite() && power.watts >= 0.0)
    {
        let direction = battery_direction(battery.status.as_deref());
        let derived = if power.derived { "true" } else { "false" };
        metrics.push(MetricSample::gauge(
            names::HARDWARE_BATTERY_POWER_WATTS,
            "Battery power in watts.",
            labels(&[
                ("battery", &battery.name),
                ("direction", direction),
                ("derived", derived),
                ("source", SOURCE),
            ]),
            power.watts,
        ));
    }

    let mut info_labels = labels(&[
        ("component", "battery"),
        ("battery", &battery.name),
        ("device_id", &battery.name),
        ("source", SOURCE),
    ]);
    if let Some(value) = &battery.manufacturer {
        info_labels.insert("manufacturer".to_string(), value.clone());
    }
    if let Some(value) = &battery.model_name {
        info_labels.insert("model".to_string(), value.clone());
    }
    if let Some(value) = &battery.technology {
        info_labels.insert("technology".to_string(), value.clone());
    }
    metrics.push(MetricSample::gauge(
        names::HARDWARE_DEVICE_INFO,
        "Hardware device identity information.",
        info_labels,
        1.0,
    ));

    metrics
}

fn battery_direction(status: Option<&str>) -> &'static str {
    match status.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "charging" => "charge",
        Some(value) if value == "discharging" => "discharge",
        _ => "unknown",
    }
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_optional_label(path: PathBuf) -> Option<String> {
    read_trimmed(path).filter(|value| value.trim() != "")
}

fn read_f64(path: PathBuf) -> Option<f64> {
    read_trimmed(path)?.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "telemon-power-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write(path: &Path, value: &str) {
        fs::write(path, value).unwrap();
    }

    #[test]
    fn parses_battery_metrics_and_derives_power() {
        let root = temp_dir("battery");
        let battery = root.join("BAT1");
        fs::create_dir_all(&battery).unwrap();
        write(&battery.join("type"), "Battery\n");
        write(&battery.join("status"), "Discharging\n");
        write(&battery.join("capacity"), "96\n");
        write(&battery.join("voltage_now"), "8734000\n");
        write(&battery.join("current_now"), "1200000\n");
        write(&battery.join("charge_now"), "6481000\n");
        write(&battery.join("charge_full"), "6708000\n");
        write(&battery.join("model_name"), "ATC\n");

        let mut collector = LinuxPowerSupplyCollector::new(LinuxPowerSupplyConfig {
            enabled: true,
            root: root.clone(),
            derive_power_when_missing: true,
        });
        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_BATTERY_CHARGE_RATIO && metric.value > 0.96
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_BATTERY_POWER_WATTS
                && metric.labels.get("derived").map(String::as_str) == Some("true")
                && metric.labels.get("direction").map(String::as_str) == Some("discharge")
                && (metric.value - 10.4808).abs() < 0.0001
        }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_non_battery_power_supplies() {
        let root = temp_dir("mains");
        let mains = root.join("ACAD");
        fs::create_dir_all(&mains).unwrap();
        write(&mains.join("type"), "Mains\n");

        let mut collector = LinuxPowerSupplyCollector::new(LinuxPowerSupplyConfig {
            enabled: true,
            root: root.clone(),
            derive_power_when_missing: true,
        });
        let result = collector.collect();

        assert!(result.success);
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_SUPPORTED && metric.value == 0.0 }));
        assert!(result
            .metrics
            .iter()
            .all(|metric| metric.name != names::HARDWARE_BATTERY_CHARGE_RATIO));

        fs::remove_dir_all(root).unwrap();
    }
}
