use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, warn};

use crate::traits::{collector_status_metrics, unix_timestamp_seconds, Collector, CollectorResult};
use telemon_core::config::MacosMacmonConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "macos_macmon";
pub const SOURCE: &str = "macmon";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawMacmonSnapshot {
    pub cpu_temperature_celsius: Option<f64>,
    pub gpu_temperature_celsius: Option<f64>,
    pub cpu_usage_ratio: Option<f64>,
    pub efficiency_cpu_utilization_ratio: Option<f64>,
    pub performance_cpu_utilization_ratio: Option<f64>,
    pub efficiency_cpu_clock_mhz: Option<f64>,
    pub performance_cpu_clock_mhz: Option<f64>,
    pub gpu_utilization_ratio: Option<f64>,
    pub gpu_clock_mhz: Option<f64>,
    pub cpu_power_watts: Option<f64>,
    pub gpu_power_watts: Option<f64>,
    pub ane_power_watts: Option<f64>,
    pub soc_power_watts: Option<f64>,
    pub system_power_watts: Option<f64>,
    pub ram_power_watts: Option<f64>,
    pub gpu_ram_power_watts: Option<f64>,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub swap_total_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MacmonSnapshot {
    pub cpu_temperature_celsius: Option<f64>,
    pub gpu_temperature_celsius: Option<f64>,
    pub cpu_usage_ratio: Option<f64>,
    pub efficiency_cpu_utilization_ratio: Option<f64>,
    pub performance_cpu_utilization_ratio: Option<f64>,
    pub efficiency_cpu_clock_hertz: Option<f64>,
    pub performance_cpu_clock_hertz: Option<f64>,
    pub gpu_utilization_ratio: Option<f64>,
    pub gpu_clock_hertz: Option<f64>,
    pub cpu_power_watts: Option<f64>,
    pub gpu_power_watts: Option<f64>,
    pub ane_power_watts: Option<f64>,
    pub soc_power_watts: Option<f64>,
    pub system_power_watts: Option<f64>,
    pub ram_power_watts: Option<f64>,
    pub gpu_ram_power_watts: Option<f64>,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub swap_total_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MacmonStaticInfo {
    pub mac_model: String,
    pub chip_name: String,
    pub efficiency_cpu_cores: u8,
    pub performance_cpu_cores: u8,
    pub efficiency_cpu_frequencies_mhz: Vec<u32>,
    pub performance_cpu_frequencies_mhz: Vec<u32>,
    pub gpu_cores: u8,
    pub gpu_frequencies_mhz: Vec<u32>,
}

#[derive(Debug, Clone)]
struct CachedSnapshot {
    snapshot: MacmonSnapshot,
    captured_at: Instant,
    captured_unix_seconds: u64,
}

#[derive(Debug, Clone, Default)]
struct MacmonCache {
    supported: bool,
    latest: Option<CachedSnapshot>,
    static_info: Option<MacmonStaticInfo>,
    errors_total: u64,
    consecutive_errors: u64,
    reinitializations_total: u64,
    invalid_samples_total: BTreeMap<String, u64>,
    last_error: Option<String>,
}

pub struct MacosMacmonCollector {
    config: MacosMacmonConfig,
    cache: Arc<Mutex<MacmonCache>>,
}

impl MacosMacmonCollector {
    pub fn new(config: MacosMacmonConfig) -> Self {
        let cache = Arc::new(Mutex::new(MacmonCache {
            supported: platform_supported(),
            ..MacmonCache::default()
        }));

        if config.enabled && platform_supported() {
            spawn_sampler_thread(config.clone(), Arc::clone(&cache));
        }

        Self { config, cache }
    }

    pub fn discover_summary(config: &MacosMacmonConfig) -> String {
        if !config.enabled {
            return "disabled".to_string();
        }
        if platform_supported() {
            "available".to_string()
        } else {
            "unsupported on this OS or architecture".to_string()
        }
    }

    fn cache_snapshot(&self) -> MacmonCache {
        match self.cache.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Collector for MacosMacmonCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();
        let cache = self.cache_snapshot();
        let stale_after = Duration::from_secs(self.config.stale_after_seconds);
        let latest_age = cache
            .latest
            .as_ref()
            .map(|snapshot| snapshot.captured_at.elapsed());
        let up = cache.supported && latest_age.map(|age| age <= stale_after).unwrap_or(false);
        let last_success_timestamp = cache
            .latest
            .as_ref()
            .map(|snapshot| snapshot.captured_unix_seconds);

        let mut metrics = collector_status_metrics(
            COLLECTOR_NAME,
            cache.supported,
            up,
            cache.errors_total,
            last_success_timestamp,
        );
        metrics.extend(diagnostic_metrics(&cache, latest_age));

        if up {
            if let Some(latest) = &cache.latest {
                let dynamic_metrics =
                    snapshot_to_dynamic_metrics(&latest.snapshot, cache.static_info.as_ref());
                metrics.push(collector_samples_metric(dynamic_metrics.len()));
                metrics.extend(dynamic_metrics);
            }
        }

        if let Some(static_info) = &cache.static_info {
            metrics.extend(static_info_to_metrics(static_info));
        }

        let waiting_for_first_sample =
            cache.supported && cache.latest.is_none() && cache.errors_total == 0;
        let success = up || !cache.supported || waiting_for_first_sample;
        let error_message = if up {
            None
        } else if !cache.supported {
            Some("macos_macmon is unsupported on this OS or architecture".to_string())
        } else if let Some(error) = cache.last_error {
            Some(error)
        } else if waiting_for_first_sample {
            Some("macos_macmon snapshot is not ready yet".to_string())
        } else {
            Some("macos_macmon snapshot is stale".to_string())
        };

        CollectorResult {
            collector: COLLECTOR_NAME,
            success,
            metrics,
            error_message,
            duration: started_at.elapsed(),
        }
    }
}

fn spawn_sampler_thread(config: MacosMacmonConfig, cache: Arc<Mutex<MacmonCache>>) {
    let thread_config = config.clone();
    let thread_cache = Arc::clone(&cache);
    if let Err(error) = thread::Builder::new()
        .name("telemon-macos-macmon".to_string())
        .spawn(move || sampler_loop(thread_config, thread_cache))
    {
        record_error(
            &cache,
            format!("failed to spawn macos_macmon sampler thread: {error}"),
        );
    }
}

fn sampler_loop(config: MacosMacmonConfig, cache: Arc<Mutex<MacmonCache>>) {
    let sample_interval = Duration::from_secs(config.sample_interval_seconds);
    let retry_backoff = sample_interval.max(Duration::from_secs(1));
    let mut provider: Option<platform::MacmonPlatformProvider> = None;

    loop {
        if provider.is_none() {
            match platform::MacmonPlatformProvider::new() {
                Ok(new_provider) => {
                    provider = Some(new_provider);
                    debug!("macos_macmon sampler initialized");
                }
                Err(error) => {
                    let message = format!("failed to initialize macmon sampler: {error}");
                    warn!(error = %message, "macos_macmon sampler initialization failed");
                    record_error(&cache, message);
                    thread::sleep(retry_backoff);
                    continue;
                }
            }
        }

        let sample_result = provider
            .as_mut()
            .expect("macmon provider must exist after initialization")
            .sample(config.sample_window_milliseconds);

        match sample_result {
            Ok((raw_snapshot, static_info)) => {
                let normalized = normalize_snapshot(&raw_snapshot, &config);
                record_success(
                    &cache,
                    normalized.snapshot,
                    static_info,
                    normalized.invalid_counts,
                );
            }
            Err(error) => {
                let message = format!("failed to sample macmon metrics: {error}");
                warn!(error = %message, "macos_macmon sampler failed");
                let consecutive_errors = record_error(&cache, message);
                if consecutive_errors >= config.reinitialize_after_consecutive_errors {
                    provider = None;
                    record_reinitialization(&cache);
                }
            }
        }

        thread::sleep(sample_interval);
    }
}

fn record_success(
    cache: &Arc<Mutex<MacmonCache>>,
    snapshot: MacmonSnapshot,
    static_info: MacmonStaticInfo,
    invalid_counts: BTreeMap<String, u64>,
) {
    update_cache(cache, |cache| {
        cache.latest = Some(CachedSnapshot {
            snapshot,
            captured_at: Instant::now(),
            captured_unix_seconds: unix_timestamp_seconds(),
        });
        cache.static_info = Some(static_info);
        cache.consecutive_errors = 0;
        cache.last_error = None;
        for (field, count) in invalid_counts {
            *cache.invalid_samples_total.entry(field).or_insert(0) += count;
        }
    });
}

fn record_error(cache: &Arc<Mutex<MacmonCache>>, error: impl Into<String>) -> u64 {
    let mut consecutive_errors = 0;
    update_cache(cache, |cache| {
        cache.errors_total += 1;
        cache.consecutive_errors += 1;
        cache.last_error = Some(error.into());
        consecutive_errors = cache.consecutive_errors;
    });
    consecutive_errors
}

fn record_reinitialization(cache: &Arc<Mutex<MacmonCache>>) {
    update_cache(cache, |cache| {
        cache.reinitializations_total += 1;
        cache.consecutive_errors = 0;
    });
}

fn update_cache(cache: &Arc<Mutex<MacmonCache>>, update: impl FnOnce(&mut MacmonCache)) {
    match cache.lock() {
        Ok(mut guard) => update(&mut guard),
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            update(&mut guard);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct NormalizedSnapshot {
    snapshot: MacmonSnapshot,
    invalid_counts: BTreeMap<String, u64>,
}

fn normalize_snapshot(raw: &RawMacmonSnapshot, config: &MacosMacmonConfig) -> NormalizedSnapshot {
    let mut invalid_counts = BTreeMap::new();
    let (memory_total_bytes, memory_used_bytes) = normalize_bytes_pair(
        "memory_total_bytes",
        "memory_used_bytes",
        raw.memory_total_bytes,
        raw.memory_used_bytes,
        false,
        &mut invalid_counts,
    );
    let (swap_total_bytes, swap_used_bytes) = normalize_bytes_pair(
        "swap_total_bytes",
        "swap_used_bytes",
        raw.swap_total_bytes,
        raw.swap_used_bytes,
        true,
        &mut invalid_counts,
    );

    NormalizedSnapshot {
        snapshot: MacmonSnapshot {
            cpu_temperature_celsius: normalize_temperature(
                "cpu_temperature_celsius",
                raw.cpu_temperature_celsius,
                config,
                &mut invalid_counts,
            ),
            gpu_temperature_celsius: normalize_temperature(
                "gpu_temperature_celsius",
                raw.gpu_temperature_celsius,
                config,
                &mut invalid_counts,
            ),
            cpu_usage_ratio: normalize_ratio(
                "cpu_usage_ratio",
                raw.cpu_usage_ratio,
                &mut invalid_counts,
            ),
            efficiency_cpu_utilization_ratio: normalize_ratio(
                "efficiency_cpu_utilization_ratio",
                raw.efficiency_cpu_utilization_ratio,
                &mut invalid_counts,
            ),
            performance_cpu_utilization_ratio: normalize_ratio(
                "performance_cpu_utilization_ratio",
                raw.performance_cpu_utilization_ratio,
                &mut invalid_counts,
            ),
            efficiency_cpu_clock_hertz: normalize_frequency_mhz(
                "efficiency_cpu_clock_mhz",
                raw.efficiency_cpu_clock_mhz,
                &mut invalid_counts,
            ),
            performance_cpu_clock_hertz: normalize_frequency_mhz(
                "performance_cpu_clock_mhz",
                raw.performance_cpu_clock_mhz,
                &mut invalid_counts,
            ),
            gpu_utilization_ratio: normalize_ratio(
                "gpu_utilization_ratio",
                raw.gpu_utilization_ratio,
                &mut invalid_counts,
            ),
            gpu_clock_hertz: normalize_frequency_mhz(
                "gpu_clock_mhz",
                raw.gpu_clock_mhz,
                &mut invalid_counts,
            ),
            cpu_power_watts: normalize_power(
                "cpu_power_watts",
                raw.cpu_power_watts,
                config,
                &mut invalid_counts,
            ),
            gpu_power_watts: normalize_power(
                "gpu_power_watts",
                raw.gpu_power_watts,
                config,
                &mut invalid_counts,
            ),
            ane_power_watts: normalize_power(
                "ane_power_watts",
                raw.ane_power_watts,
                config,
                &mut invalid_counts,
            ),
            soc_power_watts: normalize_power(
                "soc_power_watts",
                raw.soc_power_watts,
                config,
                &mut invalid_counts,
            ),
            system_power_watts: normalize_power(
                "system_power_watts",
                raw.system_power_watts,
                config,
                &mut invalid_counts,
            ),
            ram_power_watts: normalize_power(
                "ram_power_watts",
                raw.ram_power_watts,
                config,
                &mut invalid_counts,
            ),
            gpu_ram_power_watts: normalize_power(
                "gpu_ram_power_watts",
                raw.gpu_ram_power_watts,
                config,
                &mut invalid_counts,
            ),
            memory_total_bytes,
            memory_used_bytes,
            swap_total_bytes,
            swap_used_bytes,
        },
        invalid_counts,
    }
}

fn normalize_temperature(
    field: &'static str,
    value: Option<f64>,
    config: &MacosMacmonConfig,
    invalid_counts: &mut BTreeMap<String, u64>,
) -> Option<f64> {
    let value = value?;
    if value.is_finite()
        && value >= config.min_temperature_celsius
        && value <= config.max_temperature_celsius
    {
        Some(value)
    } else {
        count_invalid(invalid_counts, field);
        None
    }
}

fn normalize_ratio(
    field: &'static str,
    value: Option<f64>,
    invalid_counts: &mut BTreeMap<String, u64>,
) -> Option<f64> {
    let value = value?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Some(value)
    } else if value.is_finite() && value > 1.0 && value <= 100.0 {
        Some(value / 100.0)
    } else {
        count_invalid(invalid_counts, field);
        None
    }
}

fn normalize_frequency_mhz(
    field: &'static str,
    value: Option<f64>,
    invalid_counts: &mut BTreeMap<String, u64>,
) -> Option<f64> {
    let value = value?;
    if value.is_finite() && value > 0.0 {
        Some(value * 1_000_000.0)
    } else {
        count_invalid(invalid_counts, field);
        None
    }
}

fn normalize_power(
    field: &'static str,
    value: Option<f64>,
    config: &MacosMacmonConfig,
    invalid_counts: &mut BTreeMap<String, u64>,
) -> Option<f64> {
    let value = value?;
    if value.is_finite() && value >= 0.0 && value <= config.max_power_watts {
        Some(value)
    } else {
        count_invalid(invalid_counts, field);
        None
    }
}

fn normalize_bytes_pair(
    total_field: &'static str,
    used_field: &'static str,
    total: Option<u64>,
    used: Option<u64>,
    allow_zero_total: bool,
    invalid_counts: &mut BTreeMap<String, u64>,
) -> (Option<u64>, Option<u64>) {
    let total = match total {
        Some(value) if allow_zero_total || value > 0 => Some(value),
        Some(_) => {
            count_invalid(invalid_counts, total_field);
            None
        }
        None => None,
    };

    let used = match (used, total) {
        (Some(used), Some(total)) if used <= total => Some(used),
        (Some(_), Some(_)) | (Some(_), None) => {
            count_invalid(invalid_counts, used_field);
            None
        }
        (None, _) => None,
    };

    (total, used)
}

fn count_invalid(invalid_counts: &mut BTreeMap<String, u64>, field: &'static str) {
    *invalid_counts.entry(field.to_string()).or_insert(0) += 1;
}

fn diagnostic_metrics(cache: &MacmonCache, latest_age: Option<Duration>) -> Vec<MetricSample> {
    let mut metrics = vec![MetricSample::counter(
        names::EXPORTER_MACOS_MACMON_REINITIALIZATIONS_TOTAL,
        "Total macOS macmon sampler reinitializations.",
        labels(&[]),
        cache.reinitializations_total as f64,
    )];

    if let Some(age) = latest_age {
        metrics.push(MetricSample::gauge(
            names::EXPORTER_MACOS_MACMON_SNAPSHOT_AGE_SECONDS,
            "Age of the latest macOS macmon snapshot in seconds.",
            labels(&[]),
            age.as_secs_f64(),
        ));
    }

    for (field, count) in &cache.invalid_samples_total {
        metrics.push(MetricSample::counter(
            names::EXPORTER_MACOS_MACMON_INVALID_SAMPLES_TOTAL,
            "Total macOS macmon samples skipped during normalization.",
            labels(&[("field", field.as_str())]),
            *count as f64,
        ));
    }

    metrics
}

fn collector_samples_metric(count: usize) -> MetricSample {
    MetricSample::gauge(
        names::COLLECTOR_SAMPLES,
        "Useful samples emitted by a collector in the last collection run.",
        labels(&[("collector", COLLECTOR_NAME), ("kind", "dynamic")]),
        count as f64,
    )
}

fn snapshot_to_dynamic_metrics(
    snapshot: &MacmonSnapshot,
    static_info: Option<&MacmonStaticInfo>,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let chip = static_info
        .map(|info| info.chip_name.as_str())
        .filter(|chip| !chip.is_empty());

    if let Some(value) = snapshot.cpu_usage_ratio {
        metrics.push(MetricSample::gauge(
            names::CPU_USAGE_RATIO,
            "Total system CPU usage as a ratio from 0 to 1.",
            labels(&[("source", SOURCE)]),
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_CPU_USAGE_RATIO,
            "Combined CPU utilization ratio from macmon.",
            chip,
            value,
        ));
    }
    if let Some(value) = snapshot.efficiency_cpu_utilization_ratio {
        metrics.push(cpu_cluster_metric(
            names::HARDWARE_UTILIZATION_RATIO,
            "Hardware utilization ratio.",
            "efficiency",
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_ECPU_USAGE_RATIO,
            "Efficiency CPU cluster utilization ratio from macmon.",
            chip,
            value,
        ));
    }
    if let Some(value) = snapshot.performance_cpu_utilization_ratio {
        metrics.push(cpu_cluster_metric(
            names::HARDWARE_UTILIZATION_RATIO,
            "Hardware utilization ratio.",
            "performance",
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_PCPU_USAGE_RATIO,
            "Performance CPU cluster utilization ratio from macmon.",
            chip,
            value,
        ));
    }
    if let Some(value) = snapshot.efficiency_cpu_clock_hertz {
        metrics.push(cpu_cluster_metric(
            names::HARDWARE_CLOCK_HERTZ,
            "Hardware clock speed in hertz.",
            "efficiency",
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_ECPU_FREQUENCY_MHZ,
            "Efficiency CPU cluster frequency in MHz from macmon.",
            chip,
            hertz_to_mhz(value),
        ));
    }
    if let Some(value) = snapshot.performance_cpu_clock_hertz {
        metrics.push(cpu_cluster_metric(
            names::HARDWARE_CLOCK_HERTZ,
            "Hardware clock speed in hertz.",
            "performance",
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_PCPU_FREQUENCY_MHZ,
            "Performance CPU cluster frequency in MHz from macmon.",
            chip,
            hertz_to_mhz(value),
        ));
    }
    if let Some(value) = snapshot.gpu_utilization_ratio {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_UTILIZATION_RATIO,
            "Hardware utilization ratio.",
            labels(&[
                ("component", "gpu"),
                ("gpu_index", "0"),
                ("engine", "graphics"),
                ("source", SOURCE),
            ]),
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_GPU_USAGE_RATIO,
            "GPU utilization ratio from macmon.",
            chip,
            value,
        ));
    }
    if let Some(value) = snapshot.gpu_clock_hertz {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_CLOCK_HERTZ,
            "Hardware clock speed in hertz.",
            labels(&[
                ("component", "gpu"),
                ("gpu_index", "0"),
                ("clock", "graphics"),
                ("source", SOURCE),
            ]),
            value,
        ));
        metrics.push(macmon_metric(
            names::MACMON_GPU_FREQUENCY_MHZ,
            "GPU frequency in MHz from macmon.",
            chip,
            hertz_to_mhz(value),
        ));
    }
    if let Some(value) = snapshot.cpu_temperature_celsius {
        metrics.push(temperature_metric("cpu", value));
        metrics.push(macmon_metric(
            names::MACMON_CPU_TEMP_CELSIUS,
            "Average CPU temperature in Celsius from macmon.",
            chip,
            value,
        ));
    }
    if let Some(value) = snapshot.gpu_temperature_celsius {
        metrics.push(temperature_metric("gpu", value));
        metrics.push(macmon_metric(
            names::MACMON_GPU_TEMP_CELSIUS,
            "Average GPU temperature in Celsius from macmon.",
            chip,
            value,
        ));
    }
    for (component, macmon_name, help, value) in [
        (
            "cpu",
            names::MACMON_CPU_POWER_WATTS,
            "CPU power consumption in watts from macmon.",
            snapshot.cpu_power_watts,
        ),
        (
            "gpu",
            names::MACMON_GPU_POWER_WATTS,
            "GPU power consumption in watts from macmon.",
            snapshot.gpu_power_watts,
        ),
        (
            "ane",
            names::MACMON_ANE_POWER_WATTS,
            "ANE power consumption in watts from macmon.",
            snapshot.ane_power_watts,
        ),
        (
            "soc",
            names::MACMON_ALL_POWER_WATTS,
            "Combined SoC power consumption in watts from macmon.",
            snapshot.soc_power_watts,
        ),
        (
            "system",
            names::MACMON_SYS_POWER_WATTS,
            "System power consumption in watts from macmon.",
            snapshot.system_power_watts,
        ),
        (
            "ram",
            names::MACMON_RAM_POWER_WATTS,
            "RAM power consumption in watts from macmon.",
            snapshot.ram_power_watts,
        ),
        (
            "gpu_ram",
            names::MACMON_GPU_RAM_POWER_WATTS,
            "GPU RAM power consumption in watts from macmon.",
            snapshot.gpu_ram_power_watts,
        ),
    ] {
        if let Some(value) = value {
            metrics.push(power_metric(component, value));
            metrics.push(macmon_metric(macmon_name, help, chip, value));
        }
    }
    if let Some(value) = snapshot.memory_used_bytes {
        metrics.push(memory_metric("used", value));
        metrics.push(macmon_metric(
            names::MACMON_MEMORY_RAM_USED_BYTES,
            "RAM usage in bytes from macmon.",
            chip,
            value as f64,
        ));
    }
    if let Some(value) = snapshot.memory_total_bytes {
        metrics.push(memory_metric("total", value));
        metrics.push(macmon_metric(
            names::MACMON_MEMORY_RAM_TOTAL_BYTES,
            "Total RAM in bytes from macmon.",
            chip,
            value as f64,
        ));
    }
    if let Some(value) = snapshot.swap_used_bytes {
        metrics.push(swap_metric("used", value));
        metrics.push(macmon_metric(
            names::MACMON_MEMORY_SWAP_USED_BYTES,
            "Swap usage in bytes from macmon.",
            chip,
            value as f64,
        ));
    }
    if let Some(value) = snapshot.swap_total_bytes {
        metrics.push(swap_metric("total", value));
        metrics.push(macmon_metric(
            names::MACMON_MEMORY_SWAP_TOTAL_BYTES,
            "Total swap in bytes from macmon.",
            chip,
            value as f64,
        ));
    }

    metrics
}

fn macmon_metric(name: &str, help: &str, chip: Option<&str>, value: f64) -> MetricSample {
    MetricSample::gauge(name, help, macmon_labels(chip), value)
}

fn macmon_labels(chip: Option<&str>) -> std::collections::BTreeMap<String, String> {
    match chip {
        Some(chip) => labels(&[("chip", chip)]),
        None => labels(&[]),
    }
}

fn hertz_to_mhz(value: f64) -> f64 {
    value / 1_000_000.0
}

fn cpu_cluster_metric(name: &str, help: &str, cluster: &str, value: f64) -> MetricSample {
    MetricSample::gauge(
        name,
        help,
        labels(&[
            ("component", "cpu"),
            ("unit", "cluster"),
            ("cluster", cluster),
            ("source", SOURCE),
        ]),
        value,
    )
}

fn temperature_metric(component: &str, value: f64) -> MetricSample {
    MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Average hardware temperature in Celsius.",
        labels(&[
            ("component", component),
            ("sensor", "average"),
            ("source", SOURCE),
        ]),
        value,
    )
}

fn power_metric(component: &str, value: f64) -> MetricSample {
    MetricSample::gauge(
        names::HARDWARE_POWER_WATTS,
        "Hardware power usage in watts.",
        labels(&[("component", component), ("source", SOURCE)]),
        value,
    )
}

fn memory_metric(state: &str, value: u64) -> MetricSample {
    MetricSample::gauge(
        names::MEMORY_TOTAL_BYTES,
        "System memory bytes by state.",
        labels(&[("state", state), ("source", SOURCE)]),
        value as f64,
    )
}

fn swap_metric(state: &str, value: u64) -> MetricSample {
    MetricSample::gauge(
        names::SYSTEM_SWAP_BYTES,
        "System swap bytes by state.",
        labels(&[("state", state), ("source", SOURCE)]),
        value as f64,
    )
}

fn static_info_to_metrics(info: &MacmonStaticInfo) -> Vec<MetricSample> {
    let mut metrics = Vec::new();

    if !info.mac_model.is_empty() || !info.chip_name.is_empty() {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_DEVICE_INFO,
            "Apple Silicon SoC identity information.",
            labels(&[
                ("component", "soc"),
                ("source", SOURCE),
                ("model", info.mac_model.as_str()),
                ("chip_name", info.chip_name.as_str()),
            ]),
            1.0,
        ));
    }
    if info.efficiency_cpu_cores > 0 {
        metrics.push(cpu_cluster_cores_metric(
            "efficiency",
            info.efficiency_cpu_cores,
        ));
    }
    if info.performance_cpu_cores > 0 {
        metrics.push(cpu_cluster_cores_metric(
            "performance",
            info.performance_cpu_cores,
        ));
    }
    if info.gpu_cores > 0 {
        metrics.push(MetricSample::gauge(
            names::HARDWARE_GPU_CORES,
            "Apple Silicon GPU core count.",
            labels(&[("gpu_index", "0"), ("source", SOURCE)]),
            f64::from(info.gpu_cores),
        ));
    }

    metrics.extend(available_clock_metrics(
        "cpu",
        Some("efficiency"),
        None,
        &info.efficiency_cpu_frequencies_mhz,
    ));
    metrics.extend(available_clock_metrics(
        "cpu",
        Some("performance"),
        None,
        &info.performance_cpu_frequencies_mhz,
    ));
    metrics.extend(available_clock_metrics(
        "gpu",
        None,
        Some("0"),
        &info.gpu_frequencies_mhz,
    ));

    metrics
}

fn cpu_cluster_cores_metric(cluster: &str, cores: u8) -> MetricSample {
    MetricSample::gauge(
        names::HARDWARE_CPU_CLUSTER_CORES,
        "Apple Silicon CPU core count by cluster.",
        labels(&[("cluster", cluster), ("source", SOURCE)]),
        f64::from(cores),
    )
}

fn available_clock_metrics(
    component: &str,
    cluster: Option<&str>,
    gpu_index: Option<&str>,
    frequencies_mhz: &[u32],
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    for (index, frequency_mhz) in frequencies_mhz.iter().enumerate() {
        if *frequency_mhz == 0 {
            continue;
        }

        let state = index.to_string();
        let hertz = u64::from(*frequency_mhz) * 1_000_000;
        let mut metric_labels = labels(&[
            ("component", component),
            ("state", state.as_str()),
            ("source", SOURCE),
        ]);
        if let Some(cluster) = cluster {
            metric_labels.insert("cluster".to_string(), cluster.to_string());
        }
        if let Some(gpu_index) = gpu_index {
            metric_labels.insert("gpu_index".to_string(), gpu_index.to_string());
        }

        metrics.push(MetricSample::gauge(
            names::HARDWARE_CLOCK_AVAILABLE_HERTZ,
            "Available Apple Silicon hardware clock speed in hertz.",
            metric_labels,
            hertz as f64,
        ));
    }

    metrics
}

fn platform_supported() -> bool {
    cfg!(all(
        target_os = "macos",
        target_arch = "aarch64",
        feature = "macos-macmon"
    ))
}

#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "macos-macmon"))]
mod platform {
    use super::{MacmonStaticInfo, RawMacmonSnapshot, Result};

    pub struct MacmonPlatformProvider {
        sampler: macmon::Sampler,
    }

    impl MacmonPlatformProvider {
        pub fn new() -> Result<Self> {
            Ok(Self {
                sampler: macmon::Sampler::new().map_err(|error| anyhow::anyhow!("{error}"))?,
            })
        }

        pub fn sample(
            &mut self,
            window_milliseconds: u64,
        ) -> Result<(RawMacmonSnapshot, MacmonStaticInfo)> {
            let duration = u32::try_from(window_milliseconds).unwrap_or(u32::MAX);
            let metrics = self
                .sampler
                .get_metrics(duration)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let static_info = static_info_from_soc(self.sampler.get_soc_info());
            Ok((raw_snapshot_from_metrics(metrics), static_info))
        }
    }

    fn raw_snapshot_from_metrics(metrics: macmon::Metrics) -> RawMacmonSnapshot {
        RawMacmonSnapshot {
            cpu_temperature_celsius: Some(f64::from(metrics.temp.cpu_temp_avg)),
            gpu_temperature_celsius: Some(f64::from(metrics.temp.gpu_temp_avg)),
            cpu_usage_ratio: Some(f64::from(metrics.cpu_usage_pct)),
            efficiency_cpu_utilization_ratio: Some(f64::from(metrics.ecpu_usage.1)),
            performance_cpu_utilization_ratio: Some(f64::from(metrics.pcpu_usage.1)),
            efficiency_cpu_clock_mhz: Some(f64::from(metrics.ecpu_usage.0)),
            performance_cpu_clock_mhz: Some(f64::from(metrics.pcpu_usage.0)),
            gpu_utilization_ratio: Some(f64::from(metrics.gpu_usage.1)),
            gpu_clock_mhz: Some(f64::from(metrics.gpu_usage.0)),
            cpu_power_watts: Some(f64::from(metrics.cpu_power)),
            gpu_power_watts: Some(f64::from(metrics.gpu_power)),
            ane_power_watts: Some(f64::from(metrics.ane_power)),
            soc_power_watts: Some(f64::from(metrics.all_power)),
            system_power_watts: Some(f64::from(metrics.sys_power)),
            ram_power_watts: Some(f64::from(metrics.ram_power)),
            gpu_ram_power_watts: Some(f64::from(metrics.gpu_ram_power)),
            memory_total_bytes: Some(metrics.memory.ram_total),
            memory_used_bytes: Some(metrics.memory.ram_usage),
            swap_total_bytes: Some(metrics.memory.swap_total),
            swap_used_bytes: Some(metrics.memory.swap_usage),
        }
    }

    fn static_info_from_soc(soc: &macmon::SocInfo) -> MacmonStaticInfo {
        MacmonStaticInfo {
            mac_model: soc.mac_model.clone(),
            chip_name: soc.chip_name.clone(),
            efficiency_cpu_cores: soc.ecpu_cores,
            performance_cpu_cores: soc.pcpu_cores,
            efficiency_cpu_frequencies_mhz: soc.ecpu_freqs.clone(),
            performance_cpu_frequencies_mhz: soc.pcpu_freqs.clone(),
            gpu_cores: soc.gpu_cores,
            gpu_frequencies_mhz: soc.gpu_freqs.clone(),
        }
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64", feature = "macos-macmon")))]
mod platform {
    use anyhow::bail;

    use super::{MacmonStaticInfo, RawMacmonSnapshot, Result};

    pub struct MacmonPlatformProvider;

    impl MacmonPlatformProvider {
        pub fn new() -> Result<Self> {
            bail!("macos_macmon is unsupported on this OS or architecture")
        }

        pub fn sample(
            &mut self,
            _window_milliseconds: u64,
        ) -> Result<(RawMacmonSnapshot, MacmonStaticInfo)> {
            bail!("macos_macmon is unsupported on this OS or architecture")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> MacosMacmonConfig {
        MacosMacmonConfig {
            enabled: true,
            sample_interval_seconds: 1,
            sample_window_milliseconds: 1000,
            stale_after_seconds: 5,
            reinitialize_after_consecutive_errors: 5,
            min_temperature_celsius: 1.0,
            max_temperature_celsius: 130.0,
            max_power_watts: 300.0,
        }
    }

    fn metric_value(
        metrics: &[MetricSample],
        name: &str,
        expected_labels: &[(&str, &str)],
    ) -> Option<f64> {
        let expected = labels(expected_labels);
        metrics
            .iter()
            .find(|metric| metric.name == name && metric.labels == expected)
            .map(|metric| metric.value)
    }

    #[test]
    fn converts_mhz_to_hertz() {
        let raw = RawMacmonSnapshot {
            efficiency_cpu_clock_mhz: Some(744.0),
            gpu_clock_mhz: Some(1000.0),
            ..RawMacmonSnapshot::default()
        };
        let normalized = normalize_snapshot(&raw, &test_config());

        assert_eq!(
            normalized.snapshot.efficiency_cpu_clock_hertz,
            Some(744_000_000.0)
        );
        assert_eq!(normalized.snapshot.gpu_clock_hertz, Some(1_000_000_000.0));
    }

    #[test]
    fn validates_and_normalizes_ratios() {
        let raw = RawMacmonSnapshot {
            cpu_usage_ratio: Some(0.25),
            efficiency_cpu_utilization_ratio: Some(50.0),
            performance_cpu_utilization_ratio: Some(125.0),
            ..RawMacmonSnapshot::default()
        };
        let normalized = normalize_snapshot(&raw, &test_config());

        assert_eq!(normalized.snapshot.cpu_usage_ratio, Some(0.25));
        assert_eq!(
            normalized.snapshot.efficiency_cpu_utilization_ratio,
            Some(0.5)
        );
        assert_eq!(normalized.snapshot.performance_cpu_utilization_ratio, None);
        assert_eq!(
            normalized
                .invalid_counts
                .get("performance_cpu_utilization_ratio"),
            Some(&1)
        );
    }

    #[test]
    fn validates_temperature_bounds() {
        let raw = RawMacmonSnapshot {
            cpu_temperature_celsius: Some(63.5),
            gpu_temperature_celsius: Some(0.0),
            ..RawMacmonSnapshot::default()
        };
        let normalized = normalize_snapshot(&raw, &test_config());

        assert_eq!(normalized.snapshot.cpu_temperature_celsius, Some(63.5));
        assert_eq!(normalized.snapshot.gpu_temperature_celsius, None);
        assert_eq!(
            normalized.invalid_counts.get("gpu_temperature_celsius"),
            Some(&1)
        );
    }

    #[test]
    fn validates_power_bounds() {
        let raw = RawMacmonSnapshot {
            cpu_power_watts: Some(7.2),
            gpu_power_watts: Some(301.0),
            ..RawMacmonSnapshot::default()
        };
        let normalized = normalize_snapshot(&raw, &test_config());

        assert_eq!(normalized.snapshot.cpu_power_watts, Some(7.2));
        assert_eq!(normalized.snapshot.gpu_power_watts, None);
        assert_eq!(normalized.invalid_counts.get("gpu_power_watts"), Some(&1));
    }

    #[test]
    fn invalid_fields_are_skipped_without_dropping_snapshot() {
        let raw = RawMacmonSnapshot {
            cpu_temperature_celsius: Some(55.0),
            gpu_temperature_celsius: Some(f64::NAN),
            memory_total_bytes: Some(8),
            memory_used_bytes: Some(9),
            ..RawMacmonSnapshot::default()
        };
        let normalized = normalize_snapshot(&raw, &test_config());
        let metrics = snapshot_to_dynamic_metrics(&normalized.snapshot, None);

        assert!(metric_value(
            &metrics,
            names::TEMPERATURE_CELSIUS,
            &[
                ("component", "cpu"),
                ("sensor", "average"),
                ("source", SOURCE)
            ]
        )
        .is_some());
        assert!(metric_value(
            &metrics,
            names::TEMPERATURE_CELSIUS,
            &[
                ("component", "gpu"),
                ("sensor", "average"),
                ("source", SOURCE)
            ]
        )
        .is_none());
        assert!(metric_value(
            &metrics,
            names::MEMORY_USED_BYTES,
            &[("state", "used"), ("source", SOURCE)]
        )
        .is_none());
    }

    #[test]
    fn renders_dynamic_metrics() {
        let snapshot = MacmonSnapshot {
            cpu_temperature_celsius: Some(63.5),
            gpu_temperature_celsius: Some(59.1),
            cpu_usage_ratio: Some(0.4),
            efficiency_cpu_utilization_ratio: Some(0.2),
            performance_cpu_utilization_ratio: Some(0.6),
            efficiency_cpu_clock_hertz: Some(744_000_000.0),
            performance_cpu_clock_hertz: Some(3_600_000_000.0),
            gpu_utilization_ratio: Some(0.33),
            gpu_clock_hertz: Some(1_000_000_000.0),
            cpu_power_watts: Some(7.2),
            gpu_power_watts: Some(5.8),
            ane_power_watts: Some(0.3),
            soc_power_watts: Some(13.0),
            system_power_watts: Some(18.0),
            ram_power_watts: Some(1.1),
            gpu_ram_power_watts: Some(0.2),
            memory_total_bytes: Some(16),
            memory_used_bytes: Some(6),
            swap_total_bytes: Some(8),
            swap_used_bytes: Some(2),
        };
        let info = MacmonStaticInfo {
            chip_name: "Apple M3 Pro".to_string(),
            ..MacmonStaticInfo::default()
        };
        let metrics = snapshot_to_dynamic_metrics(&snapshot, Some(&info));

        assert_eq!(
            metric_value(&metrics, names::CPU_USAGE_RATIO, &[("source", SOURCE)]),
            Some(0.4)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MACMON_CPU_TEMP_CELSIUS,
                &[("chip", "Apple M3 Pro")]
            ),
            Some(63.5)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MACMON_ECPU_FREQUENCY_MHZ,
                &[("chip", "Apple M3 Pro")]
            ),
            Some(744.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::MACMON_MEMORY_RAM_USED_BYTES,
                &[("chip", "Apple M3 Pro")]
            ),
            Some(6.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::HARDWARE_POWER_WATTS,
                &[("component", "soc"), ("source", SOURCE)]
            ),
            Some(13.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::SYSTEM_SWAP_BYTES,
                &[("state", "used"), ("source", SOURCE)]
            ),
            Some(2.0)
        );
    }

    #[test]
    fn renders_static_metrics() {
        let info = MacmonStaticInfo {
            mac_model: "Mac15,6".to_string(),
            chip_name: "Apple M3 Pro".to_string(),
            efficiency_cpu_cores: 6,
            performance_cpu_cores: 6,
            efficiency_cpu_frequencies_mhz: vec![744, 1536],
            performance_cpu_frequencies_mhz: vec![600, 3600],
            gpu_cores: 18,
            gpu_frequencies_mhz: vec![500, 1000],
        };
        let metrics = static_info_to_metrics(&info);

        assert_eq!(
            metric_value(
                &metrics,
                names::HARDWARE_CPU_CLUSTER_CORES,
                &[("cluster", "efficiency"), ("source", SOURCE)]
            ),
            Some(6.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::HARDWARE_GPU_CORES,
                &[("gpu_index", "0"), ("source", SOURCE)]
            ),
            Some(18.0)
        );
        assert_eq!(
            metric_value(
                &metrics,
                names::HARDWARE_CLOCK_AVAILABLE_HERTZ,
                &[
                    ("cluster", "efficiency"),
                    ("component", "cpu"),
                    ("source", SOURCE),
                    ("state", "1")
                ]
            ),
            Some(1_536_000_000.0)
        );
    }

    #[test]
    fn stale_cache_reports_down_without_dynamic_metrics() {
        let cache = MacmonCache {
            supported: true,
            latest: Some(CachedSnapshot {
                snapshot: MacmonSnapshot {
                    cpu_temperature_celsius: Some(63.5),
                    ..MacmonSnapshot::default()
                },
                captured_at: Instant::now() - Duration::from_secs(10),
                captured_unix_seconds: 1,
            }),
            ..MacmonCache::default()
        };
        let mut collector = MacosMacmonCollector {
            config: test_config(),
            cache: Arc::new(Mutex::new(cache)),
        };
        let result = collector.collect();

        assert!(!result.success);
        assert_eq!(
            metric_value(
                &result.metrics,
                names::COLLECTOR_UP,
                &[("collector", COLLECTOR_NAME)]
            ),
            Some(0.0)
        );
        assert!(result
            .metrics
            .iter()
            .all(|metric| metric.name != names::TEMPERATURE_CELSIUS));
    }

    #[test]
    fn unsupported_platform_reports_unsupported_down() {
        let cache = MacmonCache {
            supported: false,
            ..MacmonCache::default()
        };
        let mut collector = MacosMacmonCollector {
            config: test_config(),
            cache: Arc::new(Mutex::new(cache)),
        };
        let result = collector.collect();

        assert!(result.success);
        assert_eq!(
            metric_value(
                &result.metrics,
                names::COLLECTOR_SUPPORTED,
                &[("collector", COLLECTOR_NAME)]
            ),
            Some(0.0)
        );
        assert_eq!(
            metric_value(
                &result.metrics,
                names::COLLECTOR_UP,
                &[("collector", COLLECTOR_NAME)]
            ),
            Some(0.0)
        );
    }
}
