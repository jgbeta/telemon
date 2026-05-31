use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use telemon_core::config::DiagnosticsConfig;
use telemon_core::metrics::model::MetricSample;
use telemon_core::metrics::names;
use tracing::{info, warn};

use crate::cache::MetricCacheMetadata;

#[derive(Debug, Clone)]
pub struct ExporterDiagnostics {
    config: DiagnosticsConfig,
    inner: Arc<Mutex<DiagnosticsInner>>,
}

#[derive(Debug, Default)]
struct DiagnosticsInner {
    scrape_requests_total: BTreeMap<(String, u16), u64>,
    scrape_last_request_timestamp_seconds: BTreeMap<String, u64>,
    scrape_request_gap_seconds: BTreeMap<String, u64>,
    scrape_gaps_total: BTreeMap<String, u64>,
    current_requested_scrape_interval_seconds: Option<u64>,
    requested_scrape_interval_changes_total: BTreeMap<(u64, u64), u64>,
    requested_scrape_interval_last_change_timestamp_seconds: Option<u64>,
}

impl ExporterDiagnostics {
    pub fn new(config: DiagnosticsConfig) -> Self {
        Self {
            config,
            inner: Arc::new(Mutex::new(DiagnosticsInner::default())),
        }
    }

    pub fn disabled() -> Self {
        let config = DiagnosticsConfig {
            enabled: false,
            ..DiagnosticsConfig::default()
        };
        Self::new(config)
    }

    pub fn record_scrape(&self, endpoint: &str, status: u16, gap_threshold_seconds: u64) {
        self.record_scrape_at(
            endpoint,
            status,
            gap_threshold_seconds,
            unix_timestamp_seconds(),
        );
    }

    fn record_scrape_at(&self, endpoint: &str, status: u16, gap_threshold_seconds: u64, now: u64) {
        if !self.config.enabled {
            return;
        }

        let mut inner = self.inner.lock().expect("diagnostics mutex poisoned");
        let key = (endpoint.to_string(), status);
        *inner.scrape_requests_total.entry(key).or_insert(0) += 1;

        if let Some(previous) = inner
            .scrape_last_request_timestamp_seconds
            .insert(endpoint.to_string(), now)
        {
            let gap = now.saturating_sub(previous);
            inner
                .scrape_request_gap_seconds
                .insert(endpoint.to_string(), gap);
            let threshold_seconds =
                gap_threshold_seconds.max(self.config.scrape_gap_threshold_seconds);
            if gap >= threshold_seconds {
                let total = inner
                    .scrape_gaps_total
                    .entry(endpoint.to_string())
                    .or_insert(0);
                *total += 1;
                if self.config.log_scrape_gaps {
                    warn!(
                        endpoint,
                        status,
                        gap_seconds = gap,
                        threshold_seconds,
                        requested_scrape_interval_seconds = inner
                            .current_requested_scrape_interval_seconds
                            .unwrap_or_default(),
                        "exporter scrape request gap observed"
                    );
                }
            }
        } else {
            inner
                .scrape_request_gap_seconds
                .insert(endpoint.to_string(), 0);
        }
    }

    pub fn record_requested_interval(&self, interval_seconds: u64, source: &'static str) {
        self.record_requested_interval_at(interval_seconds, source, unix_timestamp_seconds());
    }

    fn record_requested_interval_at(&self, interval_seconds: u64, source: &'static str, now: u64) {
        if !self.config.enabled {
            return;
        }

        let mut inner = self.inner.lock().expect("diagnostics mutex poisoned");
        if let Some(previous) = inner.current_requested_scrape_interval_seconds {
            if previous != interval_seconds {
                *inner
                    .requested_scrape_interval_changes_total
                    .entry((previous, interval_seconds))
                    .or_insert(0) += 1;
                inner.requested_scrape_interval_last_change_timestamp_seconds = Some(now);
                if self.config.log_scrape_interval_changes {
                    info!(
                        previous_interval_seconds = previous,
                        requested_interval_seconds = interval_seconds,
                        source,
                        "exporter requested scrape interval changed"
                    );
                }
            }
        }
        inner.current_requested_scrape_interval_seconds = Some(interval_seconds);
    }

    pub fn record_scheduler_lag(&self, collector: &'static str, lag_seconds: f64) {
        if !self.config.enabled || !self.config.log_scheduler_lag {
            return;
        }
        if lag_seconds >= self.config.scheduler_lag_threshold_seconds as f64 {
            warn!(
                collector,
                lag_seconds,
                threshold_seconds = self.config.scheduler_lag_threshold_seconds,
                "collector scheduler lag observed"
            );
        }
    }

    pub fn metrics(
        &self,
        dynamic_metadata: MetricCacheMetadata,
        static_metadata: MetricCacheMetadata,
    ) -> Vec<MetricSample> {
        if !self.config.enabled {
            return Vec::new();
        }

        let inner = self.inner.lock().expect("diagnostics mutex poisoned");
        let mut metrics = Vec::new();
        metrics.extend(snapshot_metrics("dynamic", dynamic_metadata));
        metrics.extend(snapshot_metrics("static", static_metadata));

        for ((endpoint, status), value) in &inner.scrape_requests_total {
            metrics.push(MetricSample::counter(
                names::SCRAPE_REQUESTS_TOTAL,
                "Total scrape requests handled by the exporter.",
                labels(&[
                    ("endpoint", endpoint.as_str()),
                    ("status", &status.to_string()),
                ]),
                *value as f64,
            ));
        }
        for (endpoint, value) in &inner.scrape_last_request_timestamp_seconds {
            metrics.push(MetricSample::gauge(
                names::SCRAPE_LAST_REQUEST_TIMESTAMP_SECONDS,
                "Unix timestamp of the last scrape request seen by the exporter.",
                labels(&[("endpoint", endpoint.as_str())]),
                *value as f64,
            ));
        }
        for (endpoint, value) in &inner.scrape_request_gap_seconds {
            metrics.push(MetricSample::gauge(
                names::SCRAPE_REQUEST_GAP_SECONDS,
                "Seconds between the two most recent scrape requests for an endpoint.",
                labels(&[("endpoint", endpoint.as_str())]),
                *value as f64,
            ));
        }
        for (endpoint, value) in &inner.scrape_gaps_total {
            metrics.push(MetricSample::counter(
                names::SCRAPE_GAPS_TOTAL,
                "Total scrape request gaps exceeding the configured threshold.",
                labels(&[("endpoint", endpoint.as_str())]),
                *value as f64,
            ));
        }
        for ((from, to), value) in &inner.requested_scrape_interval_changes_total {
            metrics.push(MetricSample::counter(
                names::REQUESTED_SCRAPE_INTERVAL_CHANGES_TOTAL,
                "Total exporter requested scrape interval changes.",
                labels(&[("from", &from.to_string()), ("to", &to.to_string())]),
                *value as f64,
            ));
        }
        if let Some(value) = inner.requested_scrape_interval_last_change_timestamp_seconds {
            metrics.push(MetricSample::gauge(
                names::REQUESTED_SCRAPE_INTERVAL_LAST_CHANGE_TIMESTAMP_SECONDS,
                "Unix timestamp of the last exporter requested scrape interval change.",
                labels(&[]),
                value as f64,
            ));
        }

        metrics
    }
}

fn snapshot_metrics(kind: &str, metadata: MetricCacheMetadata) -> Vec<MetricSample> {
    let mut metrics = vec![MetricSample::counter(
        names::SNAPSHOT_UPDATES_TOTAL,
        "Total exporter metric snapshot updates.",
        labels(&[("kind", kind)]),
        metadata.updates_total as f64,
    )];

    if let Some(timestamp) = metadata.updated_at_unix_seconds {
        metrics.push(MetricSample::gauge(
            names::SNAPSHOT_LAST_UPDATE_TIMESTAMP_SECONDS,
            "Unix timestamp of the last exporter metric snapshot update.",
            labels(&[("kind", kind)]),
            timestamp as f64,
        ));
    }
    if let Some(age_seconds) = metadata.age_seconds() {
        metrics.push(MetricSample::gauge(
            names::SNAPSHOT_AGE_SECONDS,
            "Age in seconds of the current exporter metric snapshot.",
            labels(&[("kind", kind)]),
            age_seconds,
        ));
    }

    metrics
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn metric_value(metrics: &[MetricSample], name: &str, pairs: &[(&str, &str)]) -> Option<f64> {
        let labels = labels(pairs);
        metrics
            .iter()
            .find(|metric| metric.name == name && metric.labels == labels)
            .map(|metric| metric.value)
    }

    #[test]
    fn scrape_gap_counter_increments_above_threshold() {
        let config = DiagnosticsConfig {
            scrape_gap_threshold_seconds: 30,
            log_scrape_gaps: false,
            ..DiagnosticsConfig::default()
        };
        let diagnostics = ExporterDiagnostics::new(config);
        diagnostics.record_scrape_at("/metrics", 200, 30, 100);
        diagnostics.record_scrape_at("/metrics", 200, 30, 129);
        diagnostics.record_scrape_at("/metrics", 200, 30, 160);

        let metrics = diagnostics.metrics(
            MetricCacheMetadata::default(),
            MetricCacheMetadata::default(),
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SCRAPE_REQUEST_GAP_SECONDS,
                &[("endpoint", "/metrics")]
            ),
            Some(31.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SCRAPE_GAPS_TOTAL,
                &[("endpoint", "/metrics")]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SCRAPE_REQUESTS_TOTAL,
                &[("endpoint", "/metrics"), ("status", "200")]
            ),
            Some(3.0)
        );
    }

    #[test]
    fn endpoint_specific_threshold_prevents_static_false_gap() {
        let config = DiagnosticsConfig {
            scrape_gap_threshold_seconds: 30,
            log_scrape_gaps: false,
            ..DiagnosticsConfig::default()
        };
        let diagnostics = ExporterDiagnostics::new(config);
        diagnostics.record_scrape_at("/metrics/static", 200, 600, 100);
        diagnostics.record_scrape_at("/metrics/static", 200, 600, 400);

        let metrics = diagnostics.metrics(
            MetricCacheMetadata::default(),
            MetricCacheMetadata::default(),
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SCRAPE_REQUEST_GAP_SECONDS,
                &[("endpoint", "/metrics/static")]
            ),
            Some(300.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SCRAPE_GAPS_TOTAL,
                &[("endpoint", "/metrics/static")]
            ),
            None
        );
    }

    #[test]
    fn requested_interval_changes_are_counted() {
        let config = DiagnosticsConfig {
            log_scrape_interval_changes: false,
            ..DiagnosticsConfig::default()
        };
        let diagnostics = ExporterDiagnostics::new(config);
        diagnostics.record_requested_interval_at(15, "startup", 10);
        diagnostics.record_requested_interval_at(1, "forced", 20);
        diagnostics.record_requested_interval_at(1, "forced", 21);
        diagnostics.record_requested_interval_at(15, "adaptive", 40);

        let metrics = diagnostics.metrics(
            MetricCacheMetadata::default(),
            MetricCacheMetadata::default(),
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::REQUESTED_SCRAPE_INTERVAL_CHANGES_TOTAL,
                &[("from", "15"), ("to", "1")]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::REQUESTED_SCRAPE_INTERVAL_CHANGES_TOTAL,
                &[("from", "1"), ("to", "15")]
            ),
            Some(1.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::REQUESTED_SCRAPE_INTERVAL_LAST_CHANGE_TIMESTAMP_SECONDS,
                &[]
            ),
            Some(40.0)
        );
    }

    #[test]
    fn snapshot_metrics_include_metadata() {
        let diagnostics = ExporterDiagnostics::new(DiagnosticsConfig::default());
        let metadata = MetricCacheMetadata {
            updated_at: Some(Instant::now() - Duration::from_secs(2)),
            updated_at_unix_seconds: Some(1234),
            updates_total: 7,
        };

        let metrics = diagnostics.metrics(metadata, MetricCacheMetadata::default());
        assert_eq!(
            metric_value(
                &metrics,
                names::SNAPSHOT_LAST_UPDATE_TIMESTAMP_SECONDS,
                &[("kind", "dynamic")]
            ),
            Some(1234.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SNAPSHOT_UPDATES_TOTAL,
                &[("kind", "dynamic")]
            ),
            Some(7.0)
        );
        assert!(metric_value(
            &metrics,
            names::SNAPSHOT_AGE_SECONDS,
            &[("kind", "dynamic")]
        )
        .is_some_and(|value| value >= 2.0));
    }
}
