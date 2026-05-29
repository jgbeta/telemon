use std::time::Instant;

use crate::traits::{collector_status_metrics, Collector, CollectorResult};
use telemon_core::config::MacosExactTemperatureExperimentalConfig;

pub const COLLECTOR_NAME: &str = "macos_exact_temperature_experimental";

pub struct MacosExactTemperatureExperimentalCollector {
    config: MacosExactTemperatureExperimentalConfig,
    errors_total: u64,
}

impl MacosExactTemperatureExperimentalCollector {
    pub fn new(config: MacosExactTemperatureExperimentalConfig) -> Self {
        Self {
            config,
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &MacosExactTemperatureExperimentalConfig) -> String {
        if config.enabled {
            "not implemented".to_string()
        } else {
            "disabled".to_string()
        }
    }
}

impl Collector for MacosExactTemperatureExperimentalCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

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
            error_message: Some(
                "macos_exact_temperature_experimental is not implemented".to_string(),
            ),
            duration: started_at.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telemon_core::metrics::names;

    #[test]
    fn reports_unsupported_without_temperature_metrics() {
        let mut collector = MacosExactTemperatureExperimentalCollector::new(Default::default());
        let result = collector.collect();

        assert!(result.success);
        assert!(result
            .metrics
            .iter()
            .any(|metric| metric.name == names::COLLECTOR_SUPPORTED && metric.value == 0.0));
        assert!(result
            .metrics
            .iter()
            .all(|metric| metric.name != names::TEMPERATURE_CELSIUS));
    }
}
