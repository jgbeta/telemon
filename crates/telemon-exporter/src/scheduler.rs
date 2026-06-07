use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{debug, error};

use crate::adaptive::{self, AdaptiveInterval, AdaptiveSamplingState};
use crate::cache::SharedMetricCache;
use crate::diagnostics;
use crate::runtime_diagnostics::ExporterDiagnostics;
use telemon_collectors::traits::Collector;
use telemon_core::config::AdaptiveSamplingConfig;
use telemon_core::metrics::model::MetricSample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectorGroup {
    Dynamic,
    Static,
}

pub trait SamplingOverride: Send {
    fn forced_interval_seconds(&mut self) -> Option<u64>;
}

pub struct ScheduledCollector {
    collector: Box<dyn Collector>,
    interval: Duration,
    group: CollectorGroup,
    adaptive_interval: Option<AdaptiveInterval>,
}

impl ScheduledCollector {
    pub fn new(collector: Box<dyn Collector>, interval: Duration) -> Self {
        Self {
            collector,
            interval,
            group: CollectorGroup::Dynamic,
            adaptive_interval: None,
        }
    }

    pub fn static_collector(collector: Box<dyn Collector>, interval: Duration) -> Self {
        Self {
            collector,
            interval,
            group: CollectorGroup::Static,
            adaptive_interval: None,
        }
    }

    pub fn adaptive(mut self, default_interval_seconds: u64) -> Self {
        self.interval = Duration::from_secs(default_interval_seconds);
        self.adaptive_interval = Some(AdaptiveInterval::new(default_interval_seconds));
        self
    }
}

pub struct SchedulerRuntime {
    pub collectors: Vec<ScheduledCollector>,
    pub dynamic_cache: SharedMetricCache,
    pub static_cache: SharedMetricCache,
    pub adaptive_config: AdaptiveSamplingConfig,
    pub adaptive_state: AdaptiveSamplingState,
    pub sampling_override: Option<Box<dyn SamplingOverride>>,
    pub exporter_diagnostics: ExporterDiagnostics,
}

pub async fn run_scheduler(runtime: SchedulerRuntime, mut shutdown: watch::Receiver<bool>) {
    let SchedulerRuntime {
        mut collectors,
        dynamic_cache,
        static_cache,
        adaptive_config,
        adaptive_state,
        mut sampling_override,
        exporter_diagnostics,
    } = runtime;
    if collectors.is_empty() {
        if let Ok(mut cache) = static_cache.write() {
            cache.replace_snapshot(vec![diagnostics::build_info_metric()]);
        }
    }

    let mut next_due = vec![Instant::now(); collectors.len()];
    let mut latest_dynamic_by_collector: BTreeMap<&'static str, Vec<MetricSample>> =
        BTreeMap::new();
    let mut latest_static_by_collector: BTreeMap<&'static str, Vec<MetricSample>> = BTreeMap::new();
    adaptive_state.set_requested_interval_seconds(adaptive_config.levels.normal_seconds);
    exporter_diagnostics
        .record_requested_interval(adaptive_config.levels.normal_seconds, "scheduler_startup");

    loop {
        if *shutdown.borrow() {
            break;
        }

        let now = Instant::now();
        let forced_interval_seconds = sampling_override
            .as_mut()
            .and_then(|override_source| override_source.forced_interval_seconds());
        if let Some(interval_seconds) = forced_interval_seconds {
            let forced_due = now + Duration::from_secs(interval_seconds);
            for (index, scheduled) in collectors.iter().enumerate() {
                if scheduled.group == CollectorGroup::Dynamic && next_due[index] > forced_due {
                    next_due[index] = now;
                }
            }
        }
        let mut changed = false;

        for (index, scheduled) in collectors.iter_mut().enumerate() {
            if now >= next_due[index] {
                let scheduled_due = next_due[index];
                let collector_name = scheduled.collector.name();
                exporter_diagnostics.record_scheduler_lag(
                    collector_name,
                    now.duration_since(scheduled_due).as_secs_f64(),
                );
                let result = scheduled.collector.collect();
                if result.success {
                    debug!(
                        collector = result.collector,
                        duration_ms = result.duration.as_millis(),
                        "collector completed"
                    );
                } else {
                    error!(
                        collector = result.collector,
                        duration_ms = result.duration.as_millis(),
                        error = result.error_message.as_deref().unwrap_or("unknown"),
                        "collector failed"
                    );
                }

                let metrics = result.metrics;
                if let Some(interval) = &mut scheduled.adaptive_interval {
                    let desired = adaptive::evaluate_requested_interval_seconds(
                        &adaptive_config,
                        metrics.as_slice(),
                    );
                    let current = interval.update(
                        desired,
                        Duration::from_secs(adaptive_config.cooldown_seconds),
                    );
                    scheduled.interval = Duration::from_secs(current);
                }

                let (dynamic_metrics, static_metrics) = match scheduled.group {
                    CollectorGroup::Dynamic => split_static_metrics(metrics),
                    CollectorGroup::Static => (Vec::new(), metrics),
                };

                match scheduled.group {
                    CollectorGroup::Dynamic => {
                        latest_dynamic_by_collector.insert(result.collector, dynamic_metrics);
                        latest_static_by_collector.insert(result.collector, static_metrics);
                    }
                    CollectorGroup::Static => {
                        latest_static_by_collector.insert(result.collector, static_metrics);
                    }
                }

                let effective_interval = forced_interval_seconds
                    .filter(|_| scheduled.group == CollectorGroup::Dynamic)
                    .map(Duration::from_secs)
                    .unwrap_or(scheduled.interval);
                next_due[index] = now + effective_interval;
                changed = true;
            }
        }

        if changed {
            let requested_interval = requested_interval_seconds(
                collectors.as_slice(),
                &adaptive_config,
                forced_interval_seconds,
            );
            let interval_source = if forced_interval_seconds.is_some() {
                "sampling_override"
            } else {
                "adaptive_or_base"
            };
            exporter_diagnostics.record_requested_interval(requested_interval, interval_source);
            adaptive_state.set_requested_interval_seconds(requested_interval);
            let mut dynamic_snapshot = vec![adaptive::requested_scrape_interval_metric(
                adaptive_state.requested_interval_seconds(),
            )];
            for metrics in latest_dynamic_by_collector.values() {
                dynamic_snapshot.extend(metrics.clone());
            }

            let mut static_snapshot = vec![diagnostics::build_info_metric()];
            for metrics in latest_static_by_collector.values() {
                static_snapshot.extend(metrics.clone());
            }

            if let Ok(mut cache) = dynamic_cache.write() {
                cache.replace_snapshot(dynamic_snapshot);
            }
            if let Ok(mut cache) = static_cache.write() {
                cache.replace_snapshot(static_snapshot);
            }
        }

        let delay = next_due
            .iter()
            .min()
            .map(|next| next.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|| Duration::from_secs(60))
            .min(Duration::from_secs(1));

        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

pub fn collect_snapshot_once(
    collectors: &mut [ScheduledCollector],
    adaptive_config: &AdaptiveSamplingConfig,
) -> Vec<MetricSample> {
    let mut dynamic_snapshot = vec![adaptive::requested_scrape_interval_metric(
        adaptive_config.levels.normal_seconds,
    )];
    let mut static_snapshot = vec![diagnostics::build_info_metric()];

    for scheduled in collectors {
        let result = scheduled.collector.collect();
        if !result.success {
            error!(
                collector = result.collector,
                error = result.error_message.as_deref().unwrap_or("unknown"),
                "collector failed"
            );
        }
        let mut metrics = result.metrics;
        if scheduled.adaptive_interval.is_some() {
            let requested =
                adaptive::evaluate_requested_interval_seconds(adaptive_config, metrics.as_slice());
            dynamic_snapshot[0] = adaptive::requested_scrape_interval_metric(requested);
        }

        match scheduled.group {
            CollectorGroup::Dynamic => {
                let (mut dynamic_metrics, mut static_metrics) = split_static_metrics(metrics);
                dynamic_snapshot.append(&mut dynamic_metrics);
                static_snapshot.append(&mut static_metrics);
            }
            CollectorGroup::Static => static_snapshot.append(&mut metrics),
        }
    }

    let mut snapshot = dynamic_snapshot;
    snapshot.extend(static_snapshot);
    snapshot
}

fn split_static_metrics(metrics: Vec<MetricSample>) -> (Vec<MetricSample>, Vec<MetricSample>) {
    let mut dynamic_metrics = Vec::new();
    let mut static_metrics = Vec::new();

    for metric in metrics {
        if is_static_metric(&metric) {
            static_metrics.push(metric);
        } else {
            dynamic_metrics.push(metric);
        }
    }

    (dynamic_metrics, static_metrics)
}

fn is_static_metric(metric: &MetricSample) -> bool {
    use telemon_core::metrics::names;

    let name = metric.name.as_str();
    if name == names::COLLECTOR_SUPPORTED
        || name == names::CPU_INFO
        || name == names::COMPUTER_SYSTEM_INFO
        || name == names::GPU_POWER_LIMIT_WATTS
        || name == names::HARDWARE_CLOCK_AVAILABLE_HERTZ
        || name == names::HARDWARE_CPU_CLUSTER_CORES
        || name == names::HARDWARE_DEVICE_INFO
        || name == names::HARDWARE_GPU_CORES
        || name == names::HARDWARE_SENSOR_INFO
        || name == names::STORAGE_DEVICE_INFO
        || name == names::STORAGE_NAMESPACE_CAPACITY_BYTES
        || name == names::SYSTEM_CPU_COUNT
        || name == names::SYSTEM_OS_INFO
        || name == names::TEMPERATURE_LIMIT_CELSIUS
    {
        return true;
    }

    if name == names::MEMORY_TOTAL_BYTES {
        return metric.labels.get("state").map(String::as_str) == Some("total");
    }

    if name == names::SYSTEM_SWAP_BYTES {
        return metric.labels.get("state").map(String::as_str) == Some("total");
    }

    if name == names::FILESYSTEM_SIZE_BYTES {
        return metric.labels.get("state").map(String::as_str) == Some("size");
    }

    if name == names::GPU_MEMORY_TOTAL_BYTES {
        return metric.labels.get("state").map(String::as_str) == Some("total");
    }

    false
}

fn requested_interval_seconds(
    collectors: &[ScheduledCollector],
    adaptive_config: &AdaptiveSamplingConfig,
    forced_interval_seconds: Option<u64>,
) -> u64 {
    if let Some(interval_seconds) = forced_interval_seconds {
        return interval_seconds;
    }
    if !adaptive_config.enabled {
        return adaptive_config.levels.normal_seconds;
    }

    collectors
        .iter()
        .filter_map(|collector| {
            collector
                .adaptive_interval
                .as_ref()
                .map(AdaptiveInterval::current_seconds)
        })
        .min()
        .unwrap_or(adaptive_config.levels.normal_seconds)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use telemon_collectors::traits::{Collector, CollectorResult};

    use super::*;

    struct FailingCollector;

    impl Collector for FailingCollector {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn collect(&mut self) -> CollectorResult {
            CollectorResult::failure(self.name(), "expected failure", 1, Instant::now())
        }
    }

    #[test]
    fn forced_interval_overrides_adaptive_request() {
        let config = AdaptiveSamplingConfig::default();
        let collectors =
            vec![
                ScheduledCollector::new(Box::new(FailingCollector), Duration::from_secs(15))
                    .adaptive(15),
            ];

        assert_eq!(requested_interval_seconds(&collectors, &config, Some(1)), 1);
    }

    #[test]
    fn failed_collector_does_not_prevent_snapshot() {
        let mut collectors = vec![ScheduledCollector::new(
            Box::new(FailingCollector),
            Duration::from_secs(1),
        )];

        let snapshot = collect_snapshot_once(&mut collectors, &AdaptiveSamplingConfig::default());

        assert!(snapshot
            .iter()
            .any(|sample| sample.name == telemon_core::metrics::names::BUILD_INFO));
        assert!(snapshot
            .iter()
            .any(|sample| sample.name == telemon_core::metrics::names::COLLECTOR_UP));
    }

    #[test]
    fn storage_identity_and_capacity_are_static() {
        let storage_info = MetricSample::gauge(
            telemon_core::metrics::names::STORAGE_DEVICE_INFO,
            "storage info",
            telemon_core::metrics::model::labels(&[]),
            1.0,
        );
        let namespace_capacity = MetricSample::gauge(
            telemon_core::metrics::names::STORAGE_NAMESPACE_CAPACITY_BYTES,
            "storage capacity",
            telemon_core::metrics::model::labels(&[]),
            512.0,
        );

        assert!(is_static_metric(&storage_info));
        assert!(is_static_metric(&namespace_capacity));
    }

    #[test]
    fn windows_inventory_and_stable_baseline_metrics_are_static() {
        let os_info = MetricSample::gauge(
            telemon_core::metrics::names::WINDOWS_OS_INFO,
            "windows os info",
            telemon_core::metrics::model::labels(&[]),
            1.0,
        );
        let cpu_info = MetricSample::gauge(
            telemon_core::metrics::names::CPU_INFO,
            "cpu info",
            telemon_core::metrics::model::labels(&[]),
            1.0,
        );
        let computer_info = MetricSample::gauge(
            telemon_core::metrics::names::COMPUTER_SYSTEM_INFO,
            "computer info",
            telemon_core::metrics::model::labels(&[]),
            1.0,
        );
        let memory_total = MetricSample::gauge(
            telemon_core::metrics::names::MEMORY_TOTAL_BYTES,
            "memory total",
            telemon_core::metrics::model::labels(&[("state", "total")]),
            16.0,
        );
        let memory_available = MetricSample::gauge(
            telemon_core::metrics::names::MEMORY_AVAILABLE_BYTES,
            "memory available",
            telemon_core::metrics::model::labels(&[("state", "available")]),
            6.0,
        );
        let filesystem_size = MetricSample::gauge(
            telemon_core::metrics::names::FILESYSTEM_SIZE_BYTES,
            "filesystem size",
            telemon_core::metrics::model::labels(&[("state", "size")]),
            512.0,
        );
        let filesystem_free = MetricSample::gauge(
            telemon_core::metrics::names::FILESYSTEM_FREE_BYTES,
            "filesystem free",
            telemon_core::metrics::model::labels(&[("state", "free")]),
            128.0,
        );

        assert!(is_static_metric(&os_info));
        assert!(is_static_metric(&cpu_info));
        assert!(is_static_metric(&computer_info));
        assert!(is_static_metric(&memory_total));
        assert!(is_static_metric(&filesystem_size));
        assert!(!is_static_metric(&memory_available));
        assert!(!is_static_metric(&filesystem_free));
    }

    #[test]
    fn system_memory_total_and_cpu_count_are_static() {
        let memory_total = MetricSample::gauge(
            telemon_core::metrics::names::MEMORY_TOTAL_BYTES,
            "memory total",
            telemon_core::metrics::model::labels(&[("state", "total")]),
            16.0,
        );
        let memory_available = MetricSample::gauge(
            telemon_core::metrics::names::MEMORY_AVAILABLE_BYTES,
            "memory available",
            telemon_core::metrics::model::labels(&[("state", "available")]),
            6.0,
        );
        let cpu_count = MetricSample::gauge(
            telemon_core::metrics::names::SYSTEM_CPU_COUNT,
            "cpu count",
            telemon_core::metrics::model::labels(&[]),
            10.0,
        );
        let uptime = MetricSample::gauge(
            telemon_core::metrics::names::UPTIME_SECONDS,
            "uptime",
            telemon_core::metrics::model::labels(&[]),
            12_345.0,
        );

        assert!(is_static_metric(&memory_total));
        assert!(is_static_metric(&cpu_count));
        assert!(!is_static_metric(&memory_available));
        assert!(!is_static_metric(&uptime));
    }

    #[test]
    fn gpu_power_limit_is_static_but_usage_is_dynamic() {
        let power_limit = MetricSample::gauge(
            telemon_core::metrics::names::GPU_POWER_LIMIT_WATTS,
            "power limit",
            telemon_core::metrics::model::labels(&[]),
            450.0,
        );
        let power_usage = MetricSample::gauge(
            telemon_core::metrics::names::GPU_POWER_USAGE_WATTS,
            "power usage",
            telemon_core::metrics::model::labels(&[]),
            57.0,
        );

        assert!(is_static_metric(&power_limit));
        assert!(!is_static_metric(&power_usage));
    }
}
