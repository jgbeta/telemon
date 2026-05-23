use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

#[cfg(target_os = "windows")]
use anyhow::anyhow;
#[cfg(any(target_os = "windows", test))]
use anyhow::Context;
use anyhow::Result;
#[cfg(any(target_os = "windows", test))]
use serde::Deserialize;
use serde::Serialize;

#[cfg(target_os = "windows")]
use std::process::Command;

use crate::temperature::model::{normalize_sensor_label, Component};
use crate::traits::{collector_status_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::WindowsLhmWmiConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "windows_lhm_wmi";
pub const SOURCE: &str = "windows_lhm_wmi";

pub struct WindowsLhmWmiCollector {
    config: WindowsLhmWmiConfig,
    provider: Box<dyn LhmWmiProvider>,
    errors_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowsLhmWmiInspection {
    pub enabled: bool,
    pub supported: bool,
    pub status: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub sensor_count: usize,
    pub emitted_temperature_count: usize,
    pub temperatures: Vec<WindowsLhmWmiTemperatureInspection>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowsLhmWmiTemperatureInspection {
    pub component: String,
    pub sensor: String,
    pub value_celsius: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsLhmWmiDiscovery {
    pub enabled: bool,
    pub supported: bool,
    pub status: &'static str,
    pub namespace: String,
    pub sensor_count: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct LhmWmiQuery {
    provider_available: bool,
    sensors: Vec<LhmWmiSensor>,
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct LhmWmiSensor {
    hardware_type: Option<String>,
    hardware_name: Option<String>,
    name: Option<String>,
    sensor_type: Option<String>,
    value: Option<f64>,
}

trait LhmWmiProvider: Send + Sync {
    fn query(&mut self, namespace: &str) -> Result<LhmWmiQuery>;
}

impl WindowsLhmWmiCollector {
    pub fn new(config: WindowsLhmWmiConfig) -> Self {
        Self {
            config,
            provider: default_provider(),
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &WindowsLhmWmiConfig) -> WindowsLhmWmiDiscovery {
        if !config.enabled {
            return WindowsLhmWmiDiscovery {
                enabled: false,
                supported: false,
                status: "disabled",
                namespace: config.namespace.clone(),
                sensor_count: 0,
                message: None,
            };
        }

        let mut provider = default_provider();
        discovery_from_query_result(config, provider.query(&config.namespace))
    }

    #[cfg(test)]
    fn with_provider(config: WindowsLhmWmiConfig, provider: impl LhmWmiProvider + 'static) -> Self {
        Self {
            config,
            provider: Box::new(provider),
            errors_total: 0,
        }
    }
}

pub fn inspect_hardware(config: &WindowsLhmWmiConfig) -> WindowsLhmWmiInspection {
    if !config.enabled {
        return WindowsLhmWmiInspection {
            enabled: false,
            supported: false,
            status: "disabled".to_string(),
            namespace: config.namespace.clone(),
            message: None,
            sensor_count: 0,
            emitted_temperature_count: 0,
            temperatures: Vec::new(),
        };
    }

    let mut provider = default_provider();
    match provider.query(&config.namespace) {
        Ok(query) if !query.provider_available => WindowsLhmWmiInspection {
            enabled: true,
            supported: false,
            status: missing_provider_status(config).to_string(),
            namespace: config.namespace.clone(),
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
                .map(|reading| WindowsLhmWmiTemperatureInspection {
                    component: reading.component.label_value().to_string(),
                    sensor: reading.sensor.clone(),
                    value_celsius: reading.value_celsius,
                })
                .collect::<Vec<_>>();

            WindowsLhmWmiInspection {
                enabled: true,
                supported: true,
                status: "available".to_string(),
                namespace: config.namespace.clone(),
                message: query.message,
                sensor_count,
                emitted_temperature_count: temperatures.len(),
                temperatures,
            }
        }
        Err(error) => WindowsLhmWmiInspection {
            enabled: true,
            supported: true,
            status: "error".to_string(),
            namespace: config.namespace.clone(),
            message: Some(error.to_string()),
            sensor_count: 0,
            emitted_temperature_count: 0,
            temperatures: Vec::new(),
        },
    }
}

impl Collector for WindowsLhmWmiCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match self.provider.query(&self.config.namespace) {
            Ok(query) if !query.provider_available => {
                if self.config.require_provider {
                    self.errors_total += 1;
                }
                unsupported_result(&self.config, self.errors_total, query.message, started_at)
            }
            Ok(query) => {
                let readings = temperature_readings(&self.config, query.sensors);
                let mut metrics = collector_status_metrics(
                    COLLECTOR_NAME,
                    true,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.extend(readings.iter().map(temperature_metric));
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
    config: &WindowsLhmWmiConfig,
    result: Result<LhmWmiQuery>,
) -> WindowsLhmWmiDiscovery {
    match result {
        Ok(query) if !query.provider_available => WindowsLhmWmiDiscovery {
            enabled: true,
            supported: false,
            status: missing_provider_status(config),
            namespace: config.namespace.clone(),
            sensor_count: 0,
            message: query.message,
        },
        Ok(query) => WindowsLhmWmiDiscovery {
            enabled: true,
            supported: true,
            status: "available",
            namespace: config.namespace.clone(),
            sensor_count: query.sensors.len(),
            message: query.message,
        },
        Err(error) => WindowsLhmWmiDiscovery {
            enabled: true,
            supported: true,
            status: "error",
            namespace: config.namespace.clone(),
            sensor_count: 0,
            message: Some(error.to_string()),
        },
    }
}

fn unsupported_result(
    config: &WindowsLhmWmiConfig,
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

fn missing_provider_status(config: &WindowsLhmWmiConfig) -> &'static str {
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
    value_celsius: f64,
}

fn temperature_readings(
    config: &WindowsLhmWmiConfig,
    sensors: Vec<LhmWmiSensor>,
) -> Vec<TemperatureReading> {
    let allowlist = normalized_filter_set(&config.sensor_allowlist);
    let denylist = normalized_filter_set(&config.sensor_denylist);
    let mut readings = Vec::new();
    let mut duplicate_counts: BTreeMap<(String, String), usize> = BTreeMap::new();

    for sensor in sensors {
        let Some(sensor_type) = sensor.sensor_type.as_deref() else {
            continue;
        };
        if !sensor_type.eq_ignore_ascii_case("temperature") {
            continue;
        }

        let Some(value_celsius) = sensor.value else {
            continue;
        };
        if !valid_temperature(value_celsius) {
            continue;
        }

        let component = map_component(
            sensor.hardware_type.as_deref(),
            sensor.hardware_name.as_deref(),
        );
        if component == Component::Unknown && !config.include_unknown_sensors {
            continue;
        }

        let raw_sensor = sensor.name.as_deref().unwrap_or("temperature");
        let normalized_sensor = normalize_lhm_sensor_label(raw_sensor);
        if !allowlist.is_empty() && !allowlist.contains(&normalized_sensor) {
            continue;
        }
        if denylist.contains(&normalized_sensor) {
            continue;
        }

        let duplicate_key = (
            component.label_value().to_string(),
            normalized_sensor.clone(),
        );
        let count = duplicate_counts.entry(duplicate_key).or_insert(0);
        *count += 1;
        let sensor = if *count > 1 {
            format!("{normalized_sensor}_{count}")
        } else {
            normalized_sensor
        };

        readings.push(TemperatureReading {
            component,
            sensor,
            value_celsius,
        });
    }

    readings
}

fn temperature_metric(reading: &TemperatureReading) -> MetricSample {
    MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Temperature reading in degrees Celsius.",
        labels(&[
            ("component", reading.component.label_value()),
            ("sensor", reading.sensor.as_str()),
            ("source", SOURCE),
        ]),
        reading.value_celsius,
    )
}

fn map_component(hardware_type: Option<&str>, hardware_name: Option<&str>) -> Component {
    let combined = format!(
        "{} {}",
        hardware_type.unwrap_or_default(),
        hardware_name.unwrap_or_default()
    )
    .to_ascii_lowercase();

    if combined.contains("cpu") || combined.contains("processor") || combined.contains("ryzen") {
        Component::Cpu
    } else if combined.contains("gpu")
        || combined.contains("radeon")
        || combined.contains("geforce")
    {
        Component::Gpu
    } else if combined.contains("storage")
        || combined.contains("hdd")
        || combined.contains("ssd")
        || combined.contains("nvme")
        || combined.contains("drive")
    {
        Component::Storage
    } else if combined.contains("motherboard")
        || combined.contains("superio")
        || combined.contains("mainboard")
    {
        Component::Motherboard
    } else if combined.contains("battery") {
        Component::Battery
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

#[cfg(any(target_os = "windows", test))]
fn parse_json_value(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(any(target_os = "windows", test))]
fn parse_powershell_response(text: &str) -> Result<LhmWmiQuery> {
    let response: PowerShellLhmResponse = serde_json::from_str(text.trim())
        .with_context(|| "failed to parse LibreHardwareMonitor WMI JSON response")?;
    Ok(response.into())
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Deserialize)]
struct PowerShellLhmResponse {
    #[serde(default)]
    provider_available: bool,
    #[serde(default)]
    sensors: Vec<PowerShellLhmSensor>,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Deserialize)]
struct PowerShellLhmSensor {
    #[serde(rename = "HardwareType")]
    hardware_type: Option<String>,
    #[serde(rename = "HardwareName")]
    hardware_name: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "SensorType")]
    sensor_type: Option<String>,
    #[serde(rename = "Value")]
    value: Option<serde_json::Value>,
}

#[cfg(any(target_os = "windows", test))]
impl From<PowerShellLhmResponse> for LhmWmiQuery {
    fn from(value: PowerShellLhmResponse) -> Self {
        Self {
            provider_available: value.provider_available,
            sensors: value.sensors.into_iter().map(LhmWmiSensor::from).collect(),
            message: value.error.filter(|message| !message.trim().is_empty()),
        }
    }
}

#[cfg(any(target_os = "windows", test))]
impl From<PowerShellLhmSensor> for LhmWmiSensor {
    fn from(value: PowerShellLhmSensor) -> Self {
        Self {
            hardware_type: value.hardware_type,
            hardware_name: value.hardware_name,
            name: value.name,
            sensor_type: value.sensor_type,
            value: value.value.as_ref().and_then(parse_json_value),
        }
    }
}

#[cfg(target_os = "windows")]
struct PowerShellLhmWmiProvider;

#[cfg(target_os = "windows")]
impl LhmWmiProvider for PowerShellLhmWmiProvider {
    fn query(&mut self, namespace: &str) -> Result<LhmWmiQuery> {
        let namespace = powershell_single_quoted(namespace);
        let command = format!(
            "$ErrorActionPreference = 'Stop'; \
             try {{ \
               $sensors = @(Get-CimInstance -Namespace {namespace} -ClassName Sensor -ErrorAction Stop | Select-Object HardwareType,HardwareName,Name,SensorType,Value); \
               [pscustomobject]@{{ provider_available = $true; sensors = $sensors; error = $null }} | ConvertTo-Json -Depth 5 -Compress \
             }} catch {{ \
               [pscustomobject]@{{ provider_available = $false; sensors = @(); error = $_.Exception.Message }} | ConvertTo-Json -Depth 5 -Compress \
             }}"
        );

        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                command.as_str(),
            ])
            .output()
            .with_context(|| {
                "failed to execute powershell.exe for LibreHardwareMonitor WMI query"
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = stderr.trim();
            if message.is_empty() {
                return Err(anyhow!(
                    "LibreHardwareMonitor WMI PowerShell query failed with status {}",
                    output.status
                ));
            }
            return Err(anyhow!(
                "LibreHardwareMonitor WMI PowerShell query failed: {message}"
            ));
        }

        parse_powershell_response(&stdout)
    }
}

#[cfg(not(target_os = "windows"))]
struct UnsupportedPlatformProvider;

#[cfg(not(target_os = "windows"))]
impl LhmWmiProvider for UnsupportedPlatformProvider {
    fn query(&mut self, _namespace: &str) -> Result<LhmWmiQuery> {
        Ok(LhmWmiQuery {
            provider_available: false,
            sensors: Vec::new(),
            message: Some("windows_lhm_wmi is unsupported on this OS".to_string()),
        })
    }
}

fn default_provider() -> Box<dyn LhmWmiProvider> {
    #[cfg(target_os = "windows")]
    {
        Box::new(PowerShellLhmWmiProvider)
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
        result: Result<LhmWmiQuery, String>,
    }

    impl LhmWmiProvider for FakeProvider {
        fn query(&mut self, _namespace: &str) -> Result<LhmWmiQuery> {
            self.result.clone().map_err(anyhow::Error::msg)
        }
    }

    fn sensor(
        hardware_type: &str,
        hardware_name: &str,
        name: &str,
        sensor_type: &str,
        value: f64,
    ) -> LhmWmiSensor {
        LhmWmiSensor {
            hardware_type: Some(hardware_type.to_string()),
            hardware_name: Some(hardware_name.to_string()),
            name: Some(name.to_string()),
            sensor_type: Some(sensor_type.to_string()),
            value: Some(value),
        }
    }

    fn available_query(sensors: Vec<LhmWmiSensor>) -> LhmWmiQuery {
        LhmWmiQuery {
            provider_available: true,
            sensors,
            message: None,
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
    fn emits_amd_cpu_package_temperature() {
        let config = WindowsLhmWmiConfig::default();
        let provider = FakeProvider {
            result: Ok(available_query(vec![sensor(
                "Cpu",
                "AMD Ryzen 5 7600X",
                "CPU Package",
                "Temperature",
                67.0,
            )])),
        };
        let mut collector = WindowsLhmWmiCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(result.success);
        assert_eq!(metric_value(&result, "package"), 67.0);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::TEMPERATURE_CELSIUS
                && metric.labels.get("component").map(String::as_str) == Some("cpu")
                && metric.labels.get("source").map(String::as_str) == Some(SOURCE)
        }));
    }

    #[test]
    fn normalizes_core_temperature_and_disambiguates_duplicates() {
        let config = WindowsLhmWmiConfig::default();
        let readings = temperature_readings(
            &config,
            vec![
                sensor("Cpu", "AMD Ryzen", "Core #1", "Temperature", 61.0),
                sensor("Cpu", "AMD Ryzen", "Core #1", "Temperature", 62.0),
                sensor("GpuNvidia", "NVIDIA", "GPU Core", "Temperature", 40.0),
                sensor(
                    "Motherboard",
                    "Board",
                    "Temperature #1",
                    "Temperature",
                    35.0,
                ),
            ],
        );

        assert_eq!(readings[0].sensor, "core_1");
        assert_eq!(readings[1].sensor, "core_1_2");
        assert_eq!(readings[2].sensor, "core");
        assert_eq!(readings[3].sensor, "temp1");
    }

    #[test]
    fn skips_unknown_sources_by_default() {
        let config = WindowsLhmWmiConfig::default();
        let readings = temperature_readings(
            &config,
            vec![sensor(
                "Mystery",
                "Unknown",
                "Temperature #1",
                "Temperature",
                35.0,
            )],
        );

        assert!(readings.is_empty());
    }

    #[test]
    fn applies_allowlist_and_denylist() {
        let config = WindowsLhmWmiConfig {
            sensor_allowlist: vec!["package".to_string(), "core_1".to_string()],
            sensor_denylist: vec!["core_1".to_string()],
            ..WindowsLhmWmiConfig::default()
        };
        let readings = temperature_readings(
            &config,
            vec![
                sensor("Cpu", "AMD Ryzen", "CPU Package", "Temperature", 67.0),
                sensor("Cpu", "AMD Ryzen", "Core #1", "Temperature", 61.0),
                sensor("Cpu", "AMD Ryzen", "Core #2", "Temperature", 62.0),
            ],
        );

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].sensor, "package");
    }

    #[test]
    fn filters_non_temperature_and_implausible_values() {
        let config = WindowsLhmWmiConfig::default();
        let readings = temperature_readings(
            &config,
            vec![
                sensor("Cpu", "AMD Ryzen", "CPU Total", "Load", 50.0),
                sensor("Cpu", "AMD Ryzen", "CPU Package", "Temperature", f64::NAN),
                sensor("Cpu", "AMD Ryzen", "CPU Package", "Temperature", 300.0),
                sensor("Cpu", "AMD Ryzen", "Core #1", "Temperature", 61.0),
            ],
        );

        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].sensor, "core_1");
    }

    #[test]
    fn missing_provider_is_non_fatal_by_default() {
        let config = WindowsLhmWmiConfig::default();
        let provider = FakeProvider {
            result: Ok(LhmWmiQuery {
                provider_available: false,
                sensors: Vec::new(),
                message: Some("namespace not found".to_string()),
            }),
        };
        let mut collector = WindowsLhmWmiCollector::with_provider(config, provider);

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
        let config = WindowsLhmWmiConfig {
            require_provider: true,
            ..WindowsLhmWmiConfig::default()
        };
        let provider = FakeProvider {
            result: Ok(LhmWmiQuery {
                provider_available: false,
                sensors: Vec::new(),
                message: Some("namespace not found".to_string()),
            }),
        };
        let mut collector = WindowsLhmWmiCollector::with_provider(config, provider);

        let result = collector.collect();

        assert!(!result.success);
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_ERRORS_TOTAL && metric.value == 1.0 }));
    }

    #[test]
    fn parses_powershell_response_numbers_and_strings() {
        let query = parse_powershell_response(
            r#"{
                "provider_available": true,
                "sensors": [
                  {"HardwareType":"Cpu","HardwareName":"AMD Ryzen","Name":"CPU Package","SensorType":"Temperature","Value":67.5},
                  {"HardwareType":"GpuAmd","HardwareName":"Radeon","Name":"GPU Core","SensorType":"Temperature","Value":"44.25"}
                ],
                "error": null
            }"#,
        )
        .unwrap();

        assert!(query.provider_available);
        assert_eq!(query.sensors.len(), 2);
        assert_eq!(query.sensors[0].value, Some(67.5));
        assert_eq!(query.sensors[1].value, Some(44.25));
    }
}
