use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::traits::{
    collector_health_metrics, collector_status_metrics, unix_timestamp_seconds, Collector,
    CollectorResult,
};
use telemon_core::config::LinuxAmdgpuConfig;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub const COLLECTOR_NAME: &str = "linux_amdgpu";
pub const SOURCE: &str = "linux_amdgpu";
const SOURCE_DRIVER: &str = "amdgpu";

#[derive(Debug, Clone)]
pub struct LinuxAmdgpuCollector {
    config: LinuxAmdgpuConfig,
    errors_total: u64,
}

#[derive(Debug, Clone, Default)]
struct AmdgpuScan {
    root_exists: bool,
    cards: Vec<AmdgpuCard>,
}

#[derive(Debug, Clone, Default)]
struct AmdgpuCard {
    card: String,
    gpu_index: String,
    device_path: PathBuf,
    pci_vendor_id: Option<String>,
    pci_device_id: Option<String>,
    subsystem_vendor_id: Option<String>,
    subsystem_device_id: Option<String>,
    gpu_busy_ratio: Option<f64>,
    vram_total_bytes: Option<u64>,
    vram_used_bytes: Option<u64>,
    gtt_total_bytes: Option<u64>,
    gtt_used_bytes: Option<u64>,
    sclk_hertz: Option<f64>,
    mclk_hertz: Option<f64>,
    power_dpm_state: Option<String>,
    performance_level: Option<String>,
    power_profile_mode: Option<String>,
    gpu_metrics_present: bool,
    gpu_metrics_status: String,
    gpu_metrics: Option<AmdgpuGpuMetrics>,
}

#[derive(Debug, Clone, Default)]
struct AmdgpuGpuMetrics {
    format_revision: u8,
    content_revision: u8,
    gpu_temperature_celsius: Option<f64>,
    soc_temperature_celsius: Option<f64>,
    cpu_temperature_celsius: Option<f64>,
    gpu_power_watts: Option<f64>,
    cpu_power_watts: Option<f64>,
    gpu_busy_ratio: Option<f64>,
    gpu_clock_hertz: Option<f64>,
    memory_clock_hertz: Option<f64>,
    power_throttled: Option<bool>,
    current_throttled: Option<bool>,
    thermal_throttled: Option<bool>,
    other_throttled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxAmdgpuInspection {
    pub root: String,
    pub root_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_error: Option<String>,
    pub cards: Vec<LinuxAmdgpuCardInspection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinuxAmdgpuCardInspection {
    pub card: String,
    pub device_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_vendor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_busy_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vram_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vram_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtt_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtt_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sclk_hertz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mclk_hertz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_dpm_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_profile_mode: Option<String>,
    pub gpu_metrics_present: bool,
    pub gpu_metrics_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_format_revision: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_content_revision: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_cpu_temperature_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_gpu_temperature_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_gpu_power_watts: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_metrics_cpu_power_watts: Option<f64>,
}

impl LinuxAmdgpuCollector {
    pub fn new(config: LinuxAmdgpuConfig) -> Self {
        Self {
            config,
            errors_total: 0,
        }
    }

    pub fn discover_summary(config: &LinuxAmdgpuConfig) -> String {
        let inspection = inspect_hardware(config);
        if !inspection.root_exists {
            return "unavailable, root missing".to_string();
        }
        if let Some(error) = inspection.root_error {
            return format!("error: {error}");
        }
        format!("available, amd_cards={}", inspection.cards.len())
    }
}

impl Collector for LinuxAmdgpuCollector {
    fn name(&self) -> &'static str {
        COLLECTOR_NAME
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = Instant::now();

        match scan_amdgpu(&self.config) {
            Ok(scan) => {
                let supported = scan.root_exists && !scan.cards.is_empty();
                let mut metrics = if supported {
                    collector_health_metrics(
                        COLLECTOR_NAME,
                        true,
                        self.errors_total,
                        Some(unix_timestamp_seconds()),
                    )
                } else {
                    collector_status_metrics(COLLECTOR_NAME, false, false, self.errors_total, None)
                };
                for card in scan.cards {
                    metrics.extend(card_to_metrics(&card));
                }
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

pub fn inspect_hardware(config: &LinuxAmdgpuConfig) -> LinuxAmdgpuInspection {
    let mut inspection = LinuxAmdgpuInspection {
        root: config.root.display().to_string(),
        root_exists: config.root.exists(),
        root_error: None,
        cards: Vec::new(),
    };

    if !inspection.root_exists {
        return inspection;
    }

    match scan_amdgpu(config) {
        Ok(scan) => {
            inspection.cards = scan
                .cards
                .into_iter()
                .map(|card| LinuxAmdgpuCardInspection {
                    card: card.card,
                    device_path: card.device_path.display().to_string(),
                    pci_vendor_id: card.pci_vendor_id,
                    pci_device_id: card.pci_device_id,
                    gpu_busy_ratio: card.gpu_busy_ratio,
                    vram_total_bytes: card.vram_total_bytes,
                    vram_used_bytes: card.vram_used_bytes,
                    gtt_total_bytes: card.gtt_total_bytes,
                    gtt_used_bytes: card.gtt_used_bytes,
                    sclk_hertz: card.sclk_hertz,
                    mclk_hertz: card.mclk_hertz,
                    power_dpm_state: card.power_dpm_state,
                    performance_level: card.performance_level,
                    power_profile_mode: card.power_profile_mode,
                    gpu_metrics_present: card.gpu_metrics_present,
                    gpu_metrics_status: card.gpu_metrics_status,
                    gpu_metrics_format_revision: card
                        .gpu_metrics
                        .as_ref()
                        .map(|metrics| metrics.format_revision),
                    gpu_metrics_content_revision: card
                        .gpu_metrics
                        .as_ref()
                        .map(|metrics| metrics.content_revision),
                    gpu_metrics_cpu_temperature_celsius: card
                        .gpu_metrics
                        .as_ref()
                        .and_then(|metrics| metrics.cpu_temperature_celsius),
                    gpu_metrics_gpu_temperature_celsius: card
                        .gpu_metrics
                        .as_ref()
                        .and_then(|metrics| metrics.gpu_temperature_celsius),
                    gpu_metrics_gpu_power_watts: card
                        .gpu_metrics
                        .as_ref()
                        .and_then(|metrics| metrics.gpu_power_watts),
                    gpu_metrics_cpu_power_watts: card
                        .gpu_metrics
                        .as_ref()
                        .and_then(|metrics| metrics.cpu_power_watts),
                })
                .collect();
        }
        Err(error) => inspection.root_error = Some(error.to_string()),
    }

    inspection
}

fn scan_amdgpu(config: &LinuxAmdgpuConfig) -> Result<AmdgpuScan> {
    if !config.root.exists() {
        return Ok(AmdgpuScan {
            root_exists: false,
            cards: Vec::new(),
        });
    }

    let mut entries = fs::read_dir(&config.root)
        .with_context(|| format!("failed to read DRM root {}", config.root.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    let mut cards = Vec::new();
    for entry in entries {
        let card_path = entry.path();
        let Some(card_name) = card_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_drm_card_name(card_name) {
            continue;
        }
        let device_path = card_path.join("device");
        if read_trimmed(device_path.join("vendor")).as_deref() != Some("0x1002") {
            continue;
        }
        cards.push(read_card(card_name.to_string(), device_path));
    }

    Ok(AmdgpuScan {
        root_exists: true,
        cards,
    })
}

fn read_card(card: String, device_path: PathBuf) -> AmdgpuCard {
    let gpu_index = card.trim_start_matches("card").to_string();
    let (gpu_metrics_present, gpu_metrics_status, gpu_metrics) =
        read_gpu_metrics(device_path.join("gpu_metrics"));
    let gpu_busy_ratio = read_f64(device_path.join("gpu_busy_percent"))
        .filter(|value| (0.0..=100.0).contains(value))
        .map(|value| value / 100.0);
    let vram_total_bytes = read_u64(device_path.join("mem_info_vram_total"));
    let vram_used_bytes = read_u64(device_path.join("mem_info_vram_used"));
    let gtt_total_bytes = read_u64(device_path.join("mem_info_gtt_total"));
    let gtt_used_bytes = read_u64(device_path.join("mem_info_gtt_used"));

    AmdgpuCard {
        card,
        gpu_index,
        pci_vendor_id: read_trimmed(device_path.join("vendor")),
        pci_device_id: read_trimmed(device_path.join("device")),
        subsystem_vendor_id: read_trimmed(device_path.join("subsystem_vendor")),
        subsystem_device_id: read_trimmed(device_path.join("subsystem_device")),
        gpu_busy_ratio,
        vram_total_bytes,
        vram_used_bytes,
        gtt_total_bytes,
        gtt_used_bytes,
        sclk_hertz: read_trimmed(device_path.join("pp_dpm_sclk"))
            .and_then(|value| parse_active_dpm_clock_hertz(&value)),
        mclk_hertz: read_trimmed(device_path.join("pp_dpm_mclk"))
            .and_then(|value| parse_active_dpm_clock_hertz(&value)),
        power_dpm_state: read_trimmed(device_path.join("power_dpm_state")),
        performance_level: read_trimmed(device_path.join("power_dpm_force_performance_level")),
        power_profile_mode: read_trimmed(device_path.join("pp_power_profile_mode"))
            .and_then(|value| active_power_profile_mode(&value)),
        gpu_metrics_present,
        gpu_metrics_status,
        gpu_metrics,
        device_path,
    }
}

fn card_to_metrics(card: &AmdgpuCard) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    let base = [
        ("component", "gpu"),
        ("device_id", card.card.as_str()),
        ("gpu_index", card.gpu_index.as_str()),
        ("source", SOURCE),
        ("source_driver", SOURCE_DRIVER),
    ];

    let mut info = labels(&base);
    info.insert("vendor".to_string(), "amd".to_string());
    if let Some(value) = &card.pci_vendor_id {
        info.insert("pci_vendor_id".to_string(), value.clone());
    }
    if let Some(value) = &card.pci_device_id {
        info.insert("pci_device_id".to_string(), value.clone());
    }
    if let Some(value) = &card.subsystem_vendor_id {
        info.insert("pci_subsystem_vendor_id".to_string(), value.clone());
    }
    if let Some(value) = &card.subsystem_device_id {
        info.insert("pci_subsystem_device_id".to_string(), value.clone());
    }
    metrics.push(MetricSample::gauge(
        names::HARDWARE_DEVICE_INFO,
        "Hardware device identity information.",
        info,
        1.0,
    ));

    if let Some(value) = card
        .gpu_metrics
        .as_ref()
        .and_then(|gpu_metrics| gpu_metrics.gpu_busy_ratio)
        .or(card.gpu_busy_ratio)
    {
        let mut metric_labels = labels(&base);
        metric_labels.insert("sensor".to_string(), "gpu_busy".to_string());
        metric_labels.insert("engine".to_string(), "graphics".to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_UTILIZATION_RATIO,
            "Hardware utilization as a ratio from 0 to 1.",
            metric_labels,
            value,
        ));
    }

    if let Some(gpu_metrics) = &card.gpu_metrics {
        metrics.extend(gpu_metrics_to_metrics(card, &base, gpu_metrics));
    }

    push_memory_metrics(
        &mut metrics,
        &base,
        "vram",
        card.vram_total_bytes,
        card.vram_used_bytes,
    );
    push_memory_metrics(
        &mut metrics,
        &base,
        "gtt",
        card.gtt_total_bytes,
        card.gtt_used_bytes,
    );

    let prefer_gpu_metrics_clock = matches!(
        card.pci_device_id.as_deref(),
        Some("0x1435") | Some("0x163f")
    );
    let gpu_metrics_sclk = card
        .gpu_metrics
        .as_ref()
        .and_then(|gpu_metrics| gpu_metrics.gpu_clock_hertz);
    let gpu_metrics_mclk = card
        .gpu_metrics
        .as_ref()
        .and_then(|gpu_metrics| gpu_metrics.memory_clock_hertz);
    let sclk = if prefer_gpu_metrics_clock {
        gpu_metrics_sclk.or(card.sclk_hertz)
    } else {
        card.sclk_hertz.or(gpu_metrics_sclk)
    };
    let mclk = if prefer_gpu_metrics_clock {
        gpu_metrics_mclk.or(card.mclk_hertz)
    } else {
        card.mclk_hertz.or(gpu_metrics_mclk)
    };
    if let Some(value) = sclk {
        metrics.push(clock_metric(&base, "gpu_core_clock", "graphics", value));
    }
    if let Some(value) = mclk {
        metrics.push(clock_metric(&base, "gpu_memory_clock", "memory", value));
    }

    metrics
}

fn gpu_metrics_to_metrics(
    card: &AmdgpuCard,
    gpu_base: &[(&str, &str)],
    gpu_metrics: &AmdgpuGpuMetrics,
) -> Vec<MetricSample> {
    let mut metrics = Vec::new();
    if let Some(value) = gpu_metrics.cpu_temperature_celsius {
        metrics.push(component_temperature_metric(
            card,
            "cpu",
            "apu_cpu_package_temp",
            "max_core",
            value,
        ));
    }
    if let Some(value) = gpu_metrics.gpu_temperature_celsius {
        metrics.push(component_temperature_metric(
            card,
            "gpu",
            "gpu_edge_temp",
            "gpu_metrics",
            value,
        ));
    }
    if let Some(value) = gpu_metrics.soc_temperature_celsius {
        metrics.push(component_temperature_metric(
            card,
            "system",
            "apu_soc_temp",
            "soc",
            value,
        ));
    }
    if let Some(value) = gpu_metrics.gpu_power_watts {
        metrics.push(power_metric(gpu_base, "gpu_power", "current", value));
    }
    if let Some(value) = gpu_metrics.cpu_power_watts {
        let mut cpu_base = labels(gpu_base);
        cpu_base.insert("component".to_string(), "cpu".to_string());
        metrics.push(MetricSample::gauge(
            names::HARDWARE_POWER_WATTS,
            "Hardware power in watts.",
            power_labels(cpu_base, "apu_cpu_power", "current"),
            value,
        ));
    }
    for (state, value) in [
        ("power", gpu_metrics.power_throttled),
        ("current", gpu_metrics.current_throttled),
        ("thermal", gpu_metrics.thermal_throttled),
        ("other", gpu_metrics.other_throttled),
    ] {
        if let Some(value) = value {
            let mut metric_labels = labels(gpu_base);
            metric_labels.insert("sensor".to_string(), "gpu_throttle".to_string());
            metric_labels.insert("state".to_string(), state.to_string());
            metrics.push(MetricSample::gauge(
                names::HARDWARE_STATE,
                "Hardware numeric state value.",
                metric_labels,
                if value { 1.0 } else { 0.0 },
            ));
        }
    }
    metrics
}

fn component_temperature_metric(
    card: &AmdgpuCard,
    component: &str,
    sensor: &str,
    sensor_instance: &str,
    value: f64,
) -> MetricSample {
    let gpu_index = card.gpu_index.as_str();
    let mut metric_labels = labels(&[
        ("component", component),
        ("device_id", card.card.as_str()),
        ("gpu_index", gpu_index),
        ("sensor", sensor),
        ("sensor_instance", sensor_instance),
        ("source", SOURCE),
        ("source_driver", SOURCE_DRIVER),
    ]);
    if let Some(value) = &card.pci_vendor_id {
        metric_labels.insert("pci_vendor_id".to_string(), value.clone());
    }
    if let Some(value) = &card.pci_device_id {
        metric_labels.insert("pci_device_id".to_string(), value.clone());
    }
    MetricSample::gauge(
        names::TEMPERATURE_CELSIUS,
        "Hardware temperature reading in degrees Celsius.",
        metric_labels,
        value,
    )
}

fn power_metric(
    base: &[(&str, &str)],
    sensor: &str,
    sensor_instance: &str,
    value: f64,
) -> MetricSample {
    MetricSample::gauge(
        names::HARDWARE_POWER_WATTS,
        "Hardware power in watts.",
        power_labels(labels(base), sensor, sensor_instance),
        value,
    )
}

fn power_labels(
    mut metric_labels: std::collections::BTreeMap<String, String>,
    sensor: &str,
    sensor_instance: &str,
) -> std::collections::BTreeMap<String, String> {
    metric_labels.insert("sensor".to_string(), sensor.to_string());
    metric_labels.insert("sensor_instance".to_string(), sensor_instance.to_string());
    metric_labels
}

fn push_memory_metrics(
    metrics: &mut Vec<MetricSample>,
    base: &[(&str, &str)],
    memory: &str,
    total: Option<u64>,
    used: Option<u64>,
) {
    if let Some(total) = total {
        metrics.push(memory_metric(base, memory, "total", total));
    }
    if let Some(used) = used {
        metrics.push(memory_metric(base, memory, "used", used));
    }
    if let (Some(total), Some(used)) = (total, used) {
        if used <= total {
            metrics.push(memory_metric(base, memory, "free", total - used));
        }
    }
}

fn memory_metric(base: &[(&str, &str)], memory: &str, state: &str, value: u64) -> MetricSample {
    let mut metric_labels = labels(base);
    metric_labels.insert("memory".to_string(), memory.to_string());
    metric_labels.insert("state".to_string(), state.to_string());
    MetricSample::gauge(
        names::HARDWARE_MEMORY_BYTES,
        "Hardware memory in decimal megabytes by state.",
        metric_labels,
        bytes_to_mb(value),
    )
}

fn bytes_to_mb(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

fn clock_metric(base: &[(&str, &str)], sensor: &str, clock: &str, value: f64) -> MetricSample {
    let mut metric_labels = labels(base);
    metric_labels.insert("sensor".to_string(), sensor.to_string());
    metric_labels.insert("clock".to_string(), clock.to_string());
    MetricSample::gauge(
        names::HARDWARE_CLOCK_HERTZ,
        "Hardware frequency in decimal megahertz.",
        metric_labels,
        value / 1_000_000.0,
    )
}

fn is_drm_card_name(value: &str) -> bool {
    value
        .strip_prefix("card")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

fn parse_active_dpm_clock_hertz(text: &str) -> Option<f64> {
    text.lines().find_map(|line| {
        if !line.contains('*') {
            return None;
        }
        let value = line.split(':').nth(1)?.replace('*', "");
        parse_mhz_to_hertz(value.trim())
    })
}

fn parse_mhz_to_hertz(value: &str) -> Option<f64> {
    let normalized = value
        .trim()
        .trim_end_matches("MHz")
        .trim_end_matches("Mhz")
        .trim_end_matches("mhz")
        .trim();
    let mhz = normalized.parse::<f64>().ok()?;
    (mhz.is_finite() && mhz >= 0.0).then_some(mhz * 1_000_000.0)
}

fn active_power_profile_mode(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        if !line.contains('*') {
            return None;
        }
        Some(
            line.replace('*', "")
                .split_whitespace()
                .last()?
                .to_ascii_lowercase(),
        )
    })
}

fn read_gpu_metrics(path: PathBuf) -> (bool, String, Option<AmdgpuGpuMetrics>) {
    if !path.exists() {
        return (false, "missing".to_string(), None);
    }
    match fs::read(&path) {
        Ok(bytes) => match parse_gpu_metrics(&bytes) {
            Some(metrics) => (true, "parsed".to_string(), Some(metrics)),
            None => (true, "present, unsupported layout".to_string(), None),
        },
        Err(error) => (true, format!("read_error: {error}"), None),
    }
}

fn parse_gpu_metrics(bytes: &[u8]) -> Option<AmdgpuGpuMetrics> {
    if bytes.len() < 4 {
        return None;
    }
    let structure_size = read_u16_le(bytes, 0).unwrap_or(0) as usize;
    if structure_size > bytes.len() {
        return None;
    }
    let format_revision = bytes[2];
    let content_revision = bytes[3];
    match format_revision {
        2 => parse_gpu_metrics_v2(bytes, content_revision),
        3 => parse_gpu_metrics_v3(bytes, content_revision),
        _ => None,
    }
}

fn parse_gpu_metrics_v2(bytes: &[u8], content_revision: u8) -> Option<AmdgpuGpuMetrics> {
    let mut metrics = AmdgpuGpuMetrics {
        format_revision: 2,
        content_revision,
        ..AmdgpuGpuMetrics::default()
    };

    if content_revision == 0 {
        metrics.gpu_temperature_celsius = read_centi_celsius(bytes, 16);
        metrics.soc_temperature_celsius = read_centi_celsius(bytes, 18);
        metrics.cpu_temperature_celsius =
            max_centi_celsius(bytes, 20, 8).or_else(|| max_centi_celsius(bytes, 52, 8));
        metrics.gpu_busy_ratio = read_percent_ratio(bytes, 40);
        metrics.cpu_power_watts =
            read_milliwatts(bytes, 46).or_else(|| sum_milliwatts(bytes, 52, 8));
        metrics.gpu_power_watts = read_milliwatts(bytes, 50);
        metrics.gpu_clock_hertz = read_mhz_hertz(bytes, 84).or_else(|| read_mhz_hertz(bytes, 72));
        metrics.memory_clock_hertz =
            read_mhz_hertz(bytes, 88).or_else(|| read_mhz_hertz(bytes, 76));
        return Some(metrics);
    }

    metrics.gpu_temperature_celsius =
        read_centi_celsius(bytes, 4).or_else(|| read_centi_celsius(bytes, 128));
    metrics.soc_temperature_celsius =
        read_centi_celsius(bytes, 6).or_else(|| read_centi_celsius(bytes, 130));
    metrics.cpu_temperature_celsius =
        max_centi_celsius(bytes, 8, 8).or_else(|| max_centi_celsius(bytes, 132, 8));
    metrics.gpu_busy_ratio = read_percent_ratio(bytes, 28);
    metrics.cpu_power_watts = read_milliwatts(bytes, 42).or_else(|| sum_milliwatts(bytes, 48, 8));
    metrics.gpu_power_watts = read_milliwatts(bytes, 46);
    metrics.gpu_clock_hertz = read_mhz_hertz(bytes, 80).or_else(|| read_mhz_hertz(bytes, 68));
    metrics.memory_clock_hertz = read_mhz_hertz(bytes, 84).or_else(|| read_mhz_hertz(bytes, 72));

    let throttle = read_u64_le(bytes, 120).or_else(|| read_u32_le(bytes, 112).map(u64::from));
    if let Some(throttle) = throttle {
        metrics.power_throttled = Some((throttle & 0x0000_0000_0000_00ff) != 0);
        metrics.current_throttled = Some((throttle & 0x0000_0000_00ff_0000) != 0);
        metrics.thermal_throttled = Some((throttle & 0x0000_ffff_0000_0000) != 0);
        metrics.other_throttled = Some((throttle & 0xff00_0000_0000_0000) != 0);
    }

    Some(metrics)
}

fn parse_gpu_metrics_v3(bytes: &[u8], content_revision: u8) -> Option<AmdgpuGpuMetrics> {
    let mut metrics = AmdgpuGpuMetrics {
        format_revision: 3,
        content_revision,
        ..AmdgpuGpuMetrics::default()
    };
    metrics.gpu_temperature_celsius = read_centi_celsius(bytes, 4);
    metrics.soc_temperature_celsius = read_centi_celsius(bytes, 6);
    metrics.cpu_temperature_celsius = max_centi_celsius(bytes, 8, 16);
    metrics.gpu_busy_ratio = read_percent_ratio(bytes, 42);
    metrics.gpu_power_watts = read_milliwatts_u32(bytes, 124);
    metrics.cpu_power_watts = match (read_u32_le(bytes, 120), read_u32_le(bytes, 124)) {
        (Some(apu), Some(gpu)) if apu >= gpu && apu != u32::MAX && gpu != u32::MAX => {
            Some((apu - gpu) as f64 / 1_000.0)
        }
        _ => read_milliwatts_u32(bytes, 132),
    };
    metrics.gpu_clock_hertz = read_mhz_hertz(bytes, 174);
    metrics.memory_clock_hertz = read_mhz_hertz(bytes, 186);
    Some(metrics)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn valid_u16(value: u16) -> Option<u16> {
    (value != u16::MAX).then_some(value)
}

fn read_centi_celsius(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = valid_u16(read_u16_le(bytes, offset)?)? as f64 / 100.0;
    (value.is_finite() && (-20.0..=250.0).contains(&value)).then_some(value)
}

fn max_centi_celsius(bytes: &[u8], offset: usize, count: usize) -> Option<f64> {
    (0..count)
        .filter_map(|index| read_centi_celsius(bytes, offset + index * 2))
        .max_by(|left, right| left.total_cmp(right))
}

fn read_percent_ratio(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = valid_u16(read_u16_le(bytes, offset)?)? as f64;
    (0.0..=100.0).contains(&value).then_some(value / 100.0)
}

fn read_milliwatts(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = valid_u16(read_u16_le(bytes, offset)?)? as f64 / 1_000.0;
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn sum_milliwatts(bytes: &[u8], offset: usize, count: usize) -> Option<f64> {
    let mut sum = 0.0;
    let mut found = false;
    for index in 0..count {
        if let Some(value) = read_milliwatts(bytes, offset + index * 2) {
            sum += value;
            found = true;
        }
    }
    found.then_some(sum)
}

fn read_milliwatts_u32(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = read_u32_le(bytes, offset)?;
    if value == u32::MAX {
        return None;
    }
    let value = value as f64 / 1_000.0;
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn read_mhz_hertz(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = valid_u16(read_u16_le(bytes, offset)?)? as f64;
    (value.is_finite() && value >= 0.0).then_some(value * 1_000_000.0)
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn read_u64(path: PathBuf) -> Option<u64> {
    read_trimmed(path)?.parse::<u64>().ok()
}

fn read_f64(path: PathBuf) -> Option<f64> {
    read_trimmed(path)?.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "telemon-amdgpu-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write(path: &Path, value: &str) {
        fs::write(path, value).unwrap();
    }

    fn write_u16_le(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn gpu_metrics_v2_1_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 128];
        write_u16_le(&mut bytes, 0, 128);
        bytes[2] = 2;
        bytes[3] = 1;
        write_u16_le(&mut bytes, 4, 4_100);
        write_u16_le(&mut bytes, 6, 4_300);
        write_u16_le(&mut bytes, 8, 5_500);
        write_u16_le(&mut bytes, 10, 5_100);
        write_u16_le(&mut bytes, 28, 32);
        write_u16_le(&mut bytes, 42, 12_000);
        write_u16_le(&mut bytes, 46, 7_000);
        write_u16_le(&mut bytes, 80, 200);
        write_u16_le(&mut bytes, 84, 400);
        bytes[120..128].copy_from_slice(&0x0000_0001_0000_0060_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_amdgpu_sysfs_metrics() {
        let root = temp_dir("card");
        let device = root.join("card0/device");
        fs::create_dir_all(&device).unwrap();
        write(&device.join("vendor"), "0x1002\n");
        write(&device.join("device"), "0x1435\n");
        write(&device.join("gpu_busy_percent"), "32\n");
        write(&device.join("mem_info_vram_total"), "1000000000\n");
        write(&device.join("mem_info_vram_used"), "700000000\n");
        write(&device.join("mem_info_gtt_total"), "2000\n");
        write(&device.join("mem_info_gtt_used"), "500\n");
        write(&device.join("pp_dpm_sclk"), "0: 200Mhz *\n1: 1100Mhz\n");
        write(&device.join("pp_dpm_mclk"), "0: 400Mhz\n1: 800Mhz *\n");
        fs::write(device.join("gpu_metrics"), gpu_metrics_v2_1_fixture()).unwrap();

        let mut collector = LinuxAmdgpuCollector::new(LinuxAmdgpuConfig {
            enabled: true,
            root: root.clone(),
            include_diagnostic_only_gpu_metrics: true,
        });
        let result = collector.collect();

        assert!(result.success);
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_UTILIZATION_RATIO && (metric.value - 0.32).abs() < 0.001
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_MEMORY_BYTES
                && metric.labels.get("memory").map(String::as_str) == Some("vram")
                && metric.labels.get("state").map(String::as_str) == Some("free")
                && metric.value == 300.0
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_CLOCK_HERTZ
                && metric.labels.get("clock").map(String::as_str) == Some("graphics")
                && metric.value == 200.0
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::TEMPERATURE_CELSIUS
                && metric.labels.get("component").map(String::as_str) == Some("cpu")
                && metric.labels.get("sensor").map(String::as_str) == Some("apu_cpu_package_temp")
                && (metric.value - 55.0).abs() < 0.001
        }));
        assert!(result.metrics.iter().any(|metric| {
            metric.name == names::HARDWARE_POWER_WATTS
                && metric.labels.get("component").map(String::as_str) == Some("cpu")
                && (metric.value - 12.0).abs() < 0.001
        }));
        let inspection = inspect_hardware(&collector.config);
        assert!(inspection.cards[0].gpu_metrics_present);
        assert_eq!(inspection.cards[0].gpu_metrics_status, "parsed");
        assert_eq!(
            inspection.cards[0].gpu_metrics_cpu_temperature_celsius,
            Some(55.0)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_non_amd_drm_cards() {
        let root = temp_dir("intel");
        let device = root.join("card0/device");
        fs::create_dir_all(&device).unwrap();
        write(&device.join("vendor"), "0x8086\n");

        let mut collector = LinuxAmdgpuCollector::new(LinuxAmdgpuConfig {
            enabled: true,
            root: root.clone(),
            include_diagnostic_only_gpu_metrics: true,
        });
        let result = collector.collect();

        assert!(result.success);
        assert!(result
            .metrics
            .iter()
            .any(|metric| { metric.name == names::COLLECTOR_SUPPORTED && metric.value == 0.0 }));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_active_dpm_clock() {
        assert_eq!(
            parse_active_dpm_clock_hertz("0: 400Mhz\n1: 800Mhz *\n"),
            Some(800_000_000.0)
        );
    }
}
