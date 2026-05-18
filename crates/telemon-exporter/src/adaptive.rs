use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use telemon_core::config::AdaptiveSamplingConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

#[derive(Debug, Clone)]
pub struct AdaptiveSamplingState {
    requested_interval_seconds: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct AdaptiveInterval {
    current_seconds: u64,
    last_changed_at: Instant,
}

impl AdaptiveSamplingState {
    pub fn new(default_interval_seconds: u64) -> Self {
        Self {
            requested_interval_seconds: Arc::new(AtomicU64::new(default_interval_seconds)),
        }
    }

    pub fn requested_interval_seconds(&self) -> u64 {
        self.requested_interval_seconds.load(Ordering::Relaxed)
    }

    pub fn set_requested_interval_seconds(&self, value: u64) {
        self.requested_interval_seconds
            .store(value, Ordering::Relaxed);
    }
}

impl AdaptiveInterval {
    pub fn new(current_seconds: u64) -> Self {
        Self {
            current_seconds,
            last_changed_at: Instant::now(),
        }
    }

    pub fn current_seconds(&self) -> u64 {
        self.current_seconds
    }

    pub fn update(&mut self, desired_seconds: u64, cooldown: Duration) -> u64 {
        if desired_seconds != self.current_seconds
            && (desired_seconds < self.current_seconds
                || self.last_changed_at.elapsed() >= cooldown)
        {
            self.current_seconds = desired_seconds;
            self.last_changed_at = Instant::now();
        }
        self.current_seconds
    }
}

pub fn evaluate_requested_interval_seconds(
    config: &AdaptiveSamplingConfig,
    metrics: &[MetricSample],
) -> u64 {
    if !config.enabled {
        return config.levels.normal_seconds;
    }

    let mut requested = config.levels.normal_seconds;
    if config.temperature.enabled {
        for sample in metrics {
            if sample.name != names::TEMPERATURE_CELSIUS {
                continue;
            }
            requested = requested.min(interval_for_temperature(config, sample.value));
        }
    }
    requested
}

pub fn requested_scrape_interval_metric(interval_seconds: u64) -> MetricSample {
    MetricSample::gauge(
        names::REQUESTED_SCRAPE_INTERVAL_SECONDS,
        "Exporter requested Prometheus scrape interval in seconds.",
        labels(&[]),
        interval_seconds as f64,
    )
}

fn interval_for_temperature(config: &AdaptiveSamplingConfig, temperature_celsius: f64) -> u64 {
    if temperature_celsius >= config.temperature.critical_celsius {
        config.levels.critical_seconds
    } else if temperature_celsius >= config.temperature.hot_celsius {
        config.levels.hot_seconds
    } else if temperature_celsius >= config.temperature.warm_celsius {
        config.levels.warm_seconds
    } else {
        config.levels.normal_seconds
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::thread;

    use super::*;

    fn sample(value: f64) -> MetricSample {
        MetricSample::gauge(
            names::TEMPERATURE_CELSIUS,
            "Temperature.",
            BTreeMap::new(),
            value,
        )
    }

    #[test]
    fn maps_temperature_to_interval_levels() {
        let config = AdaptiveSamplingConfig::default();

        assert_eq!(
            evaluate_requested_interval_seconds(&config, &[sample(40.0)]),
            15
        );
        assert_eq!(
            evaluate_requested_interval_seconds(&config, &[sample(60.0)]),
            10
        );
        assert_eq!(
            evaluate_requested_interval_seconds(&config, &[sample(75.0)]),
            5
        );
        assert_eq!(
            evaluate_requested_interval_seconds(&config, &[sample(85.0)]),
            1
        );
    }

    #[test]
    fn slows_down_only_after_cooldown() {
        let mut interval = AdaptiveInterval::new(1);

        assert_eq!(interval.update(15, Duration::from_secs(60)), 1);
        thread::sleep(Duration::from_millis(2));
        assert_eq!(interval.update(15, Duration::from_millis(1)), 15);
    }
}
