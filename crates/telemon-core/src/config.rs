use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub identity: IdentityConfig,
    pub registration: RegistrationConfig,
    pub collection: CollectionConfig,
    pub adaptive_sampling: AdaptiveSamplingConfig,
    pub collectors: CollectorsConfig,
    pub diagnostics: DiagnosticsConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryAppConfig {
    pub registry: RegistryConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub listen: String,
    pub metrics_path: String,
    pub static_metrics_path: String,
    pub fps_metrics_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    pub user_name: String,
    pub device_name: String,
    pub machine_uuid: String,
    pub machine_uuid_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistrationConfig {
    pub enabled: bool,
    pub registry_addr: String,
    pub enrollment_token: String,
    pub device_id_file: PathBuf,
    pub heartbeat_interval_seconds: u64,
    pub scrape_port: u16,
    pub advertised_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryConfig {
    pub listen: String,
    pub storage_path: PathBuf,
    pub enrollment_token: String,
    pub device_stale_after_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionConfig {
    pub scrape_cache_stale_after_seconds: u64,
    pub system_interval_seconds: u64,
    pub macos_thermal_state_interval_seconds: u64,
    pub temperature_interval_seconds: u64,
    pub sensor_rescan_interval_seconds: u64,
    pub gpu_interval_seconds: u64,
    pub windows_baseline_interval_seconds: u64,
    pub windows_inventory_interval_seconds: u64,
    pub static_info_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveSamplingConfig {
    pub enabled: bool,
    pub levels: AdaptiveSamplingLevelsConfig,
    pub temperature: AdaptiveTemperatureConfig,
    pub cooldown_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveSamplingLevelsConfig {
    pub normal_seconds: u64,
    pub warm_seconds: u64,
    pub hot_seconds: u64,
    pub critical_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveTemperatureConfig {
    pub enabled: bool,
    pub warm_celsius: f64,
    pub hot_celsius: f64,
    pub critical_celsius: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticsConfig {
    pub enabled: bool,
    pub scrape_gap_threshold_seconds: u64,
    pub scheduler_lag_threshold_seconds: u64,
    pub log_scrape_gaps: bool,
    pub log_scheduler_lag: bool,
    pub log_scrape_interval_changes: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectorsConfig {
    pub system: SystemConfig,
    pub macos_thermal_state: MacosThermalStateConfig,
    pub macos_macmon: MacosMacmonConfig,
    pub macos_exact_temperature_experimental: MacosExactTemperatureExperimentalConfig,
    pub linux_hwmon: LinuxHwmonConfig,
    pub linux_power_supply: LinuxPowerSupplyConfig,
    pub linux_amdgpu: LinuxAmdgpuConfig,
    pub linux_drm: LinuxDrmConfig,
    pub steam_deck_game_state: SteamDeckGameStateConfig,
    pub steam_deck_fps: SteamDeckFpsConfig,
    pub nvidia_nvml: NvidiaNvmlConfig,
    pub windows_baseline: WindowsBaselineConfig,
    pub windows_inventory: WindowsInventoryConfig,
    pub windows_lhm_http: WindowsLhmHttpConfig,
    pub windows_lhm_wmi: WindowsLhmWmiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    pub enabled: bool,
    pub cpu_enabled: bool,
    pub memory_enabled: bool,
    pub uptime_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacosThermalStateConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacosMacmonConfig {
    pub enabled: bool,
    pub sample_interval_seconds: u64,
    pub sample_window_milliseconds: u64,
    pub stale_after_seconds: u64,
    pub reinitialize_after_consecutive_errors: u64,
    pub min_temperature_celsius: f64,
    pub max_temperature_celsius: f64,
    pub max_power_watts: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MacosExactTemperatureExperimentalConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxHwmonConfig {
    pub enabled: bool,
    pub root: PathBuf,
    pub include_unknown_sensors: bool,
    pub nvme_enrichment_enabled: bool,
    pub expose_storage_model: bool,
    pub sensor_allowlist: Vec<String>,
    pub sensor_denylist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxPowerSupplyConfig {
    pub enabled: bool,
    pub root: PathBuf,
    pub derive_power_when_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxAmdgpuConfig {
    pub enabled: bool,
    pub root: PathBuf,
    pub include_diagnostic_only_gpu_metrics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxDrmConfig {
    pub enabled: bool,
    pub drm_root: PathBuf,
    pub proc_root: PathBuf,
    pub target_pid: Option<u32>,
    pub include_hwmon: bool,
    pub include_fdinfo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamDeckGameStateConfig {
    pub enabled: bool,
    pub poll_interval_seconds: u64,
    pub stop_debounce_seconds: u64,
    pub xprop_path: String,
    pub display: String,
    pub auto_discover_steam_display: bool,
    pub desktop_fallback_enabled: bool,
    pub process_fallback_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamDeckFpsConfig {
    pub enabled: bool,
    pub windows_seconds: Vec<u64>,
    pub include_appid_label: bool,
    pub include_game_name_label: bool,
    pub max_frame_time_milliseconds: u64,
    pub poll_interval_milliseconds: u64,
    pub max_messages_per_poll: u64,
    pub source_preference: Vec<String>,
    pub gamescope_wayland: GamescopeWaylandConfig,
    pub mangohud_log: MangoHudLogConfig,
    pub gamescope_mangoapp: GamescopeMangoappConfig,
    pub steam_library_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GamescopeWaylandConfig {
    pub enabled: bool,
    pub display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MangoHudLogConfig {
    pub enabled: bool,
    pub paths: Vec<PathBuf>,
    pub auto_discover: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GamescopeMangoappConfig {
    pub enabled: bool,
    pub ftok_path: PathBuf,
    pub project_id: i32,
    pub legacy_failed_ftok_fallback_enabled: bool,
    pub allow_destructive_read: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NvidiaNvmlConfig {
    pub enabled: bool,
    pub library_paths: Vec<PathBuf>,
    pub expose_gpu_name: bool,
    pub expose_gpu_uuid: bool,
    pub fan_speed_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsBaselineConfig {
    pub enabled: bool,
    pub include_removable_drives: bool,
    pub include_remote_drives: bool,
    pub network_interface_allowlist: Vec<String>,
    pub network_interface_denylist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsInventoryConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsLhmHttpConfig {
    pub enabled: bool,
    pub url: String,
    pub timeout_ms: u64,
    pub include_unknown_sensors: bool,
    pub sensor_allowlist: Vec<String>,
    pub sensor_denylist: Vec<String>,
    pub require_provider: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowsLhmWmiConfig {
    pub enabled: bool,
    pub namespace: String,
    pub include_unknown_sensors: bool,
    pub sensor_allowlist: Vec<String>,
    pub sensor_denylist: Vec<String>,
    pub require_provider: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

impl AppConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let text = strip_leading_bom(&text);
        let config: Self = serde_yaml::from_str(text)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.server.listen.parse::<SocketAddr>().with_context(|| {
            format!(
                "server.listen must be a socket address: {}",
                self.server.listen
            )
        })?;

        if !self.server.metrics_path.starts_with('/') {
            bail!("server.metrics_path must start with /");
        }
        if !self.server.static_metrics_path.starts_with('/') {
            bail!("server.static_metrics_path must start with /");
        }
        if !self.server.fps_metrics_path.starts_with('/') {
            bail!("server.fps_metrics_path must start with /");
        }
        if self.server.static_metrics_path == self.server.metrics_path
            || self.server.fps_metrics_path == self.server.metrics_path
            || self.server.fps_metrics_path == self.server.static_metrics_path
        {
            bail!("server metrics paths must be distinct");
        }

        if self.registration.enabled {
            if self.registration.registry_addr.trim().is_empty() {
                bail!("registration.registry_addr must not be empty when registration is enabled");
            }
            if self.registration.enrollment_token.trim().is_empty() {
                bail!(
                    "registration.enrollment_token must not be empty when registration is enabled"
                );
            }
            if self.identity.user_name.trim().is_empty() {
                bail!("identity.user_name must not be empty when registration is enabled");
            }
        }

        validate_positive(
            self.collection.scrape_cache_stale_after_seconds,
            "collection.scrape_cache_stale_after_seconds",
        )?;
        validate_positive(
            self.collection.system_interval_seconds,
            "collection.system_interval_seconds",
        )?;
        validate_positive(
            self.collection.macos_thermal_state_interval_seconds,
            "collection.macos_thermal_state_interval_seconds",
        )?;
        validate_positive(
            self.collection.temperature_interval_seconds,
            "collection.temperature_interval_seconds",
        )?;
        validate_positive(
            self.collection.sensor_rescan_interval_seconds,
            "collection.sensor_rescan_interval_seconds",
        )?;
        validate_positive(
            self.collection.gpu_interval_seconds,
            "collection.gpu_interval_seconds",
        )?;
        validate_positive(
            self.collection.windows_baseline_interval_seconds,
            "collection.windows_baseline_interval_seconds",
        )?;
        validate_positive(
            self.collection.windows_inventory_interval_seconds,
            "collection.windows_inventory_interval_seconds",
        )?;
        validate_positive(
            self.collection.static_info_interval_seconds,
            "collection.static_info_interval_seconds",
        )?;
        validate_positive(
            self.registration.heartbeat_interval_seconds,
            "registration.heartbeat_interval_seconds",
        )?;
        validate_positive(
            self.adaptive_sampling.levels.normal_seconds,
            "adaptive_sampling.levels.normal_seconds",
        )?;
        validate_positive(
            self.adaptive_sampling.levels.warm_seconds,
            "adaptive_sampling.levels.warm_seconds",
        )?;
        validate_positive(
            self.adaptive_sampling.levels.hot_seconds,
            "adaptive_sampling.levels.hot_seconds",
        )?;
        validate_positive(
            self.adaptive_sampling.levels.critical_seconds,
            "adaptive_sampling.levels.critical_seconds",
        )?;
        validate_positive(
            self.adaptive_sampling.cooldown_seconds,
            "adaptive_sampling.cooldown_seconds",
        )?;
        validate_positive(
            self.diagnostics.scrape_gap_threshold_seconds,
            "diagnostics.scrape_gap_threshold_seconds",
        )?;
        validate_positive(
            self.diagnostics.scheduler_lag_threshold_seconds,
            "diagnostics.scheduler_lag_threshold_seconds",
        )?;
        if !(self.adaptive_sampling.temperature.warm_celsius
            < self.adaptive_sampling.temperature.hot_celsius
            && self.adaptive_sampling.temperature.hot_celsius
                < self.adaptive_sampling.temperature.critical_celsius)
        {
            bail!("adaptive_sampling.temperature thresholds must increase warm < hot < critical");
        }

        if self.registration.scrape_port == 0 {
            bail!("registration.scrape_port must be greater than 0");
        }

        if self.collectors.linux_hwmon.root.as_os_str().is_empty() {
            bail!("collectors.linux_hwmon.root must not be empty");
        }
        if self
            .collectors
            .linux_power_supply
            .root
            .as_os_str()
            .is_empty()
        {
            bail!("collectors.linux_power_supply.root must not be empty");
        }
        if self.collectors.linux_amdgpu.root.as_os_str().is_empty() {
            bail!("collectors.linux_amdgpu.root must not be empty");
        }
        validate_positive(
            self.collectors.steam_deck_game_state.poll_interval_seconds,
            "collectors.steam_deck_game_state.poll_interval_seconds",
        )?;
        validate_positive(
            self.collectors.steam_deck_game_state.stop_debounce_seconds,
            "collectors.steam_deck_game_state.stop_debounce_seconds",
        )?;
        if self.collectors.steam_deck_game_state.enabled
            && self
                .collectors
                .steam_deck_game_state
                .xprop_path
                .trim()
                .is_empty()
        {
            bail!("collectors.steam_deck_game_state.xprop_path must not be empty when enabled");
        }
        validate_positive(
            self.collectors.steam_deck_fps.poll_interval_milliseconds,
            "collectors.steam_deck_fps.poll_interval_milliseconds",
        )?;
        validate_positive(
            self.collectors.steam_deck_fps.max_messages_per_poll,
            "collectors.steam_deck_fps.max_messages_per_poll",
        )?;
        validate_positive(
            self.collectors.steam_deck_fps.max_frame_time_milliseconds,
            "collectors.steam_deck_fps.max_frame_time_milliseconds",
        )?;
        if self.collectors.steam_deck_fps.enabled {
            if self.collectors.steam_deck_fps.windows_seconds.is_empty() {
                bail!("collectors.steam_deck_fps.windows_seconds must not be empty when enabled");
            }
            if self.collectors.steam_deck_fps.source_preference.is_empty() {
                bail!("collectors.steam_deck_fps.source_preference must not be empty when enabled");
            }
            for (index, value) in self
                .collectors
                .steam_deck_fps
                .windows_seconds
                .iter()
                .enumerate()
            {
                validate_positive(
                    *value,
                    &format!("collectors.steam_deck_fps.windows_seconds[{index}]"),
                )?;
            }
        }
        for (index, source) in self
            .collectors
            .steam_deck_fps
            .source_preference
            .iter()
            .enumerate()
        {
            match source.as_str() {
                "gamescope_wayland" | "mangohud_log" | "gamescope_mangoapp" => {}
                _ => bail!(
                    "collectors.steam_deck_fps.source_preference[{index}] has unsupported source {source:?}"
                ),
            }
        }
        if self.collectors.steam_deck_fps.mangohud_log.enabled {
            for (index, path) in self
                .collectors
                .steam_deck_fps
                .mangohud_log
                .paths
                .iter()
                .enumerate()
            {
                if path.as_os_str().is_empty() {
                    bail!(
                        "collectors.steam_deck_fps.mangohud_log.paths[{index}] must not be empty when enabled"
                    );
                }
            }
        }
        if self.collectors.steam_deck_fps.gamescope_mangoapp.enabled
            && self
                .collectors
                .steam_deck_fps
                .gamescope_mangoapp
                .ftok_path
                .as_os_str()
                .is_empty()
        {
            bail!("collectors.steam_deck_fps.gamescope_mangoapp.ftok_path must not be empty when enabled");
        }
        if self.collectors.windows_lhm_http.enabled {
            if self.collectors.windows_lhm_http.url.trim().is_empty() {
                bail!("collectors.windows_lhm_http.url must not be empty when enabled");
            }
            if !self.collectors.windows_lhm_http.url.starts_with("http://") {
                bail!("collectors.windows_lhm_http.url must start with http:// when enabled");
            }
            validate_positive(
                self.collectors.windows_lhm_http.timeout_ms,
                "collectors.windows_lhm_http.timeout_ms",
            )?;
        }
        if self.collectors.windows_lhm_wmi.enabled
            && self.collectors.windows_lhm_wmi.namespace.trim().is_empty()
        {
            bail!("collectors.windows_lhm_wmi.namespace must not be empty when enabled");
        }
        validate_positive(
            self.collectors.macos_macmon.sample_interval_seconds,
            "collectors.macos_macmon.sample_interval_seconds",
        )?;
        validate_positive(
            self.collectors.macos_macmon.sample_window_milliseconds,
            "collectors.macos_macmon.sample_window_milliseconds",
        )?;
        if self.collectors.macos_macmon.sample_window_milliseconds > u64::from(u32::MAX) {
            bail!("collectors.macos_macmon.sample_window_milliseconds must fit in u32");
        }
        validate_positive(
            self.collectors.macos_macmon.stale_after_seconds,
            "collectors.macos_macmon.stale_after_seconds",
        )?;
        validate_positive(
            self.collectors
                .macos_macmon
                .reinitialize_after_consecutive_errors,
            "collectors.macos_macmon.reinitialize_after_consecutive_errors",
        )?;
        if !self
            .collectors
            .macos_macmon
            .min_temperature_celsius
            .is_finite()
            || !self
                .collectors
                .macos_macmon
                .max_temperature_celsius
                .is_finite()
            || self.collectors.macos_macmon.min_temperature_celsius
                >= self.collectors.macos_macmon.max_temperature_celsius
        {
            bail!(
                "collectors.macos_macmon temperature bounds must be finite and increase min < max"
            );
        }
        if !self.collectors.macos_macmon.max_power_watts.is_finite()
            || self.collectors.macos_macmon.max_power_watts <= 0.0
        {
            bail!("collectors.macos_macmon.max_power_watts must be finite and greater than 0");
        }

        match self.logging.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => {}
            other => bail!("logging.level must be trace, debug, info, warn, or error: {other}"),
        }

        Ok(())
    }

    pub fn listen_addr(&self) -> Result<SocketAddr> {
        self.server
            .listen
            .parse()
            .with_context(|| format!("invalid listen address {}", self.server.listen))
    }
}

impl RegistryAppConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read registry config {}", path.display()))?;
        let text = strip_leading_bom(&text);
        let config: Self = serde_yaml::from_str(text)
            .with_context(|| format!("failed to parse registry config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.registry
            .listen
            .parse::<SocketAddr>()
            .with_context(|| {
                format!(
                    "registry.listen must be a socket address: {}",
                    self.registry.listen
                )
            })?;

        if self.registry.storage_path.as_os_str().is_empty() {
            bail!("registry.storage_path must not be empty");
        }
        if self.registry.enrollment_token.trim().is_empty() {
            bail!("registry.enrollment_token must not be empty");
        }
        validate_positive(
            self.registry.device_stale_after_seconds,
            "registry.device_stale_after_seconds",
        )?;

        match self.logging.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => {}
            other => bail!("logging.level must be trace, debug, info, warn, or error: {other}"),
        }

        Ok(())
    }

    pub fn listen_addr(&self) -> Result<SocketAddr> {
        self.registry
            .listen
            .parse()
            .with_context(|| format!("invalid registry listen address {}", self.registry.listen))
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:9185".to_string(),
            metrics_path: "/metrics".to_string(),
            static_metrics_path: "/metrics/static".to_string(),
            fps_metrics_path: "/fps".to_string(),
        }
    }
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            user_name: String::new(),
            device_name: default_device_name(),
            machine_uuid: String::new(),
            machine_uuid_file: PathBuf::new(),
        }
    }
}

impl Default for RegistrationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            registry_addr: String::new(),
            enrollment_token: String::new(),
            device_id_file: PathBuf::new(),
            heartbeat_interval_seconds: 30,
            scrape_port: 9185,
            advertised_addr: String::new(),
        }
    }
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:9186".to_string(),
            storage_path: PathBuf::from("/data/devices.json"),
            enrollment_token: "change-me".to_string(),
            device_stale_after_seconds: 120,
        }
    }
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            scrape_cache_stale_after_seconds: 60,
            system_interval_seconds: 15,
            macos_thermal_state_interval_seconds: 15,
            temperature_interval_seconds: 15,
            sensor_rescan_interval_seconds: 300,
            gpu_interval_seconds: 15,
            windows_baseline_interval_seconds: 15,
            windows_inventory_interval_seconds: 300,
            static_info_interval_seconds: 300,
        }
    }
}

impl Default for AdaptiveSamplingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            levels: AdaptiveSamplingLevelsConfig::default(),
            temperature: AdaptiveTemperatureConfig::default(),
            cooldown_seconds: 60,
        }
    }
}

impl Default for AdaptiveSamplingLevelsConfig {
    fn default() -> Self {
        Self {
            normal_seconds: 15,
            warm_seconds: 10,
            hot_seconds: 5,
            critical_seconds: 1,
        }
    }
}

impl Default for AdaptiveTemperatureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            warm_celsius: 60.0,
            hot_celsius: 75.0,
            critical_celsius: 85.0,
        }
    }
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scrape_gap_threshold_seconds: 30,
            scheduler_lag_threshold_seconds: 5,
            log_scrape_gaps: true,
            log_scheduler_lag: true,
            log_scrape_interval_changes: true,
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_enabled: true,
            memory_enabled: true,
            uptime_enabled: true,
        }
    }
}

impl Default for MacosThermalStateConfig {
    fn default() -> Self {
        Self {
            enabled: default_macos_thermal_state_enabled(),
        }
    }
}

fn default_macos_thermal_state_enabled() -> bool {
    cfg!(target_os = "macos")
}

impl Default for MacosMacmonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_interval_seconds: 1,
            sample_window_milliseconds: 1000,
            stale_after_seconds: 5,
            reinitialize_after_consecutive_errors: 5,
            min_temperature_celsius: 1.0,
            max_temperature_celsius: 130.0,
            max_power_watts: 300.0,
        }
    }
}

impl Default for LinuxHwmonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root: PathBuf::from("/sys/class/hwmon"),
            include_unknown_sensors: false,
            nvme_enrichment_enabled: true,
            expose_storage_model: true,
            sensor_allowlist: Vec::new(),
            sensor_denylist: Vec::new(),
        }
    }
}

impl Default for LinuxPowerSupplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            root: PathBuf::from("/sys/class/power_supply"),
            derive_power_when_missing: true,
        }
    }
}

impl Default for LinuxAmdgpuConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            root: PathBuf::from("/sys/class/drm"),
            include_diagnostic_only_gpu_metrics: true,
        }
    }
}

impl Default for LinuxDrmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drm_root: PathBuf::from("/sys/class/drm"),
            proc_root: PathBuf::from("/proc"),
            target_pid: None,
            include_hwmon: true,
            include_fdinfo: false,
        }
    }
}

impl Default for SteamDeckGameStateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_seconds: 1,
            stop_debounce_seconds: 5,
            xprop_path: "xprop".to_string(),
            display: ":0".to_string(),
            auto_discover_steam_display: true,
            desktop_fallback_enabled: true,
            process_fallback_enabled: true,
        }
    }
}

impl Default for SteamDeckFpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            windows_seconds: vec![1, 5, 60],
            include_appid_label: true,
            include_game_name_label: true,
            max_frame_time_milliseconds: 1_000,
            poll_interval_milliseconds: 100,
            max_messages_per_poll: 512,
            source_preference: vec![
                "gamescope_wayland".to_string(),
                "mangohud_log".to_string(),
                "gamescope_mangoapp".to_string(),
            ],
            gamescope_wayland: GamescopeWaylandConfig::default(),
            mangohud_log: MangoHudLogConfig::default(),
            gamescope_mangoapp: GamescopeMangoappConfig::default(),
            steam_library_roots: Vec::new(),
        }
    }
}

impl Default for MangoHudLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            paths: Vec::new(),
            auto_discover: true,
        }
    }
}

impl Default for GamescopeMangoappConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ftok_path: PathBuf::from("mangoapp"),
            project_id: 65,
            legacy_failed_ftok_fallback_enabled: false,
            allow_destructive_read: false,
        }
    }
}

impl Default for NvidiaNvmlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            library_paths: Vec::new(),
            expose_gpu_name: true,
            expose_gpu_uuid: false,
            fan_speed_enabled: true,
        }
    }
}

impl Default for WindowsBaselineConfig {
    fn default() -> Self {
        Self {
            enabled: default_windows_collector_enabled(),
            include_removable_drives: false,
            include_remote_drives: false,
            network_interface_allowlist: Vec::new(),
            network_interface_denylist: vec![
                "loopback".to_string(),
                "isatap".to_string(),
                "teredo".to_string(),
            ],
        }
    }
}

impl Default for WindowsInventoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_windows_collector_enabled(),
        }
    }
}

impl Default for WindowsLhmHttpConfig {
    fn default() -> Self {
        Self {
            enabled: default_windows_collector_enabled(),
            url: "http://127.0.0.1:8085/data.json".to_string(),
            timeout_ms: 1500,
            include_unknown_sensors: false,
            sensor_allowlist: Vec::new(),
            sensor_denylist: Vec::new(),
            require_provider: false,
        }
    }
}

impl Default for WindowsLhmWmiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            namespace: r"root\LibreHardwareMonitor".to_string(),
            include_unknown_sensors: false,
            sensor_allowlist: Vec::new(),
            sensor_denylist: Vec::new(),
            require_provider: false,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

fn strip_leading_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn default_windows_collector_enabled() -> bool {
    cfg!(target_os = "windows")
}

fn validate_positive(value: u64, name: &str) -> Result<()> {
    if value == 0 {
        bail!("{name} must be greater than 0");
    }
    Ok(())
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown-device".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("telemon-{name}-{}-{nanos}.yml", std::process::id()))
    }

    #[test]
    fn default_config_is_valid() {
        AppConfig::default().validate().unwrap();
    }

    #[test]
    fn app_config_loads_utf8_bom_prefixed_yaml() {
        let path = temp_config_path("app-bom");
        std::fs::write(&path, "\u{feff}server:\n  listen: \"127.0.0.1:9185\"\n").unwrap();

        let config = AppConfig::load_from_path(&path).unwrap();

        std::fs::remove_file(&path).unwrap();
        assert_eq!(config.server.listen, "127.0.0.1:9185");
    }

    #[test]
    fn registry_config_loads_utf8_bom_prefixed_yaml() {
        let path = temp_config_path("registry-bom");
        std::fs::write(
            &path,
            "\u{feff}registry:\n  listen: \"127.0.0.1:9186\"\n  enrollment_token: \"secret\"\n",
        )
        .unwrap();

        let config = RegistryAppConfig::load_from_path(&path).unwrap();

        std::fs::remove_file(&path).unwrap();
        assert_eq!(config.registry.listen, "127.0.0.1:9186");
    }

    #[test]
    fn rejects_relative_metrics_path() {
        let mut config = AppConfig::default();
        config.server.metrics_path = "metrics".to_string();

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_intervals() {
        let mut config = AppConfig::default();
        config.collection.temperature_interval_seconds = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn defaults_linux_hwmon_config() {
        let config = AppConfig::default();

        assert!(config.collectors.linux_hwmon.enabled);
        assert!(config.collectors.linux_hwmon.nvme_enrichment_enabled);
        assert!(config.collectors.linux_hwmon.expose_storage_model);
    }

    #[test]
    fn defaults_linux_handheld_collector_config() {
        let config = AppConfig::default();

        assert!(!config.collectors.linux_power_supply.enabled);
        assert_eq!(
            config.collectors.linux_power_supply.root,
            PathBuf::from("/sys/class/power_supply")
        );
        assert!(
            config
                .collectors
                .linux_power_supply
                .derive_power_when_missing
        );
        assert!(!config.collectors.linux_amdgpu.enabled);
        assert_eq!(
            config.collectors.linux_amdgpu.root,
            PathBuf::from("/sys/class/drm")
        );
        assert!(
            config
                .collectors
                .linux_amdgpu
                .include_diagnostic_only_gpu_metrics
        );
        assert!(!config.collectors.linux_drm.enabled);
        assert_eq!(
            config.collectors.linux_drm.drm_root,
            PathBuf::from("/sys/class/drm")
        );
        assert_eq!(
            config.collectors.linux_drm.proc_root,
            PathBuf::from("/proc")
        );
        assert!(config.collectors.linux_drm.include_hwmon);
        assert!(!config.collectors.linux_drm.include_fdinfo);
        assert!(!config.collectors.steam_deck_game_state.enabled);
        assert_eq!(
            config
                .collectors
                .steam_deck_game_state
                .poll_interval_seconds,
            1
        );
        assert_eq!(
            config
                .collectors
                .steam_deck_game_state
                .stop_debounce_seconds,
            5
        );
        assert!(
            config
                .collectors
                .steam_deck_game_state
                .auto_discover_steam_display
        );
        assert!(
            config
                .collectors
                .steam_deck_game_state
                .desktop_fallback_enabled
        );
        assert!(
            config
                .collectors
                .steam_deck_game_state
                .process_fallback_enabled
        );
        assert!(!config.collectors.steam_deck_fps.enabled);
        assert_eq!(
            config.collectors.steam_deck_fps.windows_seconds,
            vec![1, 5, 60]
        );
        assert!(config.collectors.steam_deck_fps.include_appid_label);
        assert!(config.collectors.steam_deck_fps.include_game_name_label);
        assert_eq!(
            config.collectors.steam_deck_fps.max_frame_time_milliseconds,
            1_000
        );
        assert_eq!(
            config.collectors.steam_deck_fps.poll_interval_milliseconds,
            100
        );
        assert_eq!(config.collectors.steam_deck_fps.max_messages_per_poll, 512);
        assert_eq!(
            config.collectors.steam_deck_fps.source_preference,
            vec!["gamescope_wayland", "mangohud_log", "gamescope_mangoapp"]
        );
        assert!(!config.collectors.steam_deck_fps.gamescope_wayland.enabled);
        assert!(!config.collectors.steam_deck_fps.mangohud_log.enabled);
        assert!(config.collectors.steam_deck_fps.mangohud_log.auto_discover);
        assert!(config
            .collectors
            .steam_deck_fps
            .mangohud_log
            .paths
            .is_empty());
        assert!(!config.collectors.steam_deck_fps.gamescope_mangoapp.enabled);
        assert_eq!(
            config
                .collectors
                .steam_deck_fps
                .gamescope_mangoapp
                .ftok_path,
            PathBuf::from("mangoapp")
        );
        assert_eq!(
            config
                .collectors
                .steam_deck_fps
                .gamescope_mangoapp
                .project_id,
            65
        );
        assert!(
            !config
                .collectors
                .steam_deck_fps
                .gamescope_mangoapp
                .legacy_failed_ftok_fallback_enabled
        );
        assert!(
            !config
                .collectors
                .steam_deck_fps
                .gamescope_mangoapp
                .allow_destructive_read
        );
    }

    #[test]
    fn defaults_nvidia_nvml_config() {
        let config = AppConfig::default();

        assert_eq!(config.collection.gpu_interval_seconds, 15);
        assert!(config.collectors.nvidia_nvml.enabled);
        assert!(config.collectors.nvidia_nvml.library_paths.is_empty());
        assert!(config.collectors.nvidia_nvml.expose_gpu_name);
        assert!(!config.collectors.nvidia_nvml.expose_gpu_uuid);
        assert!(config.collectors.nvidia_nvml.fan_speed_enabled);
    }

    #[test]
    fn defaults_windows_collector_config() {
        let config = AppConfig::default();

        assert_eq!(config.collection.windows_baseline_interval_seconds, 15);
        assert_eq!(config.collection.windows_inventory_interval_seconds, 300);
        assert_eq!(
            config.collectors.windows_baseline.enabled,
            cfg!(target_os = "windows")
        );
        assert_eq!(
            config.collectors.windows_inventory.enabled,
            cfg!(target_os = "windows")
        );
        assert_eq!(
            config.collectors.windows_lhm_http.enabled,
            cfg!(target_os = "windows")
        );
        assert_eq!(
            config.collectors.windows_lhm_http.url,
            "http://127.0.0.1:8085/data.json"
        );
        assert_eq!(config.collectors.windows_lhm_http.timeout_ms, 1500);
        assert!(!config.collectors.windows_lhm_wmi.enabled);
        assert_eq!(
            config.collectors.windows_lhm_wmi.namespace,
            r"root\LibreHardwareMonitor"
        );
        assert!(!config.collectors.windows_baseline.include_removable_drives);
        assert!(!config.collectors.windows_baseline.include_remote_drives);
    }

    #[test]
    fn defaults_system_collector_config() {
        let config = AppConfig::default();

        assert_eq!(config.collection.system_interval_seconds, 15);
        assert!(config.collectors.system.enabled);
        assert!(config.collectors.system.cpu_enabled);
        assert!(config.collectors.system.memory_enabled);
        assert!(config.collectors.system.uptime_enabled);
    }

    #[test]
    fn defaults_macos_collector_config() {
        let config = AppConfig::default();

        assert_eq!(config.collection.macos_thermal_state_interval_seconds, 15);
        assert_eq!(
            config.collectors.macos_thermal_state.enabled,
            cfg!(target_os = "macos")
        );
        assert!(!config.collectors.macos_macmon.enabled);
        assert_eq!(config.collectors.macos_macmon.sample_interval_seconds, 1);
        assert_eq!(
            config.collectors.macos_macmon.sample_window_milliseconds,
            1000
        );
        assert_eq!(config.collectors.macos_macmon.stale_after_seconds, 5);
        assert_eq!(
            config
                .collectors
                .macos_macmon
                .reinitialize_after_consecutive_errors,
            5
        );
        assert!(
            !config
                .collectors
                .macos_exact_temperature_experimental
                .enabled
        );
    }

    #[test]
    fn rejects_zero_system_interval() {
        let mut config = AppConfig::default();
        config.collection.system_interval_seconds = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn defaults_diagnostics_config() {
        let config = AppConfig::default();

        assert!(config.diagnostics.enabled);
        assert_eq!(config.diagnostics.scrape_gap_threshold_seconds, 30);
        assert_eq!(config.diagnostics.scheduler_lag_threshold_seconds, 5);
        assert!(config.diagnostics.log_scrape_gaps);
        assert!(config.diagnostics.log_scheduler_lag);
        assert!(config.diagnostics.log_scrape_interval_changes);
    }

    #[test]
    fn rejects_zero_diagnostics_thresholds() {
        let mut config = AppConfig::default();
        config.diagnostics.scrape_gap_threshold_seconds = 0;
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.diagnostics.scheduler_lag_threshold_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_steam_deck_game_state_config() {
        let mut config = AppConfig::default();
        config
            .collectors
            .steam_deck_game_state
            .poll_interval_seconds = 0;

        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_game_state.enabled = true;
        config.collectors.steam_deck_game_state.xprop_path = String::new();

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_steam_deck_fps_config() {
        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.enabled = true;
        config.collectors.steam_deck_fps.windows_seconds.clear();
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.poll_interval_milliseconds = 0;
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.enabled = true;
        config.collectors.steam_deck_fps.source_preference.clear();
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.source_preference = vec!["unknown".to_string()];
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.mangohud_log.enabled = true;
        config.collectors.steam_deck_fps.mangohud_log.paths = vec![PathBuf::new()];
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.collectors.steam_deck_fps.gamescope_mangoapp.enabled = true;
        config
            .collectors
            .steam_deck_fps
            .gamescope_mangoapp
            .ftok_path = PathBuf::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_macos_thermal_interval() {
        let mut config = AppConfig::default();
        config.collection.macos_thermal_state_interval_seconds = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_macos_macmon_sample_interval() {
        let mut config = AppConfig::default();
        config.collectors.macos_macmon.sample_interval_seconds = 0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_macos_macmon_temperature_bounds() {
        let mut config = AppConfig::default();
        config.collectors.macos_macmon.min_temperature_celsius = 130.0;
        config.collectors.macos_macmon.max_temperature_celsius = 1.0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_macos_macmon_power_bound() {
        let mut config = AppConfig::default();
        config.collectors.macos_macmon.max_power_watts = 0.0;

        assert!(config.validate().is_err());
    }

    #[test]
    fn registration_requires_server_token_and_user_when_enabled() {
        let mut config = AppConfig::default();
        config.registration.enabled = true;

        assert!(config.validate().is_err());

        config.registration.registry_addr = "127.0.0.1:9186".to_string();
        config.registration.enrollment_token = "secret".to_string();
        config.identity.user_name = "example-user".to_string();

        config.validate().unwrap();
    }

    #[test]
    fn registry_config_is_valid_by_default() {
        RegistryAppConfig::default().validate().unwrap();
    }
}
