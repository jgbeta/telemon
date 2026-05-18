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
    pub fake_interval_seconds: u64,
    pub temperature_interval_seconds: u64,
    pub sensor_rescan_interval_seconds: u64,
    pub gpu_interval_seconds: u64,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectorsConfig {
    pub fake: FakeCollectorConfig,
    pub linux_hwmon: LinuxHwmonConfig,
    pub nvidia_nvml: NvidiaNvmlConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FakeCollectorConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxHwmonConfig {
    pub enabled: bool,
    pub root: PathBuf,
    pub include_unknown_sensors: bool,
    pub sensor_allowlist: Vec<String>,
    pub sensor_denylist: Vec<String>,
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
pub struct LoggingConfig {
    pub level: String,
}

impl AppConfig {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self = serde_yaml::from_str(&text)
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
        if self.server.static_metrics_path == self.server.metrics_path {
            bail!("server.static_metrics_path must differ from server.metrics_path");
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
            self.collection.fake_interval_seconds,
            "collection.fake_interval_seconds",
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
        let config: Self = serde_yaml::from_str(&text)
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
            fake_interval_seconds: 5,
            temperature_interval_seconds: 15,
            sensor_rescan_interval_seconds: 300,
            gpu_interval_seconds: 15,
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

impl Default for FakeCollectorConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for LinuxHwmonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root: PathBuf::from("/sys/class/hwmon"),
            include_unknown_sensors: false,
            sensor_allowlist: Vec::new(),
            sensor_denylist: Vec::new(),
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

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
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

    #[test]
    fn default_config_is_valid() {
        AppConfig::default().validate().unwrap();
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
        config.collection.fake_interval_seconds = 0;

        assert!(config.validate().is_err());
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
