use std::time::Instant;

use crate::traits::{collector_health_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

#[derive(Debug, Default)]
pub struct FakeCollector {
    errors_total: u64,
}

impl FakeCollector {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Collector for FakeCollector {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        let mut metrics = collector_health_metrics(
            self.name(),
            true,
            self.errors_total,
            Some(unix_timestamp_seconds()),
        );

        metrics.push(MetricSample::gauge(
            names::TEMPERATURE_CELSIUS,
            "Temperature reading in degrees Celsius.",
            labels(&[
                ("component", "test"),
                ("sensor", "fake"),
                ("source", "fake"),
            ]),
            42.0,
        ));

        CollectorResult::success(self.name(), metrics, started_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_collector_emits_expected_metric_names() {
        let mut collector = FakeCollector::new();
        let result = collector.collect();
        let names: Vec<_> = result
            .metrics
            .iter()
            .map(|metric| metric.name.as_str())
            .collect();

        assert!(result.success);
        assert!(names.contains(&names::COLLECTOR_SUPPORTED));
        assert!(names.contains(&names::COLLECTOR_UP));
        assert!(names.contains(&names::COLLECTOR_ERRORS_TOTAL));
        assert!(names.contains(&names::COLLECTOR_LAST_SUCCESS_TIMESTAMP_SECONDS));
        assert!(names.contains(&names::TEMPERATURE_CELSIUS));
    }
}
