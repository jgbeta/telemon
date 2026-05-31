use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telemon_core::metrics::model::MetricSample;

pub type SharedMetricCache = Arc<RwLock<MetricCache>>;

#[derive(Debug, Clone, Copy, Default)]
pub struct MetricCacheMetadata {
    pub updated_at: Option<Instant>,
    pub updated_at_unix_seconds: Option<u64>,
    pub updates_total: u64,
}

impl MetricCacheMetadata {
    pub fn age_seconds(self) -> Option<f64> {
        self.updated_at
            .map(|updated_at| updated_at.elapsed().as_secs_f64())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetricCache {
    metrics: Vec<MetricSample>,
    updated_at: Option<Instant>,
    updated_at_unix_seconds: Option<u64>,
    updates_total: u64,
}

impl MetricCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> SharedMetricCache {
        Arc::new(RwLock::new(Self::new()))
    }

    pub fn replace_snapshot(&mut self, metrics: Vec<MetricSample>) {
        self.metrics = metrics;
        self.updated_at = Some(Instant::now());
        self.updated_at_unix_seconds = Some(unix_timestamp_seconds());
        self.updates_total = self.updates_total.saturating_add(1);
    }

    pub fn snapshot(&self) -> Vec<MetricSample> {
        self.metrics.clone()
    }

    pub fn has_snapshot(&self) -> bool {
        self.updated_at.is_some()
    }

    pub fn metadata(&self) -> MetricCacheMetadata {
        MetricCacheMetadata {
            updated_at: self.updated_at,
            updated_at_unix_seconds: self.updated_at_unix_seconds,
            updates_total: self.updates_total,
        }
    }

    pub fn is_stale(&self, stale_after_seconds: u64) -> bool {
        match self.updated_at {
            Some(updated_at) => updated_at.elapsed() > Duration::from_secs(stale_after_seconds),
            None => true,
        }
    }
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use telemon_core::metrics::model::{labels, MetricSample};

    use super::*;

    #[test]
    fn stores_and_replaces_snapshot() {
        let mut cache = MetricCache::new();
        let sample = MetricSample::gauge("exporter_test", "Test.", labels(&[]), 1.0);

        assert!(!cache.has_snapshot());
        cache.replace_snapshot(vec![sample.clone()]);

        assert!(cache.has_snapshot());
        assert_eq!(cache.snapshot(), vec![sample]);
        let metadata = cache.metadata();
        assert_eq!(metadata.updates_total, 1);
        assert!(metadata.updated_at.is_some());
        assert!(metadata.updated_at_unix_seconds.is_some());
        assert!(metadata.age_seconds().is_some());
    }

    #[test]
    fn empty_cache_is_stale() {
        assert!(MetricCache::new().is_stale(60));
    }
}
