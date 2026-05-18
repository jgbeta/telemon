use std::time::Duration;

use crate::registration::DeviceInfoCollector;
use crate::scheduler::{collect_snapshot_once, ScheduledCollector};
use telemon_collectors::fake::FakeCollector;
use telemon_collectors::gpu::nvidia::collector::NvidiaNvmlCollector;
use telemon_collectors::temperature::linux_hwmon::LinuxHwmonCollector;
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

    if config.collectors.fake.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(FakeCollector::new()),
            Duration::from_secs(config.collection.fake_interval_seconds),
        ));
    }

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

    if config.collectors.fake.enabled {
        report.push_str("- fake\n");
    }
    if config.collectors.linux_hwmon.enabled {
        report.push_str("- linux_hwmon\n");
    }
    if config.collectors.nvidia_nvml.enabled {
        report.push_str("- nvidia_nvml\n");
    }
    if config.registration.enabled {
        report.push_str("- identity\n");
    }
    if !config.collectors.fake.enabled
        && !config.collectors.linux_hwmon.enabled
        && !config.collectors.nvidia_nvml.enabled
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
    report.push_str(&format!(
        "- fake: available, enabled={}\n",
        config.collectors.fake.enabled
    ));

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
    report.push_str(&format!(
        "- registration: enabled={}, registry_addr={}, advertised_addr={}\n",
        config.registration.enabled,
        config.registration.registry_addr,
        config.registration.advertised_addr
    ));

    report
}
