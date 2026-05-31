use std::collections::VecDeque;

use crate::game::capture::{CaptureStats, CaptureSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSample {
    pub timestamp_ns: u64,
    pub frame_time_ns: u64,
}

#[derive(Debug, Clone)]
pub struct RollingFrameWindow {
    window_ns: u64,
    samples: VecDeque<FrameSample>,
    total_frame_time_ns: u128,
}

impl RollingFrameWindow {
    pub fn new(window_ns: u64) -> Self {
        Self {
            window_ns,
            samples: VecDeque::new(),
            total_frame_time_ns: 0,
        }
    }

    pub fn window_ns(&self) -> u64 {
        self.window_ns
    }

    pub fn frame_count(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn push_frame_time(&mut self, timestamp_ns: u64, frame_time_ns: u64) -> bool {
        if frame_time_ns == 0 {
            return false;
        }
        self.samples.push_back(FrameSample {
            timestamp_ns,
            frame_time_ns,
        });
        self.total_frame_time_ns += frame_time_ns as u128;
        self.evict_old(timestamp_ns);
        true
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.total_frame_time_ns = 0;
    }

    pub fn frame_times_ns(&self) -> impl Iterator<Item = u64> + '_ {
        self.samples.iter().map(|sample| sample.frame_time_ns)
    }

    pub fn summary(&self) -> Option<CaptureSummary> {
        CaptureStats::from_frame_times_ns(self.frame_times_ns()).summary()
    }

    fn evict_old(&mut self, now_ns: u64) {
        let cutoff = now_ns.saturating_sub(self.window_ns);
        while let Some(front) = self.samples.front().copied() {
            if front.timestamp_ns >= cutoff {
                break;
            }
            let removed = self.samples.pop_front().expect("front existed");
            self.total_frame_time_ns = self
                .total_frame_time_ns
                .saturating_sub(removed.frame_time_ns as u128);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_samples_outside_window() {
        let mut window = RollingFrameWindow::new(1_000);
        window.push_frame_time(100, 10);
        window.push_frame_time(500, 10);
        window.push_frame_time(1_200, 10);
        assert_eq!(window.frame_count(), 2);
    }

    #[test]
    fn summary_uses_current_window() {
        let mut window = RollingFrameWindow::new(1_000_000_000);
        window.push_frame_time(16_000_000, 16_000_000);
        window.push_frame_time(33_000_000, 17_000_000);
        assert_eq!(window.summary().unwrap().frame_count, 2);
    }
}
