use std::time::Instant;

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
    let mut metric_labels = labels(&[
        ("gpu_index", gpu_index.as_str()),
        ("vendor", "nvidia"),
        ("source", SOURCE),
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
        "NVIDIA GPU identity information.",
        metric_labels,
        1.0,
    )
}

fn gpu_temperature_metric(index: u32, temperature: f64) -> MetricSample {
    let gpu_index = index.to_string();
    MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Temperature reading in degrees Celsius.",
        labels(&[
            ("component", "gpu"),
            ("sensor", "core"),
            ("source", SOURCE),
            ("gpu_index", gpu_index.as_str()),
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
    MetricSample::gauge(
        names::GPU_UTILIZATION_RATIO,
        "NVIDIA GPU utilization as a ratio from 0 to 1.",
        labels(&[
            ("gpu_index", gpu_index.as_str()),
            ("source", SOURCE),
            ("engine", engine),
        ]),
        value,
    )
}

fn gpu_memory_metrics(index: u32, memory: NvidiaMemory) -> Vec<MetricSample> {
    let gpu_index = index.to_string();
    let metric_labels = labels(&[("gpu_index", gpu_index.as_str()), ("source", SOURCE)]);

    vec![
        MetricSample::gauge(
            names::GPU_MEMORY_TOTAL_BYTES,
            "NVIDIA GPU total memory in bytes.",
            metric_labels.clone(),
            memory.total_bytes as f64,
        ),
        MetricSample::gauge(
            names::GPU_MEMORY_USED_BYTES,
            "NVIDIA GPU used memory in bytes.",
            metric_labels.clone(),
            memory.used_bytes as f64,
        ),
        MetricSample::gauge(
            names::GPU_MEMORY_FREE_BYTES,
            "NVIDIA GPU free memory in bytes.",
            metric_labels,
            memory.free_bytes as f64,
        ),
    ]
}

fn gpu_fan_speed_metric(index: u32, fan_speed: f64) -> MetricSample {
    let gpu_index = index.to_string();
    MetricSample::gauge(
        names::FAN_SPEED_RATIO,
        "Fan speed as a ratio from 0 to 1.",
        labels(&[
            ("component", "gpu"),
            ("gpu_index", gpu_index.as_str()),
            ("source", SOURCE),
        ]),
        fan_speed,
    )
}

#[cfg(test)]
mod tests {
    use super::super::fake_provider::FakeNvidiaProvider;
    use super::*;

    fn metric_value(result: &CollectorResult, name: &str, label: (&str, &str)) -> Option<f64> {
        result
            .metrics
            .iter()
            .find(|metric| {
                metric.name == name
                    && metric
                        .labels
                        .get(label.0)
                        .map(|value| value == label.1)
                        .unwrap_or(false)
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
            FakeNvidiaProvider::new(Vec::new()),
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
            FakeNvidiaProvider::one_gpu(),
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
            metric_value(&result, names::GPU_MEMORY_USED_BYTES, ("gpu_index", "0")),
            Some(2.0 * 1024.0 * 1024.0 * 1024.0)
        );
        assert_eq!(
            metric_value(&result, names::FAN_SPEED_RATIO, ("gpu_index", "0")),
            Some(0.42)
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
            NvidiaNvmlCollector::with_provider(config, FakeNvidiaProvider::one_gpu());

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
            NvidiaNvmlCollector::with_provider(config, FakeNvidiaProvider::one_gpu());

        let result = collector.collect();

        assert!(!has_metric(&result, names::FAN_SPEED_RATIO));
    }
}
