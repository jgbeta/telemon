#[cfg(target_os = "linux")]
use std::fs;
use std::time::Instant;
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tracing::debug;

use crate::system::model::SystemSnapshot;
use crate::traits::{collector_health_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::SystemConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "system";
pub const SOURCE: &str = "system";

pub trait SystemProvider: Send + Sync {
    fn snapshot(&mut self) -> Result<SystemSnapshot>;
}

#[derive(Debug, Default)]
pub struct DefaultSystemProvider;

pub struct SystemCollector {
    config: SystemConfig,
    provider: Box<dyn SystemProvider>,
    errors_total: u64,
}

impl DefaultSystemProvider {
    pub fn new() -> Self {
        Self
    }
}

impl SystemCollector {
    pub fn new(config: SystemConfig) -> Self {
        Self {
            config,
            provider: Box::new(DefaultSystemProvider::new()),
            errors_total: 0,
        }
    }

    pub fn with_provider(config: SystemConfig, provider: impl SystemProvider + 'static) -> Self {
        Self {
            config,
            provider: Box::new(provider),
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &SystemConfig) -> String {
        if config.enabled {
            "available".to_string()
        } else {
            "disabled".to_string()
        }
    }
}

impl Collector for SystemCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match self.provider.snapshot() {
            Ok(snapshot) => {
                let mut metrics = collector_health_metrics(
                    COLLECTOR_NAME,
                    true,
                    self.errors_total,
                    Some(unix_timestamp_seconds()),
                );
                metrics.extend(snapshot_to_metrics(&self.config, &snapshot));
                CollectorResult::success(COLLECTOR_NAME, metrics, started_at)
            }
            Err(error) => {
                self.errors_total += 1;
                CollectorResult::failure(
                    COLLECTOR_NAME,
                    error.to_string(),
                    self.errors_total,
                    started_at,
                )
            }
        }
    }
}

impl SystemProvider for DefaultSystemProvider {
    fn snapshot(&mut self) -> Result<SystemSnapshot> {
        Ok(SystemSnapshot {
            uptime_seconds: uptime_seconds(),
            memory_total_bytes: memory_total_bytes(),
            memory_available_bytes: memory_available_bytes(),
            cpu_count: cpu_count(),
            cpu_usage_ratio: None,
        })
    }
}

fn snapshot_to_metrics(config: &SystemConfig, snapshot: &SystemSnapshot) -> Vec<MetricSample> {
    let mut metrics = Vec::new();

    if config.uptime_enabled {
        if let Some(seconds) = snapshot
            .uptime_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            metrics.push(MetricSample::gauge(
                names::UPTIME_SECONDS,
                "System uptime in seconds.",
                labels(&[("source", SOURCE)]),
                seconds,
            ));
        }
    }

    if config.memory_enabled {
        if let Some(total) = snapshot.memory_total_bytes {
            metrics.push(memory_metric("total", total));
        }
        if let Some(available) = snapshot.memory_available_bytes {
            metrics.push(memory_metric("available", available));
        }
        match (snapshot.memory_total_bytes, snapshot.memory_available_bytes) {
            (Some(total), Some(available)) if available <= total => {
                metrics.push(memory_metric("used", total - available));
            }
            (Some(total), Some(available)) => {
                debug!(
                    total_bytes = total,
                    available_bytes = available,
                    "omitting inconsistent used memory metric"
                );
            }
            _ => {}
        }
    }

    if config.cpu_enabled {
        if let Some(count) = snapshot.cpu_count.filter(|count| *count > 0) {
            metrics.push(MetricSample::gauge(
                names::SYSTEM_CPU_COUNT,
                "Logical CPU count.",
                labels(&[("source", SOURCE)]),
                count as f64,
            ));
        }
        if let Some(value) = snapshot.cpu_usage_ratio.and_then(normalize_cpu_usage_ratio) {
            metrics.push(MetricSample::gauge(
                names::CPU_USAGE_RATIO,
                "Total system CPU usage as a ratio from 0 to 1.",
                labels(&[("component", "cpu"), ("source", SOURCE)]),
                value,
            ));
        }
    }

    metrics
}

fn memory_metric(state: &str, value: u64) -> MetricSample {
    MetricSample::gauge(
        names::MEMORY_TOTAL_BYTES,
        "System memory bytes by state.",
        labels(&[("source", SOURCE), ("state", state)]),
        value as f64,
    )
}

fn normalize_cpu_usage_ratio(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    if (0.0..=1.0).contains(&value) {
        return Some(value);
    }
    if (-0.001..=1.001).contains(&value) {
        return Some(value.clamp(0.0, 1.0));
    }

    debug!(cpu_usage_ratio = value, "omitting invalid CPU usage ratio");
    None
}

fn cpu_count() -> Option<u64> {
    std::thread::available_parallelism()
        .ok()
        .and_then(|count| u64::try_from(count.get()).ok())
}

#[cfg(target_os = "macos")]
fn uptime_seconds() -> Option<f64> {
    let boot = macos_boot_time_seconds()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    (now >= boot).then_some(now - boot)
}

#[cfg(target_os = "linux")]
fn uptime_seconds() -> Option<f64> {
    fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn uptime_seconds() -> Option<f64> {
    None
}

#[cfg(target_os = "macos")]
fn memory_total_bytes() -> Option<u64> {
    macos_sysctl_u64("hw.memsize")
}

#[cfg(target_os = "linux")]
fn memory_total_bytes() -> Option<u64> {
    linux_meminfo_bytes("MemTotal:")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory_total_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn memory_available_bytes() -> Option<u64> {
    let page_size = macos_sysctl_u64("hw.pagesize")?;
    let free = macos_sysctl_u64("vm.page_free_count")?;
    let inactive = macos_sysctl_u64("vm.page_inactive_count").unwrap_or(0);
    let speculative = macos_sysctl_u64("vm.page_speculative_count").unwrap_or(0);
    free.checked_add(inactive)?
        .checked_add(speculative)?
        .checked_mul(page_size)
}

#[cfg(target_os = "linux")]
fn memory_available_bytes() -> Option<u64> {
    linux_meminfo_bytes("MemAvailable:").or_else(|| linux_meminfo_bytes("MemFree:"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory_available_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn linux_meminfo_bytes(key: &str) -> Option<u64> {
    fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix(key)?;
            let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            kib.checked_mul(1024)
        })
}

#[cfg(target_os = "macos")]
fn macos_boot_time_seconds() -> Option<f64> {
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let mut value = std::mem::MaybeUninit::<libc::timeval>::zeroed();
    let mut size = std::mem::size_of::<libc::timeval>() as libc::size_t;
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            value.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    let value = unsafe { value.assume_init() };
    Some(value.tv_sec as f64 + (value.tv_usec as f64 / 1_000_000.0))
}

#[cfg(target_os = "macos")]
fn macos_sysctl_u64(name: &str) -> Option<u64> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = 0_u64;
    let mut size = std::mem::size_of::<u64>() as libc::size_t;
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == 0 {
        return Some(value);
    }

    let mut value = 0_u32;
    let mut size = std::mem::size_of::<u32>() as libc::size_t;
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0).then_some(u64::from(value))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    struct FakeProvider {
        snapshot: Option<SystemSnapshot>,
    }

    impl SystemProvider for FakeProvider {
        fn snapshot(&mut self) -> Result<SystemSnapshot> {
            Ok(self.snapshot.take().unwrap())
        }
    }

    fn collect_from_snapshot(snapshot: SystemSnapshot) -> Vec<MetricSample> {
        let mut collector = SystemCollector::with_provider(
            SystemConfig::default(),
            FakeProvider {
                snapshot: Some(snapshot),
            },
        );
        collector.collect().metrics
    }

    fn metric_value(
        metrics: &[MetricSample],
        name: &str,
        expected_labels: &[(&str, &str)],
    ) -> Option<f64> {
        let expected = expected_labels
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<BTreeMap<_, _>>();
        metrics
            .iter()
            .find(|metric| metric.name == name && metric.labels == expected)
            .map(|metric| metric.value)
    }

    #[test]
    fn emits_uptime_memory_and_cpu_count() {
        let metrics = collect_from_snapshot(SystemSnapshot {
            uptime_seconds: Some(12_345.0),
            memory_total_bytes: Some(16),
            memory_available_bytes: Some(6),
            cpu_count: Some(10),
            cpu_usage_ratio: Some(0.25),
        });

        assert_eq!(
            metric_value(&metrics, names::UPTIME_SECONDS, &[("source", SOURCE)]),
            Some(12_345.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_TOTAL_BYTES,
                &[("source", SOURCE), ("state", "total")]
            ),
            Some(16.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_AVAILABLE_BYTES,
                &[("source", SOURCE), ("state", "available")]
            ),
            Some(6.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_USED_BYTES,
                &[("source", SOURCE), ("state", "used")]
            ),
            Some(10.0)
        );
        assert_eq!(
            metric_value(&metrics, names::SYSTEM_CPU_COUNT, &[("source", SOURCE)]),
            Some(10.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::CPU_USAGE_RATIO,
                &[("component", "cpu"), ("source", SOURCE)]
            ),
            Some(0.25)
        );
    }

    #[test]
    fn omits_cpu_usage_when_provider_has_no_sample() {
        let metrics = collect_from_snapshot(SystemSnapshot {
            cpu_usage_ratio: None,
            ..SystemSnapshot::default()
        });

        assert!(metrics
            .iter()
            .all(|metric| metric.name != names::CPU_USAGE_RATIO));
    }

    #[test]
    fn invalid_memory_values_do_not_emit_used_memory() {
        let metrics = collect_from_snapshot(SystemSnapshot {
            memory_total_bytes: Some(6),
            memory_available_bytes: Some(16),
            ..SystemSnapshot::default()
        });

        assert!(metric_value(
            &metrics,
            names::MEMORY_USED_BYTES,
            &[("source", SOURCE), ("state", "used")]
        )
        .is_none());
    }
}
