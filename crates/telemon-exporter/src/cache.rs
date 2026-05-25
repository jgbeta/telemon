use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use telemon_core::metrics::model::MetricSample;

pub type SharedMetricCache = Arc<RwLock<MetricCache>>;

#[derive(Debug, Clone, Default)]
pub struct MetricCache {
    metrics: Vec<MetricSample>,
    updated_at: Option<Instant>,
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
    }

    pub fn snapshot(&self) -> Vec<MetricSample> {
        self.metrics.clone()
    }

    pub fn has_snapshot(&self) -> bool {
        self.updated_at.is_some()
    }

    pub fn is_stale(&self, stale_after_seconds: u64) -> bool {
        match self.updated_at {
            Some(updated_at) => updated_at.elapsed() > Duration::from_secs(stale_after_seconds),
            None => true,
        }
    }
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
    }

    #[test]
    fn empty_cache_is_stale() {
        assert!(MetricCache::new().is_stale(60));
    }
}
