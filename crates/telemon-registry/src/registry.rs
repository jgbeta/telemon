use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};
use tracing::{debug, info};
use uuid::Uuid;

use telemon_core::config::{RegistryAppConfig, RegistryConfig};

const MAX_CONCURRENT_CONNECTIONS: usize = 128;
const REQUEST_HEADER_LIMIT_BYTES: usize = 64 * 1024;
const REQUEST_BODY_LIMIT_BYTES: usize = 16 * 1024;
const REQUEST_READ_TIMEOUT_SECONDS: u64 = 5;

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
        atomic_write(path, text.as_bytes())
            .with_context(|| format!("failed to write registry store {}", path.display()))
    }

    fn register(
        &mut self,
        request: RegisterRequest,
        observed_ip: String,
        now: u64,
    ) -> DeviceRecord {
        let device_uuid = Uuid::new_v4().to_string();
        let advertised_addr = request.advertised_addr;
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
        let advertised_addr = request.advertised_addr;
        let current_ip = target_host(&advertised_addr, &observed_ip);
        let ip_changed = record.current_ip != current_ip;
        let previous_requested_scrape_interval_seconds = record.requested_scrape_interval_seconds;
        let requested_scrape_interval_seconds =
            normalize_requested_scrape_interval_seconds(request.requested_scrape_interval_seconds);
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
        record.requested_scrape_interval_seconds = requested_scrape_interval_seconds;
        record.last_seen_timestamp = now;
        if previous_requested_scrape_interval_seconds != requested_scrape_interval_seconds {
            info!(
                device_uuid = %record.device_uuid,
                device_name = %record.device_name,
                target_host = %record.current_ip,
                previous_interval_seconds = previous_requested_scrape_interval_seconds,
                requested_interval_seconds = requested_scrape_interval_seconds,
                "device requested scrape interval changed"
            );
        }
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
            .filter_map(device_record_to_target_group)
            .collect()
    }
}

pub async fn run(config: RegistryAppConfig) -> Result<()> {
    let addr = config.listen_addr()?;
    let store = DeviceStore::load(&config.registry.storage_path)?;
    let listener = TcpListener::bind(addr).await?;
    let connection_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let state = Arc::new(RegistryState {
        config: config.registry,
        store: Arc::new(Mutex::new(store)),
    });

    info!(listen = %addr, "registry server listening");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&connection_permits).try_acquire_owned() else {
                    tokio::spawn(async move {
                        let mut stream = stream;
                        let _ = write_json_error(&mut stream, 503, "server busy").await;
                    });
                    continue;
                };
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let _permit = permit;
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
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(error) => {
            let (status, message) = error.response();
            write_json_error(&mut stream, status, message).await?;
            return Ok(());
        }
    };

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/healthz") => {
            write_response(&mut stream, 200, "text/plain; charset=utf-8", "ok\n").await?;
        }
        ("GET", "/prometheus/sd") => {
            let groups = {
                let store = lock_device_store(state.store.as_ref());
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
                let store = lock_device_store(state.store.as_ref());
                store.target_groups_for_interval(
                    state.config.device_stale_after_seconds,
                    unix_timestamp_seconds(),
                    Some(interval),
                )
            };
            write_json(&mut stream, 200, &groups).await?;
        }
        ("POST", "/api/v1/register") => {
            let mut request: RegisterRequest = match serde_json::from_slice(&request.body) {
                Ok(request) => request,
                Err(_) => {
                    write_json_error(&mut stream, 400, "failed to parse register request").await?;
                    return Ok(());
                }
            };
            if !token_is_valid(&state.config, &request.enrollment_token) {
                write_json_error(&mut stream, 401, "invalid enrollment token").await?;
                return Ok(());
            }
            if let Err(message) = normalize_request_advertised_addr(&mut request.advertised_addr) {
                write_json_error(&mut stream, 400, message).await?;
                return Ok(());
            }
            let record = {
                let mut store = lock_device_store(state.store.as_ref());
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
            let mut request: HeartbeatRequest = match serde_json::from_slice(&request.body) {
                Ok(request) => request,
                Err(_) => {
                    write_json_error(&mut stream, 400, "failed to parse heartbeat request").await?;
                    return Ok(());
                }
            };
            if !token_is_valid(&state.config, &request.enrollment_token) {
                write_json_error(&mut stream, 401, "invalid enrollment token").await?;
                return Ok(());
            }
            if let Err(message) = normalize_request_advertised_addr(&mut request.advertised_addr) {
                write_json_error(&mut stream, 400, message).await?;
                return Ok(());
            }
            let heartbeat_result = {
                let mut store = lock_device_store(state.store.as_ref());
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
    constant_time_eq(token.as_bytes(), config.enrollment_token.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn lock_device_store(store: &Mutex<DeviceStore>) -> MutexGuard<'_, DeviceStore> {
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[derive(Debug)]
enum RequestReadError {
    Timeout,
    PayloadTooLarge,
    BadRequest(&'static str),
    Io,
}

impl RequestReadError {
    fn response(&self) -> (u16, &'static str) {
        match self {
            Self::Timeout => (408, "request timeout"),
            Self::PayloadTooLarge => (413, "request body too large"),
            Self::BadRequest(message) => (400, message),
            Self::Io => (400, "bad request"),
        }
    }
}

async fn read_request(
    stream: &mut TcpStream,
) -> std::result::Result<HttpRequest, RequestReadError> {
    match timeout(
        Duration::from_secs(REQUEST_READ_TIMEOUT_SECONDS),
        read_request_inner(stream),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(RequestReadError::Timeout),
    }
}

async fn read_request_inner(
    stream: &mut TcpStream,
) -> std::result::Result<HttpRequest, RequestReadError> {
    let mut buffer = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestReadError::Io)?;
        if read == 0 {
            return Err(RequestReadError::BadRequest(
                "connection closed before headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
        if buffer.len() > REQUEST_HEADER_LIMIT_BYTES {
            return Err(RequestReadError::BadRequest("request headers too large"));
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines
        .next()
        .ok_or(RequestReadError::BadRequest("missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err(RequestReadError::BadRequest("invalid request line"));
    }
    let content_length = parse_content_length(&headers)?;
    if content_length > REQUEST_BODY_LIMIT_BYTES {
        return Err(RequestReadError::PayloadTooLarge);
    }

    let body_start = header_end + 4;
    while buffer.len().saturating_sub(body_start) < content_length {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestReadError::Io)?;
        if read == 0 {
            return Err(RequestReadError::BadRequest(
                "connection closed before body",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(HttpRequest {
        method,
        path,
        body: buffer[body_start..body_start + content_length].to_vec(),
    })
}

fn parse_content_length(headers: &str) -> std::result::Result<usize, RequestReadError> {
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|_| RequestReadError::BadRequest("invalid content-length"));
        }
    }
    Ok(0)
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
        408 => "Request Timeout",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn device_record_to_target_group(record: &DeviceRecord) -> Option<PrometheusTargetGroup> {
    let target_host = effective_target_host(record)?;
    let target = format_target(&target_host, record.listen_port);
    let mut labels = BTreeMap::new();
    labels.insert("device_uuid".to_string(), record.device_uuid.clone());
    labels.insert(
        "machine_uuid".to_string(),
        normalized_machine_uuid(&record.machine_uuid, &record.device_uuid),
    );
    labels.insert("device_name".to_string(), record.device_name.clone());
    labels.insert("user_name".to_string(), record.user_name.clone());
    labels.insert("host".to_string(), record.device_name.clone());
    labels.insert("target_host".to_string(), target_host);
    labels.insert("os".to_string(), record.os.clone());
    labels.insert("os_version".to_string(), record.os_version.clone());
    labels.insert("arch".to_string(), record.arch.clone());
    labels.insert(
        "requested_scrape_interval_seconds".to_string(),
        record.requested_scrape_interval_seconds.to_string(),
    );

    Some(PrometheusTargetGroup {
        targets: vec![target],
        labels,
    })
}

fn format_target(ip: &str, port: u16) -> String {
    if ip.contains(':') {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    }
}

fn normalize_advertised_addr(value: &str) -> std::result::Result<String, &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if is_valid_target_host(trimmed) {
        Ok(trimmed.to_string())
    } else {
        Err("advertised_addr must be a host name or IP address without a port, scheme, or path")
    }
}

fn normalize_request_advertised_addr(value: &mut String) -> std::result::Result<(), &'static str> {
    let normalized = normalize_advertised_addr(value)?;
    *value = normalized;
    Ok(())
}

fn target_host(advertised_addr: &str, observed_ip: &str) -> String {
    if advertised_addr.is_empty() {
        observed_ip.to_string()
    } else {
        advertised_addr.to_string()
    }
}

fn effective_target_host(record: &DeviceRecord) -> Option<String> {
    let candidates = [
        record.current_ip.trim(),
        record.advertised_addr.trim(),
        record.observed_ip.trim(),
    ];
    candidates
        .into_iter()
        .find(|candidate| is_valid_target_host(candidate))
        .map(str::to_string)
}

fn is_valid_target_host(value: &str) -> bool {
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    is_valid_dns_hostname(value)
}

fn is_valid_dns_hostname(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 || !value.is_ascii() {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp_path = temp_store_path(path);
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("failed to open temp store {}", temp_path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write temp store {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync temp store {}", temp_path.display()))?;
        drop(file);
        replace_file(&temp_path, path)?;
        sync_parent_dir(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn temp_store_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("devices.json");
    path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()))
}

fn replace_file(temp_path: &Path, destination: &Path) -> Result<()> {
    match fs::rename(temp_path, destination) {
        Ok(()) => Ok(()),
        Err(_error) if cfg!(windows) && destination.exists() => {
            fs::remove_file(destination).with_context(|| {
                format!(
                    "failed to remove existing registry store {}",
                    destination.display()
                )
            })?;
            fs::rename(temp_path, destination).with_context(|| {
                format!(
                    "failed to rename temp store {} to {}",
                    temp_path.display(),
                    destination.display()
                )
            })
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to rename temp store {} to {}",
                temp_path.display(),
                destination.display()
            )
        }),
    }
}

fn sync_parent_dir(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
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

        let group = device_record_to_target_group(&record).unwrap();

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
            group.labels.get("target_host").map(String::as_str),
            Some("203.0.113.76")
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
        let group = device_record_to_target_group(&record).unwrap();

        assert_eq!(record.observed_ip, "198.51.100.1");
        assert_eq!(record.current_ip, "203.0.113.76");
        assert_eq!(group.targets, vec!["203.0.113.76:9185"]);
        assert_eq!(
            group.labels.get("target_host").map(String::as_str),
            Some("203.0.113.76")
        );
    }

    #[test]
    fn advertised_addr_allows_hostnames_and_ip_literals_only() {
        for value in [
            "deck",
            "deck.lan",
            "telemon-exporter.example.local",
            "203.0.113.76",
            "2001:db8::1",
        ] {
            assert_eq!(normalize_advertised_addr(value).unwrap(), value);
        }

        for value in [
            "http://deck.lan",
            "deck.lan:9185",
            "deck.lan/path",
            "[2001:db8::1]",
            "bad_host",
            "-deck.lan",
            "deck-.lan",
            "deck lan",
        ] {
            assert!(normalize_advertised_addr(value).is_err(), "{value}");
        }
    }

    #[test]
    fn unsafe_legacy_current_ip_falls_back_to_observed_ip() {
        let record = DeviceRecord {
            device_uuid: "uuid".to_string(),
            machine_uuid: "machine".to_string(),
            user_name: "example-user".to_string(),
            device_name: "workstation".to_string(),
            os: "linux".to_string(),
            os_version: "Linux".to_string(),
            arch: "x86_64".to_string(),
            current_ip: "http://attacker.example/path".to_string(),
            observed_ip: "203.0.113.76".to_string(),
            advertised_addr: "http://attacker.example/path".to_string(),
            listen_port: 9185,
            requested_scrape_interval_seconds: 15,
            last_seen_timestamp: 100,
        };

        let group = device_record_to_target_group(&record).unwrap();

        assert_eq!(group.targets, vec!["203.0.113.76:9185"]);
        assert_eq!(
            group.labels.get("target_host").map(String::as_str),
            Some("203.0.113.76")
        );
    }

    #[test]
    fn invalid_legacy_target_without_fallback_is_dropped() {
        let record = DeviceRecord {
            device_uuid: "uuid".to_string(),
            machine_uuid: "machine".to_string(),
            user_name: "example-user".to_string(),
            device_name: "workstation".to_string(),
            os: "linux".to_string(),
            os_version: "Linux".to_string(),
            arch: "x86_64".to_string(),
            current_ip: "http://attacker.example/path".to_string(),
            observed_ip: String::new(),
            advertised_addr: String::new(),
            listen_port: 9185,
            requested_scrape_interval_seconds: 15,
            last_seen_timestamp: 100,
        };

        assert!(device_record_to_target_group(&record).is_none());
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
    fn content_length_rejects_invalid_values() {
        let error =
            parse_content_length("POST / HTTP/1.1\r\nContent-Length: nope\r\n").unwrap_err();
        assert!(matches!(
            error,
            RequestReadError::BadRequest("invalid content-length")
        ));
    }

    #[test]
    fn token_compare_matches_exact_value_only() {
        let config = RegistryConfig {
            enrollment_token: "secret".to_string(),
            ..RegistryConfig::default()
        };

        assert!(token_is_valid(&config, "secret"));
        assert!(!token_is_valid(&config, "Secret"));
        assert!(!token_is_valid(&config, "secret-extra"));
        assert!(!token_is_valid(&config, ""));
    }

    #[test]
    fn poisoned_store_lock_is_recovered() {
        let store = Mutex::new(DeviceStore::default());
        let _ = std::panic::catch_unwind(|| {
            let _guard = store.lock().unwrap();
            panic!("poison registry store mutex");
        });

        let guard = lock_device_store(&store);

        assert!(guard.devices.is_empty());
    }

    #[test]
    fn store_save_round_trips_with_atomic_write_path() {
        let path = std::env::temp_dir().join(format!(
            "telemon-registry-store-test-{}.json",
            Uuid::new_v4()
        ));
        let mut store = DeviceStore::default();
        store.register(register_request(), "203.0.113.76".to_string(), 100);

        store.save(&path).unwrap();
        let loaded = DeviceStore::load(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.devices.len(), 1);
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
