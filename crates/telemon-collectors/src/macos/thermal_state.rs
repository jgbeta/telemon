use std::time::Instant;

use crate::macos::thermal_provider::{
    DefaultMacosThermalProvider, MacosThermalProvider, MacosThermalState,
};
#[cfg(target_os = "macos")]
use crate::traits::{collector_health_metrics, unix_timestamp_seconds};
use crate::traits::{collector_status_metrics, Collector, CollectorResult};
use telemon_core::config::MacosThermalStateConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const THERMAL_STATE_COLLECTOR_NAME: &str = "macos_thermal_state";
pub const SOURCE: &str = "macos_processinfo";

pub struct MacosThermalStateCollector {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    provider: Box<dyn MacosThermalProvider>,
    errors_total: u64,
}

impl MacosThermalStateCollector {
    pub fn new(config: MacosThermalStateConfig) -> Self {
        let _ = config;
        Self {
            provider: Box::new(DefaultMacosThermalProvider::new()),
            errors_total: 0,
        }
    }

    pub fn with_provider(provider: impl MacosThermalProvider + 'static) -> Self {
        Self {
            provider: Box::new(provider),
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &MacosThermalStateConfig) -> String {
        if !config.enabled {
            return "disabled".to_string();
        }
        if cfg!(target_os = "macos") {
            "available".to_string()
        } else {
            "unsupported on this OS".to_string()
        }
    }
}

impl Collector for MacosThermalStateCollector {
    fn name(&self) -> &'static str {
        THERMAL_STATE_COLLECTOR_NAME
    }

    #[cfg(not(target_os = "macos"))]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        CollectorResult {
            collector: THERMAL_STATE_COLLECTOR_NAME,
            success: true,
            metrics: collector_status_metrics(
                THERMAL_STATE_COLLECTOR_NAME,
                false,
                false,
                self.errors_total,
                None,
            ),
            error_message: Some("macos_thermal_state is unsupported on this OS".to_string()),
            duration: started_at.elapsed(),
        }
    }

    #[cfg(target_os = "macos")]
    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match self.provider.thermal_state() {
            Ok(state) => {
                let mut metrics = collector_health_metrics(
                    THERMAL_STATE_COLLECTOR_NAME,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.extend(thermal_state_metrics(state));
                CollectorResult::success(THERMAL_STATE_COLLECTOR_NAME, metrics, started_at)
            }
            Err(error) => {
                self.errors_total += 1;
                CollectorResult {
                    collector: THERMAL_STATE_COLLECTOR_NAME,
                    success: false,
                    metrics: collector_status_metrics(
                        THERMAL_STATE_COLLECTOR_NAME,
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

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn thermal_state_metrics(active: MacosThermalState) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    for state in MacosThermalState::all() {
        metrics.push(MetricSample::gauge(
            names::SYSTEM_THERMAL_STATE,
            "macOS thermal state as one-hot gauges.",
            labels(&[("source", SOURCE), ("state", state.label())]),
            if state == active { 1.0 } else { 0.0 },
        ));
    }
    metrics.push(MetricSample::gauge(
        names::SYSTEM_THERMAL_STATE_VALUE,
        "macOS thermal state numeric value.",
        labels(&[("source", SOURCE)]),
        active.numeric_value(),
    ));
    metrics
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    struct FakeProvider {
        state: MacosThermalState,
    }

    impl MacosThermalProvider for FakeProvider {
        fn thermal_state(&self) -> Result<MacosThermalState> {
            Ok(self.state)
        }
    }

    #[test]
    fn one_hot_emits_all_states() {
        let metrics = thermal_state_metrics(MacosThermalState::Fair);
        let state_metrics = metrics
            .iter()
            .filter(|metric| metric.name == names::SYSTEM_THERMAL_STATE)
            .collect::<Vec<_>>();

        assert_eq!(state_metrics.len(), 5);
        for state in MacosThermalState::all() {
            assert!(state_metrics.iter().any(|metric| {
                metric.labels.get("state").map(String::as_str) == Some(state.label())
            }));
        }
    }

    #[test]
    fn one_hot_has_exactly_one_active_state() {
        let metrics = thermal_state_metrics(MacosThermalState::Serious);
        let active_count = metrics
            .iter()
            .filter(|metric| metric.name == names::SYSTEM_THERMAL_STATE && metric.value == 1.0)
            .count();

        assert_eq!(active_count, 1);
        assert!(metrics.iter().any(|metric| {
            metric.name == names::SYSTEM_THERMAL_STATE
                && metric.value == 1.0
                && metric.labels.get("state").map(String::as_str) == Some("serious")
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn collector_uses_provider_state() {
        let mut collector = MacosThermalStateCollector::with_provider(FakeProvider {
            state: MacosThermalState::Critical,
        });
        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::SYSTEM_THERMAL_STATE_VALUE && metric.value == 3.0
        }));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_collector_reports_unsupported() {
        let mut collector = MacosThermalStateCollector::with_provider(FakeProvider {
            state: MacosThermalState::Critical,
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
            .any(|metric| { metric.name == names::COLLECTOR_UP && metric.value == 0.0 }));
    }
}
