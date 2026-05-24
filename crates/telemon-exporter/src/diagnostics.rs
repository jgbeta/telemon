use std::time::Duration;

use crate::registration::DeviceInfoCollector;
use crate::scheduler::{collect_snapshot_once, ScheduledCollector};
use telemon_collectors::gpu::nvidia::collector::NvidiaNvmlCollector;
use telemon_collectors::temperature::linux_hwmon::LinuxHwmonCollector;
use telemon_collectors::windows::baseline::WindowsBaselineCollector;
use telemon_collectors::windows::inventory::WindowsInventoryCollector;
use telemon_collectors::windows::lhm_http::WindowsLhmHttpCollector;
use telemon_collectors::windows::lhm_wmi::WindowsLhmWmiCollector;
use telemon_core::config::AppConfig;
use telemon_core::metrics::encode;
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

pub fn build_info_metric() -> MetricSample {
    MetricSample::gauge(
        names::BUILD_INFO,
        "Build information for the Telemon exporter.",
        labels(&[
            ("version", env!("CARGO_PKG_VERSION")),
            ("os", std::env::consts::OS),
            ("arch", std::env::consts::ARCH),
        ]),
        1.0,
    )
}

pub fn build_scheduled_collectors(config: &AppConfig) -> Vec<ScheduledCollector> {
    let mut collectors = Vec::new();

    if config.collectors.linux_hwmon.enabled {
        collectors.push(
            ScheduledCollector::new(
                Box::new(LinuxHwmonCollector::new(
                    config.collectors.linux_hwmon.clone(),
                )),
                Duration::from_secs(config.collection.temperature_interval_seconds),
            )
            .adaptive(config.adaptive_sampling.levels.normal_seconds),
        );
    }

    if config.collectors.nvidia_nvml.enabled {
        collectors.push(
            ScheduledCollector::new(
                Box::new(NvidiaNvmlCollector::new(
                    config.collectors.nvidia_nvml.clone(),
                )),
                Duration::from_secs(config.collection.gpu_interval_seconds),
            )
            .adaptive(config.adaptive_sampling.levels.normal_seconds),
        );
    }

    if config.collectors.windows_lhm_http.enabled {
        collectors.push(
            ScheduledCollector::new(
                Box::new(WindowsLhmHttpCollector::new(
                    config.collectors.windows_lhm_http.clone(),
                )),
                Duration::from_secs(config.collection.temperature_interval_seconds),
            )
            .adaptive(config.adaptive_sampling.levels.normal_seconds),
        );
    }

    if config.collectors.windows_lhm_wmi.enabled {
        collectors.push(
            ScheduledCollector::new(
                Box::new(WindowsLhmWmiCollector::new(
                    config.collectors.windows_lhm_wmi.clone(),
                )),
                Duration::from_secs(config.collection.temperature_interval_seconds),
            )
            .adaptive(config.adaptive_sampling.levels.normal_seconds),
        );
    }

    if config.collectors.windows_baseline.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(WindowsBaselineCollector::new(
                config.collectors.windows_baseline.clone(),
            )),
            Duration::from_secs(config.collection.windows_baseline_interval_seconds),
        ));
    }

    if config.collectors.windows_inventory.enabled {
        collectors.push(ScheduledCollector::static_collector(
            Box::new(WindowsInventoryCollector::new(
                config.collectors.windows_inventory.clone(),
            )),
            Duration::from_secs(config.collection.windows_inventory_interval_seconds),
        ));
    }

    if config.registration.enabled {
        collectors.push(ScheduledCollector::static_collector(
            Box::new(DeviceInfoCollector::new(config.clone())),
            Duration::from_secs(config.collection.static_info_interval_seconds),
        ));
    }

    collectors
}

pub fn check_report(config: &AppConfig) -> String {
    let mut report = String::new();
    report.push_str("config: ok\n");
    report.push_str(&format!("listen: {}\n", config.server.listen));
    report.push_str("enabled collectors:\n");

    if config.collectors.linux_hwmon.enabled {
        report.push_str("- linux_hwmon\n");
    }
    if config.collectors.nvidia_nvml.enabled {
        report.push_str("- nvidia_nvml\n");
    }
    if config.collectors.windows_baseline.enabled {
        report.push_str("- windows_baseline\n");
    }
    if config.collectors.windows_lhm_http.enabled {
        report.push_str("- windows_lhm_http\n");
    }
    if config.collectors.windows_lhm_wmi.enabled {
        report.push_str("- windows_lhm_wmi\n");
    }
    if config.collectors.windows_inventory.enabled {
        report.push_str("- windows_inventory\n");
    }
    if config.registration.enabled {
        report.push_str("- identity\n");
    }
    if !config.collectors.linux_hwmon.enabled
        && !config.collectors.nvidia_nvml.enabled
        && !config.collectors.windows_baseline.enabled
        && !config.collectors.windows_lhm_http.enabled
        && !config.collectors.windows_lhm_wmi.enabled
        && !config.collectors.windows_inventory.enabled
        && !config.registration.enabled
    {
        report.push_str("- none\n");
    }

    report
}

pub fn print_metrics(config: &AppConfig) -> String {
    let mut collectors = build_scheduled_collectors(config);
    let snapshot = collect_snapshot_once(&mut collectors, &config.adaptive_sampling);
    encode::encode(&snapshot)
}

pub fn discover_report(config: &AppConfig) -> String {
    let mut report = String::new();
    report.push_str("collectors:\n");
    let linux_state = LinuxHwmonCollector::discover_summary(&config.collectors.linux_hwmon);
    report.push_str(&format!(
        "- linux_hwmon: {}, enabled={}, root={}, include_unknown_sensors={}\n",
        linux_state,
        config.collectors.linux_hwmon.enabled,
        config.collectors.linux_hwmon.root.display(),
        config.collectors.linux_hwmon.include_unknown_sensors
    ));

    let nvidia_state = NvidiaNvmlCollector::discover_summary(&config.collectors.nvidia_nvml);
    report.push_str("- nvidia_nvml:\n");
    report.push_str(&format!("    enabled: {}\n", nvidia_state.enabled));
    report.push_str(&format!("    supported: {}\n", nvidia_state.supported));
    report.push_str(&format!(
        "    library_loaded: {}\n",
        nvidia_state.library_loaded
    ));
    report.push_str(&format!(
        "    device_count: {}\n",
        nvidia_state.device_count
    ));
    report.push_str(&format!("    status: {}\n", nvidia_state.status));
    if let Some(message) = nvidia_state.message {
        report.push_str(&format!("    message: {}\n", message));
    }
    let windows_baseline_state =
        WindowsBaselineCollector::discover_summary(&config.collectors.windows_baseline);
    report.push_str(&format!(
        "- windows_baseline: {}, enabled={}\n",
        windows_baseline_state, config.collectors.windows_baseline.enabled
    ));

    let windows_lhm_http_state =
        WindowsLhmHttpCollector::discover_summary(&config.collectors.windows_lhm_http);
    report.push_str("- windows_lhm_http:\n");
    report.push_str(&format!(
        "    enabled: {}\n",
        windows_lhm_http_state.enabled
    ));
    report.push_str(&format!(
        "    supported: {}\n",
        windows_lhm_http_state.supported
    ));
    report.push_str(&format!("    status: {}\n", windows_lhm_http_state.status));
    report.push_str(&format!("    url: {}\n", windows_lhm_http_state.url));
    report.push_str(&format!(
        "    sensors: {}\n",
        windows_lhm_http_state.sensor_count
    ));
    if let Some(message) = windows_lhm_http_state.message {
        report.push_str(&format!("    message: {}\n", message));
    }

    let windows_lhm_wmi_state =
        WindowsLhmWmiCollector::discover_summary(&config.collectors.windows_lhm_wmi);
    report.push_str("- windows_lhm_wmi:\n");
    report.push_str(&format!("    enabled: {}\n", windows_lhm_wmi_state.enabled));
    report.push_str(&format!(
        "    supported: {}\n",
        windows_lhm_wmi_state.supported
    ));
    report.push_str(&format!("    status: {}\n", windows_lhm_wmi_state.status));
    report.push_str(&format!(
        "    namespace: {}\n",
        windows_lhm_wmi_state.namespace
    ));
    report.push_str(&format!(
        "    sensors: {}\n",
        windows_lhm_wmi_state.sensor_count
    ));
    if let Some(message) = windows_lhm_wmi_state.message {
        report.push_str(&format!("    message: {}\n", message));
    }

    let windows_inventory_state =
        WindowsInventoryCollector::discover_summary(&config.collectors.windows_inventory);
    report.push_str(&format!(
        "- windows_inventory: {}, enabled={}\n",
        windows_inventory_state, config.collectors.windows_inventory.enabled
    ));

    report.push_str(&format!(
        "- registration: enabled={}, registry_addr={}, advertised_addr={}\n",
        config.registration.enabled,
        config.registration.registry_addr,
        config.registration.advertised_addr
    ));

    report
}
