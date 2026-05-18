use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub trait Collector: Send + Sync {
    fn name(&self) -> &'static str;
    fn collect(&mut self) -> CollectorResult;
}

#[derive(Debug, Clone)]
pub struct CollectorResult {
    pub collector: &'static str,
    pub success: bool,
    pub metrics: Vec<MetricSample>,
    pub error_message: Option<String>,
    pub duration: Duration,
}

impl CollectorResult {
    pub fn success(
        collector: &'static str,
        metrics: Vec<MetricSample>,
        started_at: Instant,
    ) -> Self {
        Self {
            collector,
            success: true,
            metrics,
            error_message: None,
            duration: started_at.elapsed(),
        }
    }

    pub fn failure(
        collector: &'static str,
        error_message: impl Into<String>,
        errors_total: u64,
        started_at: Instant,
    ) -> Self {
        Self {
            collector,
            success: false,
            metrics: collector_health_metrics(collector, false, errors_total, None),
            error_message: Some(error_message.into()),
            duration: started_at.elapsed(),
        }
    }
}

pub fn collector_health_metrics(
    collector: &'static str,
    up: bool,
    errors_total: u64,
    last_success_timestamp: Option<u64>,
) -> Vec<MetricSample> {
    collector_status_metrics(collector, true, up, errors_total, last_success_timestamp)
}

pub fn collector_status_metrics(
    collector: &'static str,
    supported: bool,
    up: bool,
    errors_total: u64,
    last_success_timestamp: Option<u64>,
) -> Vec<MetricSample> {
    let mut metrics = vec![
        MetricSample::gauge(
            names::COLLECTOR_SUPPORTED,
            "Whether a Telemon collector is supported on this host.",
            labels(&[("collector", collector)]),
            if supported { 1.0 } else { 0.0 },
        ),
        MetricSample::gauge(
            names::COLLECTOR_UP,
            "Whether a Telemon collector is currently healthy.",
            labels(&[("collector", collector)]),
            if up { 1.0 } else { 0.0 },
        ),
        MetricSample::counter(
            names::COLLECTOR_ERRORS_TOTAL,
            "Total collector errors observed by the exporter.",
            labels(&[("collector", collector)]),
            errors_total as f64,
        ),
    ];

    if let Some(timestamp) = last_success_timestamp {
        metrics.push(MetricSample::gauge(
            names::COLLECTOR_LAST_SUCCESS_TIMESTAMP_SECONDS,
            "Unix timestamp of the last successful collector run.",
            labels(&[("collector", collector)]),
            timestamp as f64,
        ));
    }

    metrics
}

pub fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
