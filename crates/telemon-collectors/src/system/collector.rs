#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::time::Instant;
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tracing::debug;

use crate::system::model::{CpuFrequencySample, SystemSnapshot};
use crate::traits::{collector_health_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::SystemConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "system";
pub const SOURCE: &str = "system";
#[cfg(target_os = "linux")]
const LINUX_CPUFREQ_SOURCE: &str = "linux_cpufreq";
#[cfg(target_os = "linux")]
const LINUX_PROC_CPUINFO_SOURCE: &str = "linux_proc_cpuinfo";
const MAX_CPU_FREQUENCY_MHZ: f64 = 20_000.0;

pub trait SystemProvider: Send + Sync {
    fn snapshot(&mut self) -> Result<SystemSnapshot>;
}

#[derive(Debug, Default)]
pub struct DefaultSystemProvider {
    #[cfg(target_os = "linux")]
    previous_cpu_times: Option<LinuxCpuTimes>,
}

pub struct SystemCollector {
    config: SystemConfig,
    provider: Box<dyn SystemProvider>,
    errors_total: u64,
}

impl DefaultSystemProvider {
    pub fn new() -> Self {
        Self::default()
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
            cpu_usage_ratio: cpu_usage_ratio(self),
            cpu_frequency_samples: cpu_frequency_samples(),
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
        for sample in &snapshot.cpu_frequency_samples {
            if let Some(value) = normalize_cpu_frequency_mhz(sample.frequency_mhz) {
                let cpu = sample.cpu.to_string();
                metrics.push(MetricSample::gauge(
                    names::CPU_FREQUENCY_MHZ,
                    "Current logical CPU frequency in decimal megahertz.",
                    labels(&[
                        ("source", sample.source),
                        ("scope", "logical_cpu"),
                        ("cpu", cpu.as_str()),
                        ("state", "current"),
                    ]),
                    value,
                ));
            }
        }
    }

    metrics
}

fn memory_metric(state: &str, value: u64) -> MetricSample {
    MetricSample::gauge(
        names::MEMORY_TOTAL_BYTES,
        "System memory in decimal megabytes by kind and state.",
        labels(&[("source", SOURCE), ("kind", "ram"), ("state", state)]),
        bytes_to_mb(value),
    )
}

fn bytes_to_mb(value: u64) -> f64 {
    value as f64 / 1_000_000.0
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

fn normalize_cpu_frequency_mhz(value: f64) -> Option<f64> {
    if value.is_finite() && value > 0.0 && value <= MAX_CPU_FREQUENCY_MHZ {
        return Some(value);
    }

    debug!(
        cpu_frequency_mhz = value,
        "omitting invalid CPU frequency sample"
    );
    None
}

fn cpu_count() -> Option<u64> {
    std::thread::available_parallelism()
        .ok()
        .and_then(|count| u64::try_from(count.get()).ok())
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxCpuTimes {
    idle: u64,
    total: u64,
}

#[cfg(target_os = "linux")]
fn cpu_usage_ratio(provider: &mut DefaultSystemProvider) -> Option<f64> {
    let current = read_linux_cpu_times()?;
    let previous = provider.previous_cpu_times.replace(current)?;
    linux_cpu_usage_ratio(previous, current)
}

#[cfg(not(target_os = "linux"))]
fn cpu_usage_ratio(_provider: &mut DefaultSystemProvider) -> Option<f64> {
    None
}

#[cfg(target_os = "linux")]
fn read_linux_cpu_times() -> Option<LinuxCpuTimes> {
    parse_linux_cpu_times(&fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(target_os = "linux")]
fn parse_linux_cpu_times(text: &str) -> Option<LinuxCpuTimes> {
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }

    let idle = values
        .get(3)
        .copied()?
        .saturating_add(values.get(4).copied().unwrap_or(0));
    let total = values.iter().copied().sum();
    Some(LinuxCpuTimes { idle, total })
}

#[cfg(target_os = "linux")]
fn linux_cpu_usage_ratio(previous: LinuxCpuTimes, current: LinuxCpuTimes) -> Option<f64> {
    let total_delta = current.total.checked_sub(previous.total)?;
    if total_delta == 0 {
        return None;
    }
    let idle_delta = current.idle.saturating_sub(previous.idle).min(total_delta);
    Some((total_delta - idle_delta) as f64 / total_delta as f64)
}

#[cfg(target_os = "linux")]
fn cpu_frequency_samples() -> Vec<CpuFrequencySample> {
    let sysfs_samples = read_linux_cpufreq_samples(Path::new("/sys/devices/system/cpu"));
    if !sysfs_samples.is_empty() {
        return sysfs_samples;
    }

    read_linux_proc_cpuinfo_frequency_samples()
}

#[cfg(not(target_os = "linux"))]
fn cpu_frequency_samples() -> Vec<CpuFrequencySample> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn read_linux_cpufreq_samples(root: &Path) -> Vec<CpuFrequencySample> {
    let mut samples = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return samples;
    };

    for entry in entries.flatten() {
        let Some(cpu_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let cpufreq = entry.path().join("cpufreq");
        let frequency_text = fs::read_to_string(cpufreq.join("scaling_cur_freq"))
            .or_else(|_| fs::read_to_string(cpufreq.join("cpuinfo_cur_freq")));
        let Ok(frequency_text) = frequency_text else {
            continue;
        };
        if let Some(sample) = parse_linux_cpufreq_sample(&cpu_name, &frequency_text) {
            samples.push(sample);
        }
    }

    sort_cpu_frequency_samples(&mut samples);
    samples
}

#[cfg(target_os = "linux")]
fn parse_linux_cpufreq_sample(cpu_name: &str, frequency_text: &str) -> Option<CpuFrequencySample> {
    let cpu = parse_linux_cpu_index(cpu_name)?;
    let khz = frequency_text.trim().parse::<f64>().ok()?;
    let frequency_mhz = normalize_cpu_frequency_mhz(khz / 1_000.0)?;
    Some(CpuFrequencySample {
        cpu,
        frequency_mhz,
        source: LINUX_CPUFREQ_SOURCE,
    })
}

#[cfg(target_os = "linux")]
fn read_linux_proc_cpuinfo_frequency_samples() -> Vec<CpuFrequencySample> {
    let Some(text) = fs::read_to_string("/proc/cpuinfo").ok() else {
        return Vec::new();
    };
    parse_linux_proc_cpuinfo_frequency_samples(&text)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_cpuinfo_frequency_samples(text: &str) -> Vec<CpuFrequencySample> {
    let mut samples = Vec::new();
    let mut current_cpu = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "processor" => current_cpu = value.trim().parse::<u32>().ok(),
            "cpu MHz" => {
                let Some(cpu) = current_cpu.take() else {
                    continue;
                };
                let Ok(frequency_mhz) = value.trim().parse::<f64>() else {
                    continue;
                };
                if let Some(frequency_mhz) = normalize_cpu_frequency_mhz(frequency_mhz) {
                    samples.push(CpuFrequencySample {
                        cpu,
                        frequency_mhz,
                        source: LINUX_PROC_CPUINFO_SOURCE,
                    });
                }
            }
            _ => {}
        }
    }

    sort_cpu_frequency_samples(&mut samples);
    samples
}

#[cfg(target_os = "linux")]
fn parse_linux_cpu_index(cpu_name: &str) -> Option<u32> {
    cpu_name.strip_prefix("cpu")?.parse::<u32>().ok()
}

#[cfg(target_os = "linux")]
fn sort_cpu_frequency_samples(samples: &mut [CpuFrequencySample]) {
    samples.sort_by_key(|sample| sample.cpu);
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
            memory_total_bytes: Some(16_000_000),
            memory_available_bytes: Some(6_000_000),
            cpu_count: Some(10),
            cpu_usage_ratio: Some(0.25),
            cpu_frequency_samples: vec![CpuFrequencySample {
                cpu: 0,
                frequency_mhz: 2_800.0,
                source: "linux_cpufreq",
            }],
        });

        assert_eq!(
            metric_value(&metrics, names::UPTIME_SECONDS, &[("source", SOURCE)]),
            Some(12_345.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_TOTAL_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "total")]
            ),
            Some(16.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_AVAILABLE_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "available")]
            ),
            Some(6.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MEMORY_USED_BYTES,
                &[("source", SOURCE), ("kind", "ram"), ("state", "used")]
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
        assert_eq!(
            metric_value(
                &metrics,
                names::CPU_FREQUENCY_MHZ,
                &[
                    ("cpu", "0"),
                    ("scope", "logical_cpu"),
                    ("source", "linux_cpufreq"),
                    ("state", "current")
                ]
            ),
            Some(2_800.0)
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

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_proc_stat_cpu_usage_delta() {
        let previous = parse_linux_cpu_times(
            "cpu  10 0 10 80 0 0 0 0 0 0
",
        )
        .unwrap();
        let current = parse_linux_cpu_times(
            "cpu  20 0 20 100 0 0 0 0 0 0
",
        )
        .unwrap();

        assert_eq!(linux_cpu_usage_ratio(previous, current), Some(0.5));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn first_default_linux_cpu_sample_is_omitted() {
        let mut provider = DefaultSystemProvider::new();
        let first = provider.snapshot().unwrap();

        assert!(first.cpu_usage_ratio.is_none());
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
            &[("source", SOURCE), ("kind", "ram"), ("state", "used")]
        )
        .is_none());
    }

    #[test]
    fn cpu_disabled_omits_frequency_samples() {
        let config = SystemConfig {
            cpu_enabled: false,
            ..SystemConfig::default()
        };
        let mut collector = SystemCollector::with_provider(
            config,
            FakeProvider {
                snapshot: Some(SystemSnapshot {
                    cpu_frequency_samples: vec![CpuFrequencySample {
                        cpu: 0,
                        frequency_mhz: 2_800.0,
                        source: "linux_cpufreq",
                    }],
                    ..SystemSnapshot::default()
                }),
            },
        );

        let metrics = collector.collect().metrics;

        assert!(metrics
            .iter()
            .all(|metric| metric.name != names::CPU_FREQUENCY_MHZ));
    }

    #[test]
    fn invalid_cpu_frequency_samples_are_omitted() {
        let metrics = collect_from_snapshot(SystemSnapshot {
            cpu_frequency_samples: vec![CpuFrequencySample {
                cpu: 0,
                frequency_mhz: 0.0,
                source: "linux_cpufreq",
            }],
            ..SystemSnapshot::default()
        });

        assert!(metrics
            .iter()
            .all(|metric| metric.name != names::CPU_FREQUENCY_MHZ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_cpufreq_samples_as_logical_cpu_frequency() {
        let mut samples = vec![
            parse_linux_cpufreq_sample("cpu10", "2800000\n").unwrap(),
            parse_linux_cpufreq_sample("cpu2", "1700000\n").unwrap(),
        ];
        sort_cpu_frequency_samples(&mut samples);

        assert_eq!(
            samples,
            vec![
                CpuFrequencySample {
                    cpu: 2,
                    frequency_mhz: 1_700.0,
                    source: LINUX_CPUFREQ_SOURCE,
                },
                CpuFrequencySample {
                    cpu: 10,
                    frequency_mhz: 2_800.0,
                    source: LINUX_CPUFREQ_SOURCE,
                },
            ]
        );
        assert!(parse_linux_cpufreq_sample("notcpu", "2800000\n").is_none());
        assert!(parse_linux_cpufreq_sample("cpu0", "0\n").is_none());
        assert!(parse_linux_cpufreq_sample("cpu0", "25000000\n").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_proc_cpuinfo_frequency_fallback_samples() {
        let samples = parse_linux_proc_cpuinfo_frequency_samples(
            "processor   : 1
cpu MHz     : 3512.345

processor   : 0
cpu MHz     : 800.000
",
        );

        assert_eq!(
            samples,
            vec![
                CpuFrequencySample {
                    cpu: 0,
                    frequency_mhz: 800.0,
                    source: LINUX_PROC_CPUINFO_SOURCE,
                },
                CpuFrequencySample {
                    cpu: 1,
                    frequency_mhz: 3512.345,
                    source: LINUX_PROC_CPUINFO_SOURCE,
                },
            ]
        );
    }
}
