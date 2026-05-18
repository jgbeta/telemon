use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};

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

#[derive(Debug, Clone)]
struct LinuxHwmonScan {
    readings: Vec<TemperatureReading>,
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
        if !cfg!(target_os = "linux") {
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

        if !cfg!(target_os = "linux") {
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

pub fn discover_readings(config: &LinuxHwmonConfig) -> Result<Vec<TemperatureReading>> {
    Ok(discover_scan(config)?.readings)
}

pub fn discover_details(config: &LinuxHwmonConfig) -> Result<LinuxHwmonDiscovery> {
    Ok(discover_scan(config)?.discovery)
}

fn discover_scan(config: &LinuxHwmonConfig) -> Result<LinuxHwmonScan> {
    let mut discovery = LinuxHwmonDiscovery {
        root_exists: config.root.exists(),
        ..LinuxHwmonDiscovery::default()
    };

    if !config.root.exists() {
        return Ok(LinuxHwmonScan {
            readings: Vec::new(),
            discovery,
        });
    }

    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut readings = Vec::new();
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

        readings.extend(read_hwmon_dir(&path, component, &allowlist, &denylist)?);
    }

    disambiguate_duplicate_sensors(&mut readings);
    discovery.readings_emitted = readings.len();

    Ok(LinuxHwmonScan {
        readings,
        discovery,
    })
}

fn read_hwmon_dir(
    path: &Path,
    component: Component,
    allowlist: &BTreeSet<String>,
    denylist: &BTreeSet<String>,
) -> Result<Vec<TemperatureReading>> {
    let mut readings = Vec::new();

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
        let sensor = normalize_sensor_label(
            raw_label
                .as_deref()
                .unwrap_or_else(|| file_name.trim_end_matches("_input")),
        );

        if !allowlist.is_empty() && !allowlist.contains(&sensor) {
            continue;
        }
        if denylist.contains(&sensor) {
            continue;
        }

        let critical_celsius = read_optional_celsius(path.join(format!("temp{index}_crit")))?;
        let warning_celsius = read_optional_celsius(path.join(format!("temp{index}_max")))?;

        readings.push(TemperatureReading {
            component,
            sensor,
            source: SOURCE,
            temperature_celsius: milli_celsius_to_celsius(raw_temperature),
            critical_celsius,
            warning_celsius,
            raw_label,
        });
    }

    Ok(readings)
}

fn disambiguate_duplicate_sensors(readings: &mut [TemperatureReading]) {
    let mut totals: BTreeMap<(String, String), usize> = BTreeMap::new();
    for reading in readings.iter() {
        *totals
            .entry((
                reading.component.label_value().to_string(),
                reading.sensor.clone(),
            ))
            .or_default() += 1;
    }

    let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
    for reading in readings.iter_mut() {
        let key = (
            reading.component.label_value().to_string(),
            reading.sensor.clone(),
        );
        let total = totals.get(&key).copied().unwrap_or_default();
        if total <= 1 {
            continue;
        }

        let index = seen.entry(key).or_default();
        *index += 1;
        if *index > 1 {
            reading.sensor = format!("{}_{}", reading.sensor, *index);
        }
    }
}

fn reading_to_metrics(reading: &TemperatureReading) -> Vec<MetricSample> {
    let mut metrics = vec![MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Temperature reading in degrees Celsius.",
        labels(&[
            ("component", reading.component.label_value()),
            ("sensor", reading.sensor.as_str()),
            ("source", reading.source),
        ]),
        reading.temperature_celsius,
    )];

    if let Some(critical) = reading.critical_celsius {
        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_LIMIT_CELSIUS,
            "Temperature limit in degrees Celsius.",
            labels(&[
                ("component", reading.component.label_value()),
                ("sensor", reading.sensor.as_str()),
                ("source", reading.source),
                ("limit", "critical"),
            ]),
            critical,
        ));
    }

    if let Some(warning) = reading.warning_celsius {
        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_LIMIT_CELSIUS,
            "Temperature limit in degrees Celsius.",
            labels(&[
                ("component", reading.component.label_value()),
                ("sensor", reading.sensor.as_str()),
                ("source", reading.source),
                ("limit", "warning"),
            ]),
            warning,
        ));
    }

    metrics
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
    match chip_name.trim().to_ascii_lowercase().as_str() {
        "coretemp" | "k10temp" | "zenpower" => Component::Cpu,
        "amdgpu" => Component::Gpu,
        "nvme" => Component::Storage,
        _ => Component::Unknown,
    }
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
    let value = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .trim()
        .parse::<i64>()
        .with_context(|| format!("failed to parse numeric hwmon value {}", path.display()))?;
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
            .find(|reading| reading.sensor == "package")
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

        assert!(readings.iter().any(|reading| reading.sensor == "package"));
        assert!(!readings.iter().any(|reading| reading.sensor == "composite"));
    }

    #[test]
    fn disambiguates_duplicate_sensor_labels() {
        let mut readings = vec![
            TemperatureReading {
                component: Component::Storage,
                sensor: "composite".to_string(),
                source: SOURCE,
                temperature_celsius: 40.0,
                critical_celsius: None,
                warning_celsius: None,
                raw_label: None,
            },
            TemperatureReading {
                component: Component::Storage,
                sensor: "composite".to_string(),
                source: SOURCE,
                temperature_celsius: 41.0,
                critical_celsius: None,
                warning_celsius: None,
                raw_label: None,
            },
        ];

        disambiguate_duplicate_sensors(&mut readings);

        assert_eq!(readings[0].sensor, "composite");
        assert_eq!(readings[1].sensor, "composite_2");
    }
}
