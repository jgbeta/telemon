use std::collections::BTreeMap;
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info};
use uuid::Uuid;

use telemon_core::config::{RegistryAppConfig, RegistryConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceRecord {
    pub device_uuid: String,
    #[serde(default)]
    pub machine_uuid: String,
    pub user_name: String,
    pub device_name: String,
    pub os: String,
    #[serde(default)]
    pub os_version: String,
    pub arch: String,
    pub current_ip: String,
    #[serde(default)]
    pub observed_ip: String,
    #[serde(default)]
    pub advertised_addr: String,
    pub listen_port: u16,
    #[serde(default = "default_requested_scrape_interval_seconds")]
    pub requested_scrape_interval_seconds: u64,
    pub last_seen_timestamp: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DeviceStore {
    devices: BTreeMap<String, DeviceRecord>,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    enrollment_token: String,
    user_name: String,
    device_name: String,
    #[serde(default)]
    machine_uuid: String,
    os: String,
    #[serde(default)]
    os_version: String,
    arch: String,
    listen_port: u16,
    #[serde(default)]
    advertised_addr: String,
    #[serde(default = "default_requested_scrape_interval_seconds")]
    requested_scrape_interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    enrollment_token: String,
    device_uuid: String,
    user_name: String,
    device_name: String,
    #[serde(default)]
    machine_uuid: String,
    os: String,
    #[serde(default)]
    os_version: String,
    arch: String,
    listen_port: u16,
    #[serde(default)]
    advertised_addr: String,
    #[serde(default = "default_requested_scrape_interval_seconds")]
    requested_scrape_interval_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub device_uuid: String,
    pub observed_ip: String,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub device_uuid: String,
    pub observed_ip: String,
    pub ip_changed: bool,
}

#[derive(Debug, Serialize)]
struct PrometheusTargetGroup {
    targets: Vec<String>,
    labels: BTreeMap<String, String>,
}

#[derive(Clone)]
struct RegistryState {
    config: RegistryConfig,
    store: Arc<Mutex<DeviceStore>>,
}

impl DeviceStore {
    fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read registry store {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse registry store {}", path.display()))
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create registry store dir {}", parent.display())
            })?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(path, text)
            .with_context(|| format!("failed to write registry store {}", path.display()))
    }

    fn register(
        &mut self,
        request: RegisterRequest,
        observed_ip: String,
        now: u64,
    ) -> DeviceRecord {
        let device_uuid = Uuid::new_v4().to_string();
        let advertised_addr = normalize_advertised_addr(&request.advertised_addr);
        let current_ip = target_host(&advertised_addr, &observed_ip);
        let record = DeviceRecord {
            device_uuid: device_uuid.clone(),
            machine_uuid: normalized_machine_uuid(&request.machine_uuid, &device_uuid),
            user_name: request.user_name,
            device_name: request.device_name,
            os: request.os,
            os_version: request.os_version,
            arch: request.arch,
            current_ip,
            observed_ip,
            advertised_addr,
            listen_port: request.listen_port,
            requested_scrape_interval_seconds: normalize_requested_scrape_interval_seconds(
                request.requested_scrape_interval_seconds,
            ),
            last_seen_timestamp: now,
        };
        self.devices.insert(device_uuid, record.clone());
        record
    }

    fn heartbeat(
        &mut self,
        request: HeartbeatRequest,
        observed_ip: String,
        now: u64,
    ) -> Option<(DeviceRecord, bool)> {
        let record = self.devices.get_mut(&request.device_uuid)?;
        let advertised_addr = normalize_advertised_addr(&request.advertised_addr);
        let current_ip = target_host(&advertised_addr, &observed_ip);
        let ip_changed = record.current_ip != current_ip;
        record.machine_uuid = normalized_machine_uuid(&request.machine_uuid, &record.device_uuid);
        record.user_name = request.user_name;
        record.device_name = request.device_name;
        record.os = request.os;
        record.os_version = request.os_version;
        record.arch = request.arch;
        record.current_ip = current_ip;
        record.observed_ip = observed_ip;
        record.advertised_addr = advertised_addr;
        record.listen_port = request.listen_port;
        record.requested_scrape_interval_seconds =
            normalize_requested_scrape_interval_seconds(request.requested_scrape_interval_seconds);
        record.last_seen_timestamp = now;
        Some((record.clone(), ip_changed))
    }

    fn target_groups(&self, stale_after_seconds: u64, now: u64) -> Vec<PrometheusTargetGroup> {
        self.target_groups_for_interval(stale_after_seconds, now, None)
    }

    fn target_groups_for_interval(
        &self,
        stale_after_seconds: u64,
        now: u64,
        interval_seconds: Option<u64>,
    ) -> Vec<PrometheusTargetGroup> {
        self.devices
            .values()
            .filter(|record| now.saturating_sub(record.last_seen_timestamp) <= stale_after_seconds)
            .filter(|record| {
                interval_seconds
                    .map(|interval| record.requested_scrape_interval_seconds == interval)
                    .unwrap_or(true)
            })
            .map(device_record_to_target_group)
            .collect()
    }
}

pub async fn run(config: RegistryAppConfig) -> Result<()> {
    let addr = config.listen_addr()?;
    let store = DeviceStore::load(&config.registry.storage_path)?;
    let listener = TcpListener::bind(addr).await?;
    let state = Arc::new(RegistryState {
        config: config.registry,
        store: Arc::new(Mutex::new(store)),
    });

    info!(listen = %addr, "registry server listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, state).await {
                        debug!(%error, "registry request failed");
                    }
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for ctrl+c")?;
                info!("registry shutdown signal received");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_connection(mut stream: TcpStream, state: Arc<RegistryState>) -> Result<()> {
    let peer_ip = stream.peer_addr()?.ip();
    let request = read_request(&mut stream).await?;

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => {
            write_response(&mut stream, 200, "text/plain; charset=utf-8", "ok\n").await?;
        }
        ("GET", "/prometheus/sd") => {
            let groups = {
                let store = state.store.lock().expect("registry store mutex poisoned");
                store.target_groups(
                    state.config.device_stale_after_seconds,
                    unix_timestamp_seconds(),
                )
            };
            write_json(&mut stream, 200, &groups).await?;
        }
        ("GET", path) if path.starts_with("/prometheus/sd/") => {
            let interval = parse_sd_interval(path.trim_start_matches("/prometheus/sd/"));
            let Some(interval) = interval else {
                write_json_error(&mut stream, 404, "unknown scrape interval").await?;
                return Ok(());
            };
            let groups = {
                let store = state.store.lock().expect("registry store mutex poisoned");
                store.target_groups_for_interval(
                    state.config.device_stale_after_seconds,
                    unix_timestamp_seconds(),
                    Some(interval),
                )
            };
            write_json(&mut stream, 200, &groups).await?;
        }
        ("POST", "/api/v1/register") => {
            let request: RegisterRequest = serde_json::from_slice(&request.body)
                .context("failed to parse register request")?;
            if !token_is_valid(&state.config, &request.enrollment_token) {
                write_json_error(&mut stream, 401, "invalid enrollment token").await?;
                return Ok(());
            }
            let record = {
                let mut store = state.store.lock().expect("registry store mutex poisoned");
                let record = store.register(
                    request,
                    observed_ip_label(peer_ip),
                    unix_timestamp_seconds(),
                );
                store.save(&state.config.storage_path)?;
                record
            };
            write_json(
                &mut stream,
                200,
                &RegisterResponse {
                    device_uuid: record.device_uuid,
                    observed_ip: record.observed_ip,
                },
            )
            .await?;
        }
        ("POST", "/api/v1/heartbeat") => {
            let request: HeartbeatRequest = serde_json::from_slice(&request.body)
                .context("failed to parse heartbeat request")?;
            if !token_is_valid(&state.config, &request.enrollment_token) {
                write_json_error(&mut stream, 401, "invalid enrollment token").await?;
                return Ok(());
            }
            let heartbeat_result = {
                let mut store = state.store.lock().expect("registry store mutex poisoned");
                let result = store.heartbeat(
                    request,
                    observed_ip_label(peer_ip),
                    unix_timestamp_seconds(),
                );
                if result.is_some() {
                    store.save(&state.config.storage_path)?;
                }
                result
            };
            let Some((record, ip_changed)) = heartbeat_result else {
                write_json_error(&mut stream, 404, "unknown device_uuid").await?;
                return Ok(());
            };
            write_json(
                &mut stream,
                200,
                &HeartbeatResponse {
                    device_uuid: record.device_uuid,
                    observed_ip: record.observed_ip,
                    ip_changed,
                },
            )
            .await?;
        }
        _ => {
            write_json_error(&mut stream, 404, "not found").await?;
        }
    }

    Ok(())
}

fn token_is_valid(config: &RegistryConfig, token: &str) -> bool {
    token == config.enrollment_token
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("connection closed before headers");
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
        if buffer.len() > 64 * 1024 {
            bail!("request headers too large");
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines.next().context("missing request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let body_start = header_end + 4;
    while buffer.len().saturating_sub(body_start) < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("connection closed before body");
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(HttpRequest {
        method,
        path,
        body: buffer[body_start..body_start + content_length].to_vec(),
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, body: &T) -> Result<()> {
    let body = serde_json::to_string(body)?;
    write_response(stream, status, "application/json", &body).await
}

async fn write_json_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    write_json(
        stream,
        status,
        &serde_json::json!({
            "error": message,
        }),
    )
    .await
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn device_record_to_target_group(record: &DeviceRecord) -> PrometheusTargetGroup {
    let target = format_target(record.current_ip.as_str(), record.listen_port);
    let mut labels = BTreeMap::new();
    labels.insert("device_uuid".to_string(), record.device_uuid.clone());
    labels.insert(
        "machine_uuid".to_string(),
        normalized_machine_uuid(&record.machine_uuid, &record.device_uuid),
    );
    labels.insert("device_name".to_string(), record.device_name.clone());
    labels.insert("user_name".to_string(), record.user_name.clone());
    labels.insert("host".to_string(), record.device_name.clone());
    labels.insert("os".to_string(), record.os.clone());
    labels.insert("os_version".to_string(), record.os_version.clone());
    labels.insert("arch".to_string(), record.arch.clone());
    labels.insert(
        "requested_scrape_interval_seconds".to_string(),
        record.requested_scrape_interval_seconds.to_string(),
    );

    PrometheusTargetGroup {
        targets: vec![target],
        labels,
    }
}

fn format_target(ip: &str, port: u16) -> String {
    if ip.contains(':') {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    }
}

fn normalize_advertised_addr(value: &str) -> String {
    value.trim().to_string()
}

fn target_host(advertised_addr: &str, observed_ip: &str) -> String {
    if advertised_addr.is_empty() {
        observed_ip.to_string()
    } else {
        advertised_addr.to_string()
    }
}

fn parse_sd_interval(value: &str) -> Option<u64> {
    let parsed = value
        .strip_suffix('s')
        .unwrap_or(value)
        .parse::<u64>()
        .ok()?;
    match parsed {
        1 | 5 | 10 | 15 => Some(parsed),
        _ => None,
    }
}

fn normalize_requested_scrape_interval_seconds(value: u64) -> u64 {
    match value {
        0 => default_requested_scrape_interval_seconds(),
        1 => 1,
        2..=5 => 5,
        6..=10 => 10,
        _ => 15,
    }
}

fn default_requested_scrape_interval_seconds() -> u64 {
    15
}

fn normalized_machine_uuid(machine_uuid: &str, device_uuid: &str) -> String {
    let trimmed = machine_uuid.trim();
    if trimmed.is_empty() {
        device_uuid.to_string()
    } else {
        trimmed.to_string()
    }
}

fn observed_ip_label(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(value) => value.to_string(),
        IpAddr::V6(value) => value
            .to_ipv4_mapped()
            .map(|mapped| mapped.to_string())
            .unwrap_or_else(|| value.to_string()),
    }
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register_request() -> RegisterRequest {
        RegisterRequest {
            enrollment_token: "secret".to_string(),
            user_name: "example-user".to_string(),
            device_name: "workstation".to_string(),
            machine_uuid: "machine".to_string(),
            os: "linux".to_string(),
            os_version: "Linux".to_string(),
            arch: "x86_64".to_string(),
            listen_port: 9185,
            advertised_addr: String::new(),
            requested_scrape_interval_seconds: 15,
        }
    }

    #[test]
    fn register_creates_uuid_record() {
        let mut store = DeviceStore::default();

        let record = store.register(register_request(), "203.0.113.76".to_string(), 100);

        assert!(Uuid::parse_str(&record.device_uuid).is_ok());
        assert_eq!(record.current_ip, "203.0.113.76");
        assert_eq!(store.devices.len(), 1);
    }

    #[test]
    fn heartbeat_updates_ip_and_last_seen() {
        let mut store = DeviceStore::default();
        let record = store.register(register_request(), "203.0.113.76".to_string(), 100);

        let (updated, changed) = store
            .heartbeat(
                HeartbeatRequest {
                    enrollment_token: "secret".to_string(),
                    device_uuid: record.device_uuid,
                    user_name: "example-user".to_string(),
                    device_name: "workstation".to_string(),
                    machine_uuid: "machine".to_string(),
                    os: "linux".to_string(),
                    os_version: "Linux".to_string(),
                    arch: "x86_64".to_string(),
                    listen_port: 9185,
                    advertised_addr: String::new(),
                    requested_scrape_interval_seconds: 5,
                },
                "203.0.113.99".to_string(),
                130,
            )
            .unwrap();

        assert!(changed);
        assert_eq!(updated.current_ip, "203.0.113.99");
        assert_eq!(updated.last_seen_timestamp, 130);
    }

    #[test]
    fn target_groups_exclude_stale_devices() {
        let mut store = DeviceStore::default();
        store.register(register_request(), "203.0.113.76".to_string(), 100);

        assert_eq!(store.target_groups(120, 219).len(), 1);
        assert_eq!(store.target_groups(120, 221).len(), 0);
    }

    #[test]
    fn target_group_includes_identity_labels() {
        let record = DeviceRecord {
            device_uuid: "uuid".to_string(),
            machine_uuid: "machine".to_string(),
            user_name: "example-user".to_string(),
            device_name: "workstation".to_string(),
            os: "linux".to_string(),
            os_version: "Linux".to_string(),
            arch: "x86_64".to_string(),
            current_ip: "203.0.113.76".to_string(),
            observed_ip: "203.0.113.76".to_string(),
            advertised_addr: String::new(),
            listen_port: 9185,
            requested_scrape_interval_seconds: 15,
            last_seen_timestamp: 100,
        };

        let group = device_record_to_target_group(&record);

        assert_eq!(group.targets, vec!["203.0.113.76:9185"]);
        assert_eq!(
            group.labels.get("device_uuid").map(String::as_str),
            Some("uuid")
        );
        assert_eq!(
            group.labels.get("device_name").map(String::as_str),
            Some("workstation")
        );
        assert_eq!(
            group.labels.get("host").map(String::as_str),
            Some("workstation")
        );
        assert_eq!(
            group.labels.get("machine_uuid").map(String::as_str),
            Some("machine")
        );
    }

    #[test]
    fn advertised_addr_overrides_observed_ip_for_targets() {
        let mut request = register_request();
        request.advertised_addr = "203.0.113.76".to_string();
        let mut store = DeviceStore::default();

        let record = store.register(request, "198.51.100.1".to_string(), 100);
        let group = device_record_to_target_group(&record);

        assert_eq!(record.observed_ip, "198.51.100.1");
        assert_eq!(record.current_ip, "203.0.113.76");
        assert_eq!(group.targets, vec!["203.0.113.76:9185"]);
    }

    #[test]
    fn target_groups_filter_by_requested_interval() {
        let mut request = register_request();
        request.requested_scrape_interval_seconds = 1;
        let mut store = DeviceStore::default();
        store.register(request, "203.0.113.76".to_string(), 100);

        assert_eq!(store.target_groups_for_interval(120, 100, Some(1)).len(), 1);
        assert_eq!(
            store.target_groups_for_interval(120, 100, Some(15)).len(),
            0
        );
    }

    #[test]
    fn service_discovery_interval_paths_are_exact() {
        assert_eq!(parse_sd_interval("1s"), Some(1));
        assert_eq!(parse_sd_interval("5s"), Some(5));
        assert_eq!(parse_sd_interval("10s"), Some(10));
        assert_eq!(parse_sd_interval("15s"), Some(15));
        assert_eq!(parse_sd_interval("7s"), None);
        assert_eq!(parse_sd_interval("999s"), None);
    }

    #[test]
    fn old_device_records_deserialize_without_new_fields() {
        let text = r#"{
          "devices": {
            "uuid": {
              "device_uuid": "uuid",
              "user_name": "example-user",
              "device_name": "workstation",
              "os": "linux",
              "arch": "x86_64",
              "current_ip": "203.0.113.76",
              "listen_port": 9185,
              "last_seen_timestamp": 100
            }
          }
        }"#;

        let store: DeviceStore = serde_json::from_str(text).unwrap();
        let record = store.devices.get("uuid").unwrap();

        assert_eq!(record.current_ip, "203.0.113.76");
        assert_eq!(record.observed_ip, "");
        assert_eq!(record.advertised_addr, "");
        assert_eq!(record.requested_scrape_interval_seconds, 15);
    }
}
