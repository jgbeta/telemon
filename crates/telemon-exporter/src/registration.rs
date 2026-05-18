use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adaptive::AdaptiveSamplingState;
use telemon_collectors::traits::{Collector, CollectorResult};
use telemon_core::config::{AppConfig, RegistrationConfig};
use telemon_core::metrics::model::{labels, MetricSample};
use telemon_core::metrics::names;

#[derive(Debug, Serialize)]
struct RegisterRequest {
    enrollment_token: String,
    user_name: String,
    device_name: String,
    machine_uuid: String,
    os: String,
    os_version: String,
    arch: String,
    listen_port: u16,
    advertised_addr: String,
    requested_scrape_interval_seconds: u64,
}

#[derive(Debug, Serialize)]
struct HeartbeatRequest {
    enrollment_token: String,
    device_uuid: String,
    user_name: String,
    device_name: String,
    machine_uuid: String,
    os: String,
    os_version: String,
    arch: String,
    listen_port: u16,
    advertised_addr: String,
    requested_scrape_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    device_uuid: String,
    observed_ip: String,
}

#[derive(Debug, Deserialize)]
struct HeartbeatResponse {
    device_uuid: String,
    observed_ip: String,
    ip_changed: bool,
}

pub struct DeviceInfoCollector {
    config: AppConfig,
}

impl DeviceInfoCollector {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }
}

impl Collector for DeviceInfoCollector {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn collect(&mut self) -> CollectorResult {
        let started_at = std::time::Instant::now();
        let metrics = device_info_metric(&self.config).into_iter().collect();
        CollectorResult::success(self.name(), metrics, started_at)
    }
}

pub async fn run_client(
    config: AppConfig,
    adaptive_state: AdaptiveSamplingState,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.registration.enabled {
        return;
    }

    let device_id_file = device_id_file(&config.registration);
    let heartbeat_interval = Duration::from_secs(config.registration.heartbeat_interval_seconds);
    let poll_interval = heartbeat_interval.min(Duration::from_secs(5));
    let mut last_heartbeat_at: Option<Instant> = None;
    let mut last_sent_requested_interval: Option<u64> = None;

    loop {
        if *shutdown.borrow() {
            break;
        }

        let requested_interval = adaptive_state.requested_interval_seconds();
        let heartbeat_due = last_heartbeat_at
            .map(|last| last.elapsed() >= heartbeat_interval)
            .unwrap_or(true);
        let interval_changed = last_sent_requested_interval
            .map(|last| last != requested_interval)
            .unwrap_or(false);

        if heartbeat_due || interval_changed {
            let result = registration_iteration(&config, &adaptive_state, &device_id_file).await;
            match result {
                Ok(()) => {
                    last_heartbeat_at = Some(Instant::now());
                    last_sent_requested_interval = Some(requested_interval);
                }
                Err(error) => {
                    last_heartbeat_at = Some(Instant::now());
                    warn!(%error, "device registration heartbeat failed");
                }
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(poll_interval) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

pub fn device_info_metric(config: &AppConfig) -> Option<MetricSample> {
    if !config.registration.enabled {
        return None;
    }

    let device_uuid = read_device_uuid(&device_id_file(&config.registration)).ok()??;
    let machine_uuid = machine_uuid(config).ok()?;
    Some(MetricSample::gauge(
        names::DEVICE_INFO,
        "Device identity assigned by the Telemon registry.",
        labels(&[
            ("device_uuid", device_uuid.as_str()),
            ("machine_uuid", machine_uuid.as_str()),
            ("user_name", effective_user_name(config).as_str()),
            ("device_name", effective_device_name(config).as_str()),
            ("os", std::env::consts::OS),
            ("os_version", os_version().as_str()),
            ("arch", std::env::consts::ARCH),
        ]),
        1.0,
    ))
}

async fn registration_iteration(
    config: &AppConfig,
    adaptive_state: &AdaptiveSamplingState,
    device_id_file: &PathBuf,
) -> Result<()> {
    let Some(device_uuid) = read_device_uuid(device_id_file)? else {
        let response = register(config, adaptive_state).await?;
        write_device_uuid(device_id_file, &response.device_uuid)?;
        update_observed_ip(device_id_file, &response.observed_ip)?;
        info!(
            device_uuid = response.device_uuid,
            observed_ip = response.observed_ip,
            "device registered"
        );
        return Ok(());
    };

    match heartbeat(config, adaptive_state, &device_uuid).await {
        Ok(response) => {
            update_observed_ip(device_id_file, &response.observed_ip)?;
            if response.ip_changed {
                info!(
                    device_uuid = response.device_uuid,
                    observed_ip = response.observed_ip,
                    "registry observed device IP change"
                );
            }
            Ok(())
        }
        Err(error) if error.to_string().contains("http status 404") => {
            warn!(
                device_uuid,
                "registry no longer recognizes device UUID; registering again"
            );
            let _ = fs::remove_file(device_id_file);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

async fn register(
    config: &AppConfig,
    adaptive_state: &AdaptiveSamplingState,
) -> Result<RegisterResponse> {
    let request = RegisterRequest {
        enrollment_token: config.registration.enrollment_token.clone(),
        user_name: effective_user_name(config),
        device_name: effective_device_name(config),
        machine_uuid: machine_uuid(config)?,
        os: std::env::consts::OS.to_string(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.to_string(),
        listen_port: config.registration.scrape_port,
        advertised_addr: advertised_addr(&config.registration),
        requested_scrape_interval_seconds: adaptive_state.requested_interval_seconds(),
    };
    post_json(
        &config.registration.registry_addr,
        "/api/v1/register",
        &request,
    )
    .await
}

async fn heartbeat(
    config: &AppConfig,
    adaptive_state: &AdaptiveSamplingState,
    device_uuid: &str,
) -> Result<HeartbeatResponse> {
    let request = HeartbeatRequest {
        enrollment_token: config.registration.enrollment_token.clone(),
        device_uuid: device_uuid.to_string(),
        user_name: effective_user_name(config),
        device_name: effective_device_name(config),
        machine_uuid: machine_uuid(config)?,
        os: std::env::consts::OS.to_string(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.to_string(),
        listen_port: config.registration.scrape_port,
        advertised_addr: advertised_addr(&config.registration),
        requested_scrape_interval_seconds: adaptive_state.requested_interval_seconds(),
    };
    post_json(
        &config.registration.registry_addr,
        "/api/v1/heartbeat",
        &request,
    )
    .await
}

async fn post_json<T, R>(registry_addr: &str, path: &str, request: &T) -> Result<R>
where
    T: Serialize,
    R: for<'de> Deserialize<'de>,
{
    let addr = normalize_registry_addr(registry_addr)?;
    let body = serde_json::to_string(request)?;
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("failed to connect to registry {addr}"))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let (status, body) = parse_http_response(&response)?;
    if status != 200 {
        bail!("registry returned http status {status}");
    }
    serde_json::from_slice(body).context("failed to parse registry response")
}

fn parse_http_response(response: &[u8]) -> Result<(u16, &[u8])> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        bail!("registry response missing headers");
    };
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .context("registry response missing status")?;
    Ok((status, &response[header_end + 4..]))
}

fn normalize_registry_addr(value: &str) -> Result<String> {
    let trimmed = value
        .trim()
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("registration.registry_addr is empty");
    }
    if trimmed.starts_with("https://") {
        bail!("registration.registry_addr only supports plain HTTP for now");
    }
    Ok(trimmed.to_string())
}

fn advertised_addr(config: &RegistrationConfig) -> String {
    config.advertised_addr.trim().to_string()
}

pub fn device_id_file(config: &RegistrationConfig) -> PathBuf {
    if !config.device_id_file.as_os_str().is_empty() {
        return config.device_id_file.clone();
    }

    if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\ProgramData\Telemon\state\device-id")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/Telemon/state/device-id")
    } else {
        PathBuf::from("/var/lib/telemon/exporter/device-id")
    }
}

fn last_ip_file(device_id_file: &Path) -> PathBuf {
    device_id_file.with_file_name("last-observed-ip")
}

pub fn machine_uuid(config: &AppConfig) -> Result<String> {
    let configured = config.identity.machine_uuid.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }

    let path = machine_uuid_file(config);
    if let Some(value) = read_device_uuid(&path)? {
        return Ok(value);
    }

    let value = Uuid::new_v4().to_string();
    write_device_uuid(&path, &value)?;
    Ok(value)
}

pub fn machine_uuid_file(config: &AppConfig) -> PathBuf {
    if !config.identity.machine_uuid_file.as_os_str().is_empty() {
        return config.identity.machine_uuid_file.clone();
    }

    let device_id_file = device_id_file(&config.registration);
    device_id_file.with_file_name("machine-id")
}

fn read_device_uuid(path: &PathBuf) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(path)
        .with_context(|| format!("failed to read device UUID file {}", path.display()))?
        .trim()
        .to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn write_device_uuid(path: &PathBuf, device_uuid: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create device state dir {}", parent.display()))?;
    }
    fs::write(path, format!("{device_uuid}\n"))
        .with_context(|| format!("failed to write device UUID file {}", path.display()))
}

fn update_observed_ip(device_id_file: &Path, observed_ip: &str) -> Result<()> {
    let path = last_ip_file(device_id_file);
    let previous = fs::read_to_string(&path)
        .ok()
        .map(|value| value.trim().to_string());
    if previous.as_deref() != Some(observed_ip) {
        if let Some(previous) = previous.filter(|value| !value.is_empty()) {
            info!(
                previous_ip = previous,
                observed_ip, "device observed IP changed"
            );
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create observed IP state dir {}",
                    parent.display()
                )
            })?;
        }
        fs::write(&path, format!("{observed_ip}\n"))
            .with_context(|| format!("failed to write observed IP file {}", path.display()))?;
    }
    Ok(())
}

fn effective_user_name(config: &AppConfig) -> String {
    config.identity.user_name.trim().to_string()
}

fn effective_device_name(config: &AppConfig) -> String {
    let configured = config.identity.device_name.trim();
    if !configured.is_empty() {
        configured.to_string()
    } else {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "unknown-device".to_string())
    }
}

fn os_version() -> String {
    if cfg!(target_os = "linux") {
        if let Ok(text) = fs::read_to_string("/etc/os-release") {
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                    return value.trim_matches('"').to_string();
                }
            }
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_registry_addr() {
        assert_eq!(
            normalize_registry_addr("http://127.0.0.1:9186/").unwrap(),
            "127.0.0.1:9186"
        );
        assert!(normalize_registry_addr("https://127.0.0.1:9186").is_err());
    }

    #[test]
    fn parses_http_response_status_and_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";

        let (status, body) = parse_http_response(response).unwrap();

        assert_eq!(status, 200);
        assert_eq!(body, b"{}");
    }
}
