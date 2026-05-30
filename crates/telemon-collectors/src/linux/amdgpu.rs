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
                    gpu_metrics_status: if card.gpu_metrics_present {
                        "present, diagnostic-only".to_string()
                    } else {
                        "missing".to_string()
                    },
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
        gpu_metrics_present: device_path.join("gpu_metrics").exists(),
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

    if let Some(value) = card.gpu_busy_ratio {
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

    if let Some(value) = card.sclk_hertz {
        metrics.push(clock_metric(&base, "gpu_core_clock", "graphics", value));
    }
    if let Some(value) = card.mclk_hertz {
        metrics.push(clock_metric(&base, "gpu_memory_clock", "memory", value));
    }

    metrics
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
        "Hardware memory bytes by state.",
        metric_labels,
        value as f64,
    )
}

fn clock_metric(base: &[(&str, &str)], sensor: &str, clock: &str, value: f64) -> MetricSample {
    let mut metric_labels = labels(base);
    metric_labels.insert("sensor".to_string(), sensor.to_string());
    metric_labels.insert("clock".to_string(), clock.to_string());
    MetricSample::gauge(
        names::HARDWARE_CLOCK_HERTZ,
        "Hardware clock speed in hertz.",
        metric_labels,
        value,
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

    #[test]
    fn parses_amdgpu_sysfs_metrics() {
        let root = temp_dir("card");
        let device = root.join("card0/device");
        fs::create_dir_all(&device).unwrap();
        write(&device.join("vendor"), "0x1002\n");
        write(&device.join("device"), "0x1435\n");
        write(&device.join("gpu_busy_percent"), "32\n");
        write(&device.join("mem_info_vram_total"), "1000\n");
        write(&device.join("mem_info_vram_used"), "700\n");
        write(&device.join("mem_info_gtt_total"), "2000\n");
        write(&device.join("mem_info_gtt_used"), "500\n");
        write(&device.join("pp_dpm_sclk"), "0: 200Mhz *\n1: 1100Mhz\n");
        write(&device.join("pp_dpm_mclk"), "0: 400Mhz\n1: 800Mhz *\n");
        write(&device.join("gpu_metrics"), "binary-ish\n");

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
                && metric.value == 200_000_000.0
        }));
        assert!(inspect_hardware(&collector.config).cards[0].gpu_metrics_present);

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
