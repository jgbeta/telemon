//! Exact FPS and frame pacing summaries from frame durations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    Worst,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureSummary {
    pub frame_count: usize,
    pub total_time_ns: u128,
    pub average_fps: f64,
    pub average_frame_time_seconds: f64,
    pub min_frame_time_seconds: f64,
    pub max_frame_time_seconds: f64,
    pub p50_frame_time_seconds: f64,
    pub p95_frame_time_seconds: f64,
    pub p99_frame_time_seconds: f64,
    pub low_1pct_fps_threshold: f64,
    pub low_1pct_fps_average: f64,
    pub low_01pct_fps_threshold: f64,
    pub low_01pct_fps_average: f64,
    pub high_1pct_fps_threshold: f64,
    pub high_1pct_fps_average: f64,
    pub jitter: Option<JitterSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JitterSummary {
    pub sample_count: usize,
    pub average_seconds: f64,
    pub p95_seconds: f64,
    pub p99_seconds: f64,
    pub max_seconds: f64,
}

#[derive(Debug, Clone, Default)]
pub struct CaptureStats {
    frame_times_ns: Vec<u64>,
    total_ns: u128,
}

impl CaptureStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_frame_times_ns<I>(frame_times_ns: I) -> Self
    where
        I: IntoIterator<Item = u64>,
    {
        let mut stats = Self::new();
        for ns in frame_times_ns {
            stats.push_frame_time_ns(ns);
        }
        stats
    }

    pub fn push_frame_time_ns(&mut self, ns: u64) -> bool {
        if ns == 0 {
            return false;
        }
        self.frame_times_ns.push(ns);
        self.total_ns += ns as u128;
        true
    }

    pub fn push_frame_time_ns_filtered(&mut self, ns: u64, max_frame_time_ns: u64) -> bool {
        if ns == 0 || ns > max_frame_time_ns {
            return false;
        }
        self.push_frame_time_ns(ns)
    }

    pub fn frame_count(&self) -> usize {
        self.frame_times_ns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frame_times_ns.is_empty()
    }

    pub fn clear(&mut self) {
        self.frame_times_ns.clear();
        self.total_ns = 0;
    }

    pub fn average_fps(&self) -> Option<f64> {
        avg_fps_from_frame_times_ns(self.frame_times_ns.len(), self.total_ns)
    }

    pub fn low_fps_threshold(&self, fraction: f64) -> Option<f64> {
        tail_threshold_fps(&self.frame_times_ns, fraction, Tail::Worst)
    }

    pub fn low_fps_average(&self, fraction: f64) -> Option<f64> {
        tail_average_fps(&self.frame_times_ns, fraction, Tail::Worst)
    }

    pub fn high_fps_threshold(&self, fraction: f64) -> Option<f64> {
        tail_threshold_fps(&self.frame_times_ns, fraction, Tail::Best)
    }

    pub fn high_fps_average(&self, fraction: f64) -> Option<f64> {
        tail_average_fps(&self.frame_times_ns, fraction, Tail::Best)
    }

    pub fn summary(&self) -> Option<CaptureSummary> {
        let n = self.frame_times_ns.len();
        if n == 0 || self.total_ns == 0 {
            return None;
        }

        let mut sorted = self.frame_times_ns.clone();
        sorted.sort_unstable();

        Some(CaptureSummary {
            frame_count: n,
            total_time_ns: self.total_ns,
            average_fps: avg_fps_from_frame_times_ns(n, self.total_ns)?,
            average_frame_time_seconds: ns_to_seconds(self.total_ns as f64 / n as f64),
            min_frame_time_seconds: ns_to_seconds(sorted[0] as f64),
            max_frame_time_seconds: ns_to_seconds(sorted[n - 1] as f64),
            p50_frame_time_seconds: ns_to_seconds(percentile_from_sorted_ns(&sorted, 0.50)? as f64),
            p95_frame_time_seconds: ns_to_seconds(percentile_from_sorted_ns(&sorted, 0.95)? as f64),
            p99_frame_time_seconds: ns_to_seconds(percentile_from_sorted_ns(&sorted, 0.99)? as f64),
            low_1pct_fps_threshold: tail_threshold_fps_from_sorted(&sorted, 0.01, Tail::Worst)?,
            low_1pct_fps_average: tail_average_fps_from_sorted(&sorted, 0.01, Tail::Worst)?,
            low_01pct_fps_threshold: tail_threshold_fps_from_sorted(&sorted, 0.001, Tail::Worst)?,
            low_01pct_fps_average: tail_average_fps_from_sorted(&sorted, 0.001, Tail::Worst)?,
            high_1pct_fps_threshold: tail_threshold_fps_from_sorted(&sorted, 0.01, Tail::Best)?,
            high_1pct_fps_average: tail_average_fps_from_sorted(&sorted, 0.01, Tail::Best)?,
            jitter: jitter_summary_ns(&self.frame_times_ns),
        })
    }
}

pub fn fps_from_frame_time_ns(frame_time_ns: u64) -> Option<f64> {
    if frame_time_ns == 0 {
        return None;
    }
    Some(1_000_000_000.0 / frame_time_ns as f64)
}

pub fn avg_fps_from_frame_times_ns(frame_count: usize, total_ns: u128) -> Option<f64> {
    if frame_count == 0 || total_ns == 0 {
        return None;
    }
    Some(frame_count as f64 * 1_000_000_000.0 / total_ns as f64)
}

pub fn tail_threshold_fps(values_ns: &[u64], fraction: f64, tail: Tail) -> Option<f64> {
    let n = values_ns.len();
    let k = tail_count(n, fraction)?;
    let idx = match tail {
        Tail::Worst => n - k,
        Tail::Best => k - 1,
    };
    let mut values = values_ns.to_vec();
    let (_, nth, _) = values.select_nth_unstable(idx);
    fps_from_frame_time_ns(*nth)
}

pub fn tail_average_fps(values_ns: &[u64], fraction: f64, tail: Tail) -> Option<f64> {
    let n = values_ns.len();
    let k = tail_count(n, fraction)?;
    let mut values = values_ns.to_vec();
    let sum_ns: u128 = match tail {
        Tail::Worst => {
            let idx = n - k;
            values.select_nth_unstable(idx);
            values[idx..].iter().map(|&ns| ns as u128).sum()
        }
        Tail::Best => {
            let idx = k - 1;
            values.select_nth_unstable(idx);
            values[..k].iter().map(|&ns| ns as u128).sum()
        }
    };
    avg_fps_from_frame_times_ns(k, sum_ns)
}

fn jitter_values_ns(frame_times_ns: &[u64]) -> Vec<u64> {
    frame_times_ns
        .windows(2)
        .map(|window| window[0].abs_diff(window[1]))
        .collect()
}

fn jitter_summary_ns(frame_times_ns: &[u64]) -> Option<JitterSummary> {
    let mut jitter = jitter_values_ns(frame_times_ns);
    let sample_count = jitter.len();
    if sample_count == 0 {
        return None;
    }
    let total_ns: u128 = jitter.iter().map(|&ns| ns as u128).sum();
    jitter.sort_unstable();
    Some(JitterSummary {
        sample_count,
        average_seconds: ns_to_seconds(total_ns as f64 / sample_count as f64),
        p95_seconds: ns_to_seconds(percentile_from_sorted_ns(&jitter, 0.95)? as f64),
        p99_seconds: ns_to_seconds(percentile_from_sorted_ns(&jitter, 0.99)? as f64),
        max_seconds: ns_to_seconds(jitter[sample_count - 1] as f64),
    })
}

fn ns_to_seconds(ns: f64) -> f64 {
    ns / 1_000_000_000.0
}

fn percentile_index(n: usize, percentile: f64) -> Option<usize> {
    if n == 0 || !percentile.is_finite() || !(0.0..=1.0).contains(&percentile) {
        return None;
    }
    if percentile == 0.0 {
        return Some(0);
    }
    Some(
        ((percentile * n as f64).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1),
    )
}

fn percentile_from_sorted_ns(sorted_ns: &[u64], percentile: f64) -> Option<u64> {
    Some(sorted_ns[percentile_index(sorted_ns.len(), percentile)?])
}

fn tail_count(n: usize, fraction: f64) -> Option<usize> {
    if n == 0 || !fraction.is_finite() || fraction <= 0.0 || fraction > 1.0 {
        return None;
    }
    Some(((n as f64 * fraction).ceil() as usize).clamp(1, n))
}

fn tail_threshold_fps_from_sorted(sorted_ns: &[u64], fraction: f64, tail: Tail) -> Option<f64> {
    let n = sorted_ns.len();
    let k = tail_count(n, fraction)?;
    let frame_time_ns = match tail {
        Tail::Worst => sorted_ns[n - k],
        Tail::Best => sorted_ns[k - 1],
    };
    fps_from_frame_time_ns(frame_time_ns)
}

fn tail_average_fps_from_sorted(sorted_ns: &[u64], fraction: f64, tail: Tail) -> Option<f64> {
    let n = sorted_ns.len();
    let k = tail_count(n, fraction)?;
    let sum_ns: u128 = match tail {
        Tail::Worst => sorted_ns[n - k..].iter().map(|&ns| ns as u128).sum(),
        Tail::Best => sorted_ns[..k].iter().map(|&ns| ns as u128).sum(),
    };
    avg_fps_from_frame_times_ns(k, sum_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(actual: f64, expected: f64, epsilon: f64) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "actual={actual}, expected={expected}"
        );
    }

    #[test]
    fn average_fps_uses_total_time() {
        let stats = CaptureStats::from_frame_times_ns([16_666_667, 33_333_333]);
        approx_eq(stats.average_fps().unwrap(), 40.0, 0.000_01);
    }

    #[test]
    fn low_and_high_one_percent_are_deterministic() {
        let mut frames = vec![10_000_000_u64; 99];
        frames.push(100_000_000);
        let stats = CaptureStats::from_frame_times_ns(frames);
        approx_eq(stats.low_fps_threshold(0.01).unwrap(), 10.0, 0.000_01);
        approx_eq(stats.low_fps_average(0.01).unwrap(), 10.0, 0.000_01);
        approx_eq(stats.high_fps_threshold(0.01).unwrap(), 100.0, 0.000_01);
        approx_eq(stats.high_fps_average(0.01).unwrap(), 100.0, 0.000_01);
    }

    #[test]
    fn summary_uses_seconds_for_frame_times() {
        let stats = CaptureStats::from_frame_times_ns([16_000_000, 17_000_000, 33_000_000]);
        let summary = stats.summary().unwrap();
        assert_eq!(summary.frame_count, 3);
        approx_eq(summary.average_frame_time_seconds, 0.022, 0.000_001);
        approx_eq(summary.average_fps, 45.454_545, 0.000_01);
        approx_eq(summary.p50_frame_time_seconds, 0.017, 0.000_001);
        assert!(summary.jitter.is_some());
    }
}
