use std::collections::BTreeMap;
use std::time::Instant;

use serde::Serialize;

use crate::traits::{collector_status_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::NvidiaNvmlConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

use super::model::{NvidiaDeviceInfo, NvidiaMemory, NvidiaUtilization};
use super::provider::{DynamicNvmlProvider, NvidiaError, NvidiaProvider};

pub const COLLECTOR_NAME: &str = "nvidia_nvml";
pub const SOURCE: &str = "nvidia_nvml";

pub struct NvidiaNvmlCollector {
    config: NvidiaNvmlConfig,
    provider: Option<Box<dyn NvidiaProvider>>,
    load_error: Option<NvidiaError>,
    errors_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaNvmlDiscovery {
    pub enabled: bool,
    pub supported: bool,
    pub library_loaded: bool,
    pub device_count: u32,
    pub status: &'static str,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NvidiaNvmlInspection {
    pub enabled: bool,
    pub supported: bool,
    pub library_loaded: bool,
    pub device_count: u32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub devices: Vec<NvidiaNvmlDeviceInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NvidiaNvmlDeviceInspection {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<NvidiaUtilizationInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<NvidiaMemoryInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_speed_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vbios_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_usage_milliwatts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_limit_milliwatts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphics_clock_mhz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_clock_mhz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance_state: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub field_errors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct NvidiaUtilizationInspection {
    pub graphics_ratio: f64,
    pub memory_ratio: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct NvidiaMemoryInspection {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}

pub fn inspect_hardware(config: &NvidiaNvmlConfig) -> NvidiaNvmlInspection {
    if !config.enabled {
        return NvidiaNvmlInspection {
            enabled: false,
            supported: false,
            library_loaded: false,
            device_count: 0,
            status: "disabled".to_string(),
            message: None,
            devices: Vec::new(),
        };
    }

    let mut provider = match DynamicNvmlProvider::load(&config.library_paths) {
        Ok(provider) => provider,
        Err(error) => {
            return NvidiaNvmlInspection {
                enabled: true,
                supported: false,
                library_loaded: error.library_loaded(),
                device_count: 0,
                status: error.status().to_string(),
                message: Some(error.to_string()),
                devices: Vec::new(),
            };
        }
    };

    inspect_provider(config, &mut provider, true)
}

fn inspect_provider(
    config: &NvidiaNvmlConfig,
    provider: &mut dyn NvidiaProvider,
    library_loaded: bool,
) -> NvidiaNvmlInspection {
    if !provider.is_supported() {
        return NvidiaNvmlInspection {
            enabled: config.enabled,
            supported: false,
            library_loaded,
            device_count: 0,
            status: "unsupported".to_string(),
            message: None,
            devices: Vec::new(),
        };
    }

    let device_count = match provider.device_count() {
        Ok(device_count) => device_count,
        Err(error) => {
            return NvidiaNvmlInspection {
                enabled: config.enabled,
                supported: true,
                library_loaded,
                device_count: 0,
                status: error.status().to_string(),
                message: Some(error.to_string()),
                devices: Vec::new(),
            };
        }
    };

    let mut devices = Vec::new();
    for index in 0..device_count {
        devices.push(inspect_device(provider, index));
    }

    NvidiaNvmlInspection {
        enabled: config.enabled,
        supported: true,
        library_loaded,
        device_count,
        status: if device_count == 0 {
            "no_devices".to_string()
        } else {
            "available".to_string()
        },
        message: None,
        devices,
    }
}

fn inspect_device(provider: &mut dyn NvidiaProvider, index: u32) -> NvidiaNvmlDeviceInspection {
    let mut field_errors = BTreeMap::new();
    let info = match provider.device_info(index) {
        Ok(info) => Some(info),
        Err(error) => {
            field_errors.insert("device_info".to_string(), error.to_string());
            None
        }
    };

    let utilization = inspect_optional(
        &mut field_errors,
        "utilization",
        provider.utilization(index),
    )
    .map(|utilization| NvidiaUtilizationInspection {
        graphics_ratio: utilization.graphics_ratio,
        memory_ratio: utilization.memory_ratio,
    });
    let memory =
        inspect_optional(&mut field_errors, "memory", provider.memory(index)).map(|memory| {
            NvidiaMemoryInspection {
                total_bytes: memory.total_bytes,
                used_bytes: memory.used_bytes,
                free_bytes: memory.free_bytes,
            }
        });

    NvidiaNvmlDeviceInspection {
        index,
        name: info.as_ref().and_then(|info| info.name.clone()),
        uuid: info.as_ref().and_then(|info| info.uuid.clone()),
        temperature_celsius: inspect_optional(
            &mut field_errors,
            "temperature_celsius",
            provider.temperature_celsius(index),
        ),
        utilization,
        memory,
        fan_speed_ratio: inspect_optional(
            &mut field_errors,
            "fan_speed_ratio",
            provider.fan_speed_ratio(index),
        ),
        serial: inspect_optional(&mut field_errors, "serial", provider.serial(index)),
        vbios_version: inspect_optional(
            &mut field_errors,
            "vbios_version",
            provider.vbios_version(index),
        ),
        power_usage_milliwatts: inspect_optional(
            &mut field_errors,
            "power_usage_milliwatts",
            provider.power_usage_milliwatts(index),
        ),
        power_limit_milliwatts: inspect_optional(
            &mut field_errors,
            "power_limit_milliwatts",
            provider.power_limit_milliwatts(index),
        ),
        graphics_clock_mhz: inspect_optional(
            &mut field_errors,
            "graphics_clock_mhz",
            provider.graphics_clock_mhz(index),
        ),
        memory_clock_mhz: inspect_optional(
            &mut field_errors,
            "memory_clock_mhz",
            provider.memory_clock_mhz(index),
        ),
        performance_state: inspect_optional(
            &mut field_errors,
            "performance_state",
            provider.performance_state(index),
        )
        .map(|state| format!("P{state}")),
        field_errors,
    }
}

fn inspect_optional<T>(
    field_errors: &mut BTreeMap<String, String>,
    field_name: &'static str,
    result: Result<Option<T>, NvidiaError>,
) -> Option<T> {
    match result {
        Ok(value) => value,
        Err(error) => {
            field_errors.insert(field_name.to_string(), error.to_string());
            None
        }
    }
}

impl NvidiaNvmlCollector {
    pub fn new(config: NvidiaNvmlConfig) -> Self {
        match DynamicNvmlProvider::load(&config.library_paths) {
            Ok(provider) => Self {
                config,
                provider: Some(Box::new(provider)),
                load_error: None,
                errors_total: 0,
            },
            Err(error) => {
                tracing::debug!(%error, "NVIDIA NVML collector unavailable");
                Self {
                    config,
                    provider: None,
                    load_error: Some(error),
                    errors_total: 0,
                }
            }
        }
    }

    pub fn discover_summary(config: &NvidiaNvmlConfig) -> NvidiaNvmlDiscovery {
        if !config.enabled {
            return NvidiaNvmlDiscovery {
                enabled: false,
                supported: false,
                library_loaded: false,
                device_count: 0,
                status: "disabled",
                message: None,
            };
        }

        let provider = match DynamicNvmlProvider::load(&config.library_paths) {
            Ok(provider) => provider,
            Err(error) => {
                return NvidiaNvmlDiscovery {
                    enabled: true,
                    supported: false,
                    library_loaded: error.library_loaded(),
                    device_count: 0,
                    status: error.status(),
                    message: Some(error.to_string()),
                };
            }
        };

        match provider.device_count() {
            Ok(0) => NvidiaNvmlDiscovery {
                enabled: true,
                supported: true,
                library_loaded: true,
                device_count: 0,
                status: "no_devices",
                message: None,
            },
            Ok(device_count) => NvidiaNvmlDiscovery {
                enabled: true,
                supported: true,
                library_loaded: true,
                device_count,
                status: "available",
                message: None,
            },
            Err(error) => NvidiaNvmlDiscovery {
                enabled: true,
                supported: true,
                library_loaded: true,
                device_count: 0,
                status: error.status(),
                message: Some(error.to_string()),
            },
        }
    }

    #[cfg(test)]
    pub fn with_provider(
        config: NvidiaNvmlConfig,
        provider: impl NvidiaProvider + 'static,
    ) -> Self {
        Self {
            config,
            provider: Some(Box::new(provider)),
            load_error: None,
            errors_total: 0,
        }
    }

    #[cfg(test)]
    pub fn unavailable_for_test(config: NvidiaNvmlConfig, error: NvidiaError) -> Self {
        Self {
            config,
            provider: None,
            load_error: Some(error),
            errors_total: 0,
        }
    }
}

impl Collector for NvidiaNvmlCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        let Some(provider) = self.provider.as_mut() else {
            return unsupported_result(
                COLLECTOR_NAME,
                self.errors_total,
                self.load_error.as_ref(),
                started_at,
            );
        };

        if !provider.is_supported() {
            return unsupported_result(COLLECTOR_NAME, self.errors_total, None, started_at);
        }

        let device_count = match provider.device_count() {
            Ok(device_count) => device_count,
            Err(error) => {
                self.errors_total += 1;
                return CollectorResult {
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
                };
            }
        };

        let mut partial_errors = 0;
        let mut gpu_metrics = Vec::new();
        for index in 0..device_count {
            collect_gpu_metrics(
                provider.as_mut(),
                &self.config,
                index,
                &mut partial_errors,
                &mut gpu_metrics,
            );
        }
        self.errors_total += partial_errors;

        let mut metrics = collector_status_metrics(
            COLLECTOR_NAME,
            true,
            true,
            self.errors_total,
            Some(unix_timestamp_seconds()),
        );
        metrics.extend(gpu_metrics);

        CollectorResult::success(COLLECTOR_NAME, metrics, started_at)
    }
}

fn unsupported_result(
    collector: &'static str,
    errors_total: u64,
    error: Option<&NvidiaError>,
    started_at: Instant,
) -> CollectorResult {
    CollectorResult {
        collector,
        success: true,
        metrics: collector_status_metrics(collector, false, false, errors_total, None),
        error_message: error.map(ToString::to_string),
        duration: started_at.elapsed(),
    }
}

fn collect_gpu_metrics(
    provider: &mut dyn NvidiaProvider,
    config: &NvidiaNvmlConfig,
    index: u32,
    partial_errors: &mut u64,
    metrics: &mut Vec<MetricSample>,
) {
    match provider.device_info(index) {
        Ok(info) => metrics.push(gpu_info_metric(config, &info)),
        Err(error) => record_partial_error(partial_errors, index, "device_info", error),
    }

    match provider.temperature_celsius(index) {
        Ok(Some(temperature)) => metrics.push(gpu_temperature_metric(index, temperature)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "temperature", error),
    }

    match provider.utilization(index) {
        Ok(Some(utilization)) => metrics.extend(gpu_utilization_metrics(index, utilization)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "utilization", error),
    }

    match provider.memory(index) {
        Ok(Some(memory)) => metrics.extend(gpu_memory_metrics(index, memory)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "memory", error),
    }

    match provider.power_usage_milliwatts(index) {
        Ok(Some(milliwatts)) => metrics.push(gpu_power_usage_metric(index, milliwatts)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "power_usage", error),
    }

    match provider.power_limit_milliwatts(index) {
        Ok(Some(milliwatts)) => metrics.push(gpu_power_limit_metric(index, milliwatts)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "power_limit", error),
    }

    match provider.graphics_clock_mhz(index) {
        Ok(Some(mhz)) => metrics.push(gpu_clock_metric(index, "graphics", mhz)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "graphics_clock", error),
    }

    match provider.memory_clock_mhz(index) {
        Ok(Some(mhz)) => metrics.push(gpu_clock_metric(index, "memory", mhz)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "memory_clock", error),
    }

    match provider.performance_state(index) {
        Ok(Some(state)) => metrics.push(gpu_performance_state_metric(index, state)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "performance_state", error),
    }

    match provider.current_clocks_throttle_reasons(index) {
        Ok(Some(reasons)) => metrics.extend(gpu_throttle_reason_metrics(index, reasons)),
        Ok(None) => {}
        Err(error) => record_partial_error(partial_errors, index, "throttle_reasons", error),
    }

    if config.fan_speed_enabled {
        match provider.fan_speed_ratio(index) {
            Ok(Some(fan_speed)) => metrics.push(gpu_fan_speed_metric(index, fan_speed)),
            Ok(None) => {}
            Err(error) => record_partial_error(partial_errors, index, "fan_speed", error),
        }
    }
}

fn record_partial_error(
    partial_errors: &mut u64,
    index: u32,
    operation: &'static str,
    error: NvidiaError,
) {
    *partial_errors += 1;
    tracing::debug!(
        collector = COLLECTOR_NAME,
        gpu_index = index,
        operation,
        %error,
        "NVIDIA GPU metric skipped"
    );
}

fn gpu_info_metric(config: &NvidiaNvmlConfig, info: &NvidiaDeviceInfo) -> MetricSample {
    let gpu_index = info.index.to_string();
    let device_id = gpu_device_id(info.index);
    let mut metric_labels = labels(&[
        ("component", "gpu"),
        ("device_id", device_id.as_str()),
        ("gpu_index", gpu_index.as_str()),
        ("vendor", "nvidia"),
        ("source", SOURCE),
        ("source_driver", "nvml"),
    ]);

    if config.expose_gpu_name {
        if let Some(name) = &info.name {
            metric_labels.insert("name".to_string(), name.clone());
        }
    }
    if config.expose_gpu_uuid {
        if let Some(uuid) = &info.uuid {
            metric_labels.insert("uuid".to_string(), uuid.clone());
        }
    }

    MetricSample::gauge(
        names::GPU_INFO,
        "Hardware device identity information.",
        metric_labels,
        1.0,
    )
}

fn gpu_device_id(index: u32) -> String {
    format!("gpu{index}")
}

fn gpu_temperature_metric(index: u32, temperature: f64) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Hardware temperature reading in degrees Celsius.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_edge_temp"),
            ("sensor_instance", "core"),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        temperature,
    )
}

fn gpu_utilization_metrics(index: u32, utilization: NvidiaUtilization) -> Vec<MetricSample> {
    vec![
        gpu_utilization_metric(index, "graphics", utilization.graphics_ratio),
        gpu_utilization_metric(index, "memory", utilization.memory_ratio),
    ]
}

fn gpu_utilization_metric(index: u32, engine: &str, value: f64) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    let sensor = if engine == "memory" {
        "gpu_memory_utilization"
    } else {
        "gpu_utilization"
    };
    MetricSample::gauge(
        names::GPU_UTILIZATION_RATIO,
        "Hardware utilization as a ratio from 0 to 1.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", sensor),
            ("engine", engine),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        value,
    )
}

fn gpu_memory_metrics(index: u32, memory: NvidiaMemory) -> Vec<MetricSample> {
    vec![
        gpu_memory_metric(index, "total", memory.total_bytes),
        gpu_memory_metric(index, "used", memory.used_bytes),
        gpu_memory_metric(index, "free", memory.free_bytes),
    ]
}

fn gpu_memory_metric(index: u32, state: &str, value: u64) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::GPU_MEMORY_TOTAL_BYTES,
        "Hardware memory bytes by state.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("memory", "vram"),
            ("state", state),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        value as f64,
    )
}

fn gpu_power_usage_metric(index: u32, milliwatts: u32) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::GPU_POWER_USAGE_WATTS,
        "Hardware power in watts.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_power"),
            ("sensor_instance", "current"),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        milliwatts as f64 / 1_000.0,
    )
}

fn gpu_power_limit_metric(index: u32, milliwatts: u32) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::GPU_POWER_LIMIT_WATTS,
        "Hardware power limit in watts.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_power_limit"),
            ("limit", "current"),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        milliwatts as f64 / 1_000.0,
    )
}

fn gpu_clock_metric(index: u32, clock: &str, mhz: u32) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    let sensor = match clock {
        "memory" => "gpu_memory_clock",
        "sm" => "gpu_sm_clock",
        "video" => "gpu_video_clock",
        _ => "gpu_core_clock",
    };
    MetricSample::gauge(
        names::GPU_CLOCK_HERTZ,
        "Hardware clock speed in hertz.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", sensor),
            ("clock", clock),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        mhz as f64 * 1_000_000.0,
    )
}

fn gpu_performance_state_metric(index: u32, state: u32) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::GPU_PERFORMANCE_STATE,
        "Hardware numeric state value.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_pstate"),
            ("state", "pstate"),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        state as f64,
    )
}

fn gpu_throttle_reason_metrics(index: u32, reasons: u64) -> Vec<MetricSample> {
    vec![
        gpu_throttle_reason_metric(index, "thermal", reasons & 0x0000_0000_0000_0060 != 0),
        gpu_throttle_reason_metric(index, "power", reasons & 0x0000_0000_0000_008c != 0),
        gpu_throttle_reason_metric(index, "other", reasons & 0x0000_0000_0000_0112 != 0),
    ]
}

fn gpu_throttle_reason_metric(index: u32, state: &str, active: bool) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::HARDWARE_STATE,
        "Hardware numeric state value.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_clock_throttle"),
            ("state", state),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        if active { 1.0 } else { 0.0 },
    )
}

fn gpu_fan_speed_metric(index: u32, fan_speed: f64) -> MetricSample {
    let gpu_index = index.to_string();
    let device_id = gpu_device_id(index);
    MetricSample::gauge(
        names::FAN_SPEED_RATIO,
        "Hardware fan speed as a ratio from 0 to 1.",
        labels(&[
            ("component", "gpu"),
            ("device_id", device_id.as_str()),
            ("gpu_index", gpu_index.as_str()),
            ("sensor", "gpu_fan_percent"),
            ("sensor_instance", "fan"),
            ("source", SOURCE),
            ("source_driver", "nvml"),
        ]),
        fan_speed,
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_provider::{TestNvidiaDevice, TestNvidiaProvider};
    use super::*;

    fn metric_value(result: &CollectorResult, name: &str, label: (&str, &str)) -> Option<f64> {
        metric_value_with_labels(result, name, &[label])
    }

    fn metric_value_with_labels(
        result: &CollectorResult,
        name: &str,
        expected_labels: &[(&str, &str)],
    ) -> Option<f64> {
        result
            .metrics
            .iter()
            .find(|metric| {
                metric.name == name
                    && expected_labels.iter().all(|(key, value)| {
                        metric
                            .labels
                            .get(*key)
                            .map(|actual| actual == value)
                            .unwrap_or(false)
                    })
            })
            .map(|metric| metric.value)
    }

    fn has_metric(result: &CollectorResult, name: &str) -> bool {
        result.metrics.iter().any(|metric| metric.name == name)
    }

    #[test]
    fn missing_nvml_emits_unsupported_status_without_failure() {
        let error = NvidiaError::LibraryNotFound {
            candidates: vec!["libnvidia-ml.so.1".to_string()],
            errors: vec!["not found".to_string()],
        };
        let mut collector =
            NvidiaNvmlCollector::unavailable_for_test(NvidiaNvmlConfig::default(), error);

        let result = collector.collect();

        assert!(result.success);
        assert_eq!(
            metric_value(
                &result,
                names::COLLECTOR_SUPPORTED,
                ("collector", COLLECTOR_NAME)
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(&result, names::COLLECTOR_UP, ("collector", COLLECTOR_NAME)),
            Some(0.0)
        );
    }

    #[test]
    fn no_devices_is_supported_and_up_without_gpu_metrics() {
        let mut collector = NvidiaNvmlCollector::with_provider(
            NvidiaNvmlConfig::default(),
            TestNvidiaProvider::new(Vec::new()),
        );

        let result = collector.collect();

        assert!(result.success);
        assert_eq!(
            metric_value(
                &result,
                names::COLLECTOR_SUPPORTED,
                ("collector", COLLECTOR_NAME)
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(&result, names::COLLECTOR_UP, ("collector", COLLECTOR_NAME)),
            Some(1.0)
        );
        assert!(!has_metric(&result, names::GPU_INFO));
    }

    #[test]
    fn one_gpu_emits_expected_metrics_without_uuid_by_default() {
        let mut collector = NvidiaNvmlCollector::with_provider(
            NvidiaNvmlConfig::default(),
            TestNvidiaProvider::one_gpu(),
        );

        let result = collector.collect();

        assert!(result.success);
        assert!(has_metric(&result, names::GPU_INFO));
        assert_eq!(
            metric_value(&result, names::TEMPERATURE_CELSIUS, ("gpu_index", "0")),
            Some(58.0)
        );
        assert_eq!(
            metric_value(
                &result,
                names::GPU_UTILIZATION_RATIO,
                ("engine", "graphics")
            ),
            Some(0.31)
        );
        assert_eq!(
            metric_value_with_labels(
                &result,
                names::GPU_MEMORY_USED_BYTES,
                &[("gpu_index", "0"), ("state", "used")]
            ),
            Some(2.0 * 1024.0 * 1024.0 * 1024.0)
        );
        assert_eq!(
            metric_value(&result, names::FAN_SPEED_RATIO, ("gpu_index", "0")),
            Some(0.42)
        );
        assert_eq!(
            metric_value(&result, names::GPU_POWER_USAGE_WATTS, ("gpu_index", "0")),
            Some(57.622)
        );
        assert_eq!(
            metric_value(&result, names::GPU_POWER_LIMIT_WATTS, ("gpu_index", "0")),
            Some(450.0)
        );
        assert_eq!(
            metric_value_with_labels(
                &result,
                names::GPU_CLOCK_HERTZ,
                &[("gpu_index", "0"), ("clock", "graphics")]
            ),
            Some(2_520_000_000.0)
        );
        assert_eq!(
            metric_value_with_labels(
                &result,
                names::GPU_CLOCK_HERTZ,
                &[("gpu_index", "0"), ("clock", "memory")]
            ),
            Some(10_501_000_000.0)
        );
        assert_eq!(
            metric_value(&result, names::GPU_PERFORMANCE_STATE, ("gpu_index", "0")),
            Some(0.0)
        );

        let info = result
            .metrics
            .iter()
            .find(|metric| metric.name == names::GPU_INFO)
            .unwrap();
        assert_eq!(
            info.labels.get("name").map(String::as_str),
            Some("Test NVIDIA GPU")
        );
        assert!(!info.labels.contains_key("uuid"));
    }

    #[test]
    fn uuid_label_is_opt_in() {
        let config = NvidiaNvmlConfig {
            expose_gpu_uuid: true,
            ..NvidiaNvmlConfig::default()
        };
        let mut collector =
            NvidiaNvmlCollector::with_provider(config, TestNvidiaProvider::one_gpu());

        let result = collector.collect();
        let info = result
            .metrics
            .iter()
            .find(|metric| metric.name == names::GPU_INFO)
            .unwrap();

        assert_eq!(
            info.labels.get("uuid").map(String::as_str),
            Some("GPU-test-uuid")
        );
    }

    #[test]
    fn fan_speed_can_be_disabled() {
        let config = NvidiaNvmlConfig {
            fan_speed_enabled: false,
            ..NvidiaNvmlConfig::default()
        };
        let mut collector =
            NvidiaNvmlCollector::with_provider(config, TestNvidiaProvider::one_gpu());

        let result = collector.collect();

        assert!(!has_metric(&result, names::FAN_SPEED_RATIO));
    }

    #[test]
    fn unsupported_optional_gpu_fields_are_skipped() {
        let provider = TestNvidiaProvider::new(vec![TestNvidiaDevice {
            info: NvidiaDeviceInfo {
                index: 0,
                name: Some("Minimal NVIDIA GPU".to_string()),
                uuid: None,
            },
            temperature_celsius: Some(50.0),
            utilization: None,
            memory: None,
            fan_speed_ratio: None,
            power_usage_milliwatts: None,
            power_limit_milliwatts: None,
            graphics_clock_mhz: None,
            memory_clock_mhz: None,
            performance_state: None,
            current_clocks_throttle_reasons: None,
        }]);
        let mut collector =
            NvidiaNvmlCollector::with_provider(NvidiaNvmlConfig::default(), provider);

        let result = collector.collect();

        assert!(result.success);
        assert!(has_metric(&result, names::GPU_INFO));
        assert_eq!(
            metric_value(&result, names::TEMPERATURE_CELSIUS, ("gpu_index", "0")),
            Some(50.0)
        );
        assert!(!has_metric(&result, names::GPU_POWER_USAGE_WATTS));
        assert!(!has_metric(&result, names::GPU_POWER_LIMIT_WATTS));
        assert!(!has_metric(&result, names::GPU_CLOCK_HERTZ));
        assert!(!has_metric(&result, names::GPU_PERFORMANCE_STATE));
    }

    #[test]
    fn inspect_provider_reports_available_test_gpu() {
        let config = NvidiaNvmlConfig::default();
        let mut provider = TestNvidiaProvider::one_gpu();

        let inspection = inspect_provider(&config, &mut provider, true);

        assert!(inspection.supported);
        assert_eq!(inspection.status, "available");
        assert_eq!(inspection.device_count, 1);
        assert_eq!(
            inspection.devices[0].name.as_deref(),
            Some("Test NVIDIA GPU")
        );
        assert_eq!(inspection.devices[0].temperature_celsius, Some(58.0));
    }
}
