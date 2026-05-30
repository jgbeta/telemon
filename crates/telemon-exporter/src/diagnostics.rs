use std::time::Duration;

use crate::registration::DeviceInfoCollector;
use crate::scheduler::{collect_snapshot_once, ScheduledCollector};
use telemon_collectors::gpu::nvidia::collector::NvidiaNvmlCollector;
use telemon_collectors::linux::amdgpu::LinuxAmdgpuCollector;
use telemon_collectors::linux::drm::LinuxDrmCollector;
use telemon_collectors::linux::gamescope::inspect_hardware as inspect_steam_deck_game_state;
use telemon_collectors::linux::power_supply::LinuxPowerSupplyCollector;
use telemon_collectors::macos::exact_temperature_experimental::MacosExactTemperatureExperimentalCollector;
use telemon_collectors::macos::macmon::MacosMacmonCollector;
use telemon_collectors::macos::thermal_state::MacosThermalStateCollector;
use telemon_collectors::system::collector::SystemCollector;
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

    if config.collectors.system.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(SystemCollector::new(config.collectors.system.clone())),
            Duration::from_secs(config.collection.system_interval_seconds),
        ));
    }

    if config.collectors.macos_thermal_state.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(MacosThermalStateCollector::new(
                config.collectors.macos_thermal_state.clone(),
            )),
            Duration::from_secs(config.collection.macos_thermal_state_interval_seconds),
        ));
    }

    if config.collectors.macos_macmon.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(MacosMacmonCollector::new(
                config.collectors.macos_macmon.clone(),
            )),
            Duration::from_secs(config.collectors.macos_macmon.sample_interval_seconds),
        ));
    }

    if config
        .collectors
        .macos_exact_temperature_experimental
        .enabled
    {
        collectors.push(ScheduledCollector::new(
            Box::new(MacosExactTemperatureExperimentalCollector::new(
                config
                    .collectors
                    .macos_exact_temperature_experimental
                    .clone(),
            )),
            Duration::from_secs(config.collection.macos_thermal_state_interval_seconds),
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

    if config.collectors.linux_power_supply.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(LinuxPowerSupplyCollector::new(
                config.collectors.linux_power_supply.clone(),
            )),
            Duration::from_secs(config.collection.system_interval_seconds),
        ));
    }

    if config.collectors.linux_amdgpu.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(LinuxAmdgpuCollector::new(
                config.collectors.linux_amdgpu.clone(),
            )),
            Duration::from_secs(config.collection.gpu_interval_seconds),
        ));
    }

    if config.collectors.linux_drm.enabled {
        collectors.push(ScheduledCollector::new(
            Box::new(LinuxDrmCollector::new(config.collectors.linux_drm.clone())),
            Duration::from_secs(config.collection.gpu_interval_seconds),
        ));
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

    if config.collectors.system.enabled {
        report.push_str("- system\n");
    }
    if config.collectors.macos_thermal_state.enabled {
        report.push_str("- macos_thermal_state\n");
    }
    if config.collectors.macos_macmon.enabled {
        report.push_str("- macos_macmon\n");
    }
    if config
        .collectors
        .macos_exact_temperature_experimental
        .enabled
    {
        report.push_str("- macos_exact_temperature_experimental\n");
    }
    if config.collectors.linux_hwmon.enabled {
        report.push_str("- linux_hwmon\n");
    }
    if config.collectors.linux_power_supply.enabled {
        report.push_str("- linux_power_supply\n");
    }
    if config.collectors.linux_amdgpu.enabled {
        report.push_str("- linux_amdgpu\n");
    }
    if config.collectors.linux_drm.enabled {
        report.push_str("- linux_drm\n");
    }
    if config.collectors.steam_deck_game_state.enabled {
        report.push_str("- steam_deck_game_state_sampling_override\n");
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
        && !config.collectors.system.enabled
        && !config.collectors.macos_thermal_state.enabled
        && !config.collectors.macos_macmon.enabled
        && !config
            .collectors
            .macos_exact_temperature_experimental
            .enabled
        && !config.collectors.linux_power_supply.enabled
        && !config.collectors.linux_amdgpu.enabled
        && !config.collectors.linux_drm.enabled
        && !config.collectors.steam_deck_game_state.enabled
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
    let system_state = SystemCollector::discover_summary(&config.collectors.system);
    report.push_str(&format!(
        "- system: {}, enabled={}, cpu_enabled={}, memory_enabled={}, uptime_enabled={}\n",
        system_state,
        config.collectors.system.enabled,
        config.collectors.system.cpu_enabled,
        config.collectors.system.memory_enabled,
        config.collectors.system.uptime_enabled
    ));

    let macos_thermal_state =
        MacosThermalStateCollector::discover_summary(&config.collectors.macos_thermal_state);
    report.push_str(&format!(
        "- macos_thermal_state: {}, enabled={}\n",
        macos_thermal_state, config.collectors.macos_thermal_state.enabled
    ));

    let macos_macmon = MacosMacmonCollector::discover_summary(&config.collectors.macos_macmon);
    report.push_str(&format!(
        "- macos_macmon: {}, enabled={}, sample_interval_seconds={}, sample_window_milliseconds={}, stale_after_seconds={}\n",
        macos_macmon,
        config.collectors.macos_macmon.enabled,
        config.collectors.macos_macmon.sample_interval_seconds,
        config.collectors.macos_macmon.sample_window_milliseconds,
        config.collectors.macos_macmon.stale_after_seconds
    ));

    let macos_exact_temperature = MacosExactTemperatureExperimentalCollector::discover_summary(
        &config.collectors.macos_exact_temperature_experimental,
    );
    report.push_str(&format!(
        "- macos_exact_temperature_experimental: {}, enabled={}\n",
        macos_exact_temperature,
        config
            .collectors
            .macos_exact_temperature_experimental
            .enabled
    ));

    let linux_state = LinuxHwmonCollector::discover_summary(&config.collectors.linux_hwmon);
    report.push_str(&format!(
        "- linux_hwmon: {}, enabled={}, root={}, include_unknown_sensors={}\n",
        linux_state,
        config.collectors.linux_hwmon.enabled,
        config.collectors.linux_hwmon.root.display(),
        config.collectors.linux_hwmon.include_unknown_sensors
    ));

    let power_supply_state =
        LinuxPowerSupplyCollector::discover_summary(&config.collectors.linux_power_supply);
    report.push_str(&format!(
        "- linux_power_supply: {}, enabled={}, root={}, derive_power_when_missing={}\n",
        power_supply_state,
        config.collectors.linux_power_supply.enabled,
        config.collectors.linux_power_supply.root.display(),
        config
            .collectors
            .linux_power_supply
            .derive_power_when_missing
    ));

    let amdgpu_state = LinuxAmdgpuCollector::discover_summary(&config.collectors.linux_amdgpu);
    report.push_str(&format!(
        "- linux_amdgpu: {}, enabled={}, root={}, diagnostic_gpu_metrics={}\n",
        amdgpu_state,
        config.collectors.linux_amdgpu.enabled,
        config.collectors.linux_amdgpu.root.display(),
        config
            .collectors
            .linux_amdgpu
            .include_diagnostic_only_gpu_metrics
    ));

    let game_state = inspect_steam_deck_game_state(&config.collectors.steam_deck_game_state);
    report.push_str(&format!(
        "- steam_deck_game_state: enabled={}, supported={}, state={}, display={}, poll_interval_seconds={}, stop_debounce_seconds={}, auto_discover_steam_display={}, desktop_fallback_enabled={}, process_fallback_enabled={}\n",
        game_state.enabled,
        game_state.supported,
        game_state.state.as_str(),
        game_state.display,
        config.collectors.steam_deck_game_state.poll_interval_seconds,
        config.collectors.steam_deck_game_state.stop_debounce_seconds,
        config
            .collectors
            .steam_deck_game_state
            .auto_discover_steam_display,
        config
            .collectors
            .steam_deck_game_state
            .desktop_fallback_enabled,
        config
            .collectors
            .steam_deck_game_state
            .process_fallback_enabled
    ));

    let linux_drm_state = LinuxDrmCollector::discover_summary(&config.collectors.linux_drm);
    report.push_str(&format!(
        "- linux_drm: {}, enabled={}, drm_root={}, proc_root={}, fdinfo={}\n",
        linux_drm_state,
        config.collectors.linux_drm.enabled,
        config.collectors.linux_drm.drm_root.display(),
        config.collectors.linux_drm.proc_root.display(),
        config.collectors.linux_drm.include_fdinfo
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
