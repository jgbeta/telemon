#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="telemon-exporter"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ARTIFACT=""
SOURCE_BINARY=""
RELEASE_URL=""
TMP_WORK_DIR=""
DRY_RUN="false"

CONFIG_DIR="/etc/telemon"
CONFIG_FILE="$CONFIG_DIR/exporter.yml"
STATE_DIR="/var/lib/telemon/exporter"
TARGET_BINARY="/usr/local/bin/telemon-exporter"
PERSISTENT_BINARY=""
PROMETHEUS_IP_FILE="$CONFIG_DIR/prometheus-server-ip"
PROMETHEUS_FIREWALL_FILE="$CONFIG_DIR/prometheus-firewall-rule"
UNIT_FILE="/etc/systemd/system/${SERVICE_NAME}.service"

UNRAID_DIR="/boot/config/plugins/telemon"
RUN_SCRIPT=""
RUN_SCRIPT_ALIAS=""
FORCE_CONFIG="false"

PROMETHEUS_SERVER_IP="${TELEMON_PROMETHEUS_IP:-}"
REGISTRY_SERVER="${TELEMON_REGISTRY_SERVER:-}"
ENROLLMENT_TOKEN="${TELEMON_ENROLLMENT_TOKEN:-}"
USER_NAME="${TELEMON_USER_NAME:-}"
DEVICE_NAME="${TELEMON_DEVICE_NAME:-}"
MACHINE_UUID="${TELEMON_MACHINE_UUID:-}"
ADVERTISED_ADDR="${TELEMON_ADVERTISED_ADDR:-}"

log() {
  printf '[telemon] %s\n' "$*"
}

warn() {
  printf '[telemon] warning: %s\n' "$*" >&2
}

die() {
  printf '[telemon] error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "$TMP_WORK_DIR" ] && [ -d "$TMP_WORK_DIR" ]; then
    rm -rf "$TMP_WORK_DIR"
  fi
}

trap cleanup EXIT

usage() {
  cat <<'USAGE'
Usage: sudo bash install.sh [options] [artifact-or-binary-path]

Compatibility/bootstrap installer for quick Linux and homelab installs.
This is not the long-term package format; prefer the .deb on Debian/Ubuntu
when possible.

Options:
  --artifact PATH                 Prebuilt tar.gz bundle or raw binary.
  --release-url URL               Download a prebuilt release artifact.
  --registry-server HOST:PORT      Registry server used for UUID enrollment.
  --enrollment-token TOKEN         Shared registry enrollment token.
  --user-name NAME                 Human user label for this device.
  --device-name NAME               Human device label; defaults to hostname.
  --machine-uuid UUID              Physical machine UUID shared by multi-OS installs.
  --advertised-addr HOST_OR_IP     Scrape host/IP published to Prometheus SD.
  --prometheus-server-ip IP        Source allowed to scrape the exporter TCP port.
  --force-config                   Replace existing exporter.yml with defaults.
  --dry-run                        Resolve inputs and print planned paths without installing.
  --help, -h                       Show this help.

The script installs a prebuilt artifact. Targets should not need Cargo.
Use the versioned tar.gz from dist/current or a GitHub Release, for example:
  sudo bash install.sh --artifact dist/current/telemon-exporter-v0.1.0-linux-x86_64.tar.gz
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --artifact)
      [ "$#" -ge 2 ] || die "--artifact requires a path"
      SOURCE_ARTIFACT="$2"
      shift 2
      ;;
    --release-url)
      [ "$#" -ge 2 ] || die "--release-url requires a URL"
      RELEASE_URL="$2"
      shift 2
      ;;
    --prometheus-server-ip)
      [ "$#" -ge 2 ] || die "--prometheus-server-ip requires an IP address"
      PROMETHEUS_SERVER_IP="$2"
      shift 2
      ;;
    --registry-server)
      [ "$#" -ge 2 ] || die "--registry-server requires HOST:PORT"
      REGISTRY_SERVER="$2"
      shift 2
      ;;
    --enrollment-token)
      [ "$#" -ge 2 ] || die "--enrollment-token requires a token"
      ENROLLMENT_TOKEN="$2"
      shift 2
      ;;
    --user-name)
      [ "$#" -ge 2 ] || die "--user-name requires a value"
      USER_NAME="$2"
      shift 2
      ;;
    --device-name)
      [ "$#" -ge 2 ] || die "--device-name requires a value"
      DEVICE_NAME="$2"
      shift 2
      ;;
    --machine-uuid)
      [ "$#" -ge 2 ] || die "--machine-uuid requires a UUID"
      MACHINE_UUID="$2"
      shift 2
      ;;
    --advertised-addr)
      [ "$#" -ge 2 ] || die "--advertised-addr requires a host or IP"
      ADVERTISED_ADDR="$2"
      shift 2
      ;;
    --force-config)
      FORCE_CONFIG="true"
      shift
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    -*)
      usage >&2
      die "unknown option: $1"
      ;;
    *)
      SOURCE_ARTIFACT="$1"
      shift
      [ "$#" -eq 0 ] || die "unexpected extra argument: $1"
      ;;
  esac
done

if [ -n "$SOURCE_ARTIFACT" ] && [ -n "$RELEASE_URL" ]; then
  die "use either --artifact/path or --release-url, not both"
fi

require_root() {
  if [ "$(id -u)" -ne 0 ]; then
    die "install.sh must be run as root"
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

default_user_name() {
  if [ -n "${SUDO_USER:-}" ] && [ "${SUDO_USER:-}" != "root" ]; then
    printf '%s\n' "$SUDO_USER"
  else
    id -un 2>/dev/null || printf 'root\n'
  fi
}

default_device_name() {
  local value
  value="$(hostname 2>/dev/null || true)"
  if [ -n "$value" ]; then
    printf '%s\n' "$value"
  else
    printf 'unknown-device\n'
  fi
}

is_unraid_like() {
  [ -f "/etc/unraid-version" ] || [ -d "/boot/config" ]
}

systemd_available() {
  command -v systemctl >/dev/null 2>&1 && [ -d "/run/systemd/system" ]
}

detect_environment() {
  if is_unraid_like && [ -d "/boot/config" ]; then
    CONFIG_DIR="$UNRAID_DIR"
    CONFIG_FILE="$CONFIG_DIR/exporter.yml"
    STATE_DIR="$CONFIG_DIR/state"
    TARGET_BINARY="/usr/local/bin/telemon-exporter"
    PERSISTENT_BINARY="$CONFIG_DIR/telemon-exporter"
    PROMETHEUS_IP_FILE="$CONFIG_DIR/prometheus-server-ip"
    PROMETHEUS_FIREWALL_FILE="$CONFIG_DIR/prometheus-firewall-rule"
    RUN_SCRIPT="$CONFIG_DIR/run-telemon-exporter.sh"
    RUN_SCRIPT_ALIAS="$CONFIG_DIR/exporter.sh"
    log "detected Unraid-like environment; using persistent state at $CONFIG_DIR and runtime binary at $TARGET_BINARY"
    return 0
  fi

  if [ -r "/etc/os-release" ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    log "detected Linux distribution: ${PRETTY_NAME:-${ID:-unknown}}"
    case ",${ID:-},${ID_LIKE:-}," in
      *,debian,*|*,ubuntu,*)
        log "Debian/Ubuntu-like system detected; the .deb remains the preferred package path"
        ;;
    esac
  else
    log "detected Linux distribution: unknown"
  fi
}

prompt_registration_config() {
  local default_user default_device
  default_user="$(default_user_name)"
  default_device="$(default_device_name)"
  default_device="${default_device:-unknown-device}"

  if [ -z "$REGISTRY_SERVER" ] && [ -t 0 ]; then
    read -r -p "Registry server HOST:PORT (blank to disable registration): " REGISTRY_SERVER
  fi

  if [ -z "$REGISTRY_SERVER" ]; then
    log "registration disabled; no registry server provided"
    return 0
  fi

  if [ -z "$ENROLLMENT_TOKEN" ] && [ -t 0 ]; then
    read -r -s -p "Enrollment token: " ENROLLMENT_TOKEN
    printf '\n'
  fi
  [ -n "$ENROLLMENT_TOKEN" ] || die "registry server requires an enrollment token"

  if [ -z "$USER_NAME" ] && [ -t 0 ]; then
    read -r -p "User name label [$default_user]: " USER_NAME
  fi
  USER_NAME="${USER_NAME:-$default_user}"

  if [ -z "$DEVICE_NAME" ] && [ -t 0 ]; then
    read -r -p "Device name label [$default_device]: " DEVICE_NAME
  fi
  DEVICE_NAME="${DEVICE_NAME:-$default_device}"

  if [ -z "$ADVERTISED_ADDR" ] && [ -t 0 ]; then
    read -r -p "Advertised scrape host/IP (blank to let registry observe): " ADVERTISED_ADDR
  fi
  ADVERTISED_ADDR="${ADVERTISED_ADDR:-}"

  if [ -z "$MACHINE_UUID" ] && [ -t 0 ]; then
    read -r -p "Machine UUID (blank for auto-generated local machine UUID): " MACHINE_UUID
  fi
  MACHINE_UUID="${MACHINE_UUID:-}"
}

sed_escape() {
  printf '%s' "$1" | sed 's/[\/&|\\]/\\&/g'
}

update_config_value() {
  local section="$1"
  local key="$2"
  local value
  local mode="${4:-string}"
  local temp_file
  value="$(sed_escape "$3")"
  temp_file="$(mktemp)"
  awk -v section="$section" -v key="$key" -v value="$value" -v mode="$mode" '
    /^[^[:space:]][^:]*:/ {
      current = $1
      sub(":", "", current)
    }
    current == section && $1 == key ":" {
      if (mode == "raw") {
        print "  " key ": " value
      } else {
        print "  " key ": \"" value "\""
      }
      next
    }
    { print }
  ' "$CONFIG_FILE" > "$temp_file"
  install -m 0644 "$temp_file" "$CONFIG_FILE"
  rm -f "$temp_file"
}

ensure_config_key() {
  local section="$1"
  local key="$2"
  local line="$3"
  local temp_file

  if awk -v section="$section" -v key="$key" '
    /^[^[:space:]][^:]*:/ {
      current = $1
      sub(":", "", current)
    }
    current == section && $1 == key ":" {
      found = 1
    }
    END {
      exit found ? 0 : 1
    }
  ' "$CONFIG_FILE"; then
    return 0
  fi

  temp_file="$(mktemp)"
  awk -v section="$section" -v line="$line" '
    /^[^[:space:]][^:]*:/ {
      if (in_section && !inserted) {
        print line
        inserted = 1
      }
      current = $1
      sub(":", "", current)
      in_section = (current == section)
    }
    { print }
    END {
      if (in_section && !inserted) {
        print line
      }
    }
  ' "$CONFIG_FILE" > "$temp_file"
  install -m 0644 "$temp_file" "$CONFIG_FILE"
  rm -f "$temp_file"
}

ensure_registration_config_shape() {
  if ! grep -q '^identity:' "$CONFIG_FILE"; then
    cat >> "$CONFIG_FILE" <<'CONFIG'

identity:
  user_name: ""
  device_name: ""
  machine_uuid: ""
  machine_uuid_file: ""
CONFIG
  fi

  if ! grep -q '^registration:' "$CONFIG_FILE"; then
    cat >> "$CONFIG_FILE" <<'CONFIG'

registration:
  enabled: false
  registry_addr: ""
  enrollment_token: ""
  device_id_file: ""
  heartbeat_interval_seconds: 30
  scrape_port: 9185
  advertised_addr: ""
CONFIG
  fi

  ensure_config_key "registration" "advertised_addr" '  advertised_addr: ""'
  ensure_config_key "identity" "machine_uuid" '  machine_uuid: ""'
  ensure_config_key "identity" "machine_uuid_file" '  machine_uuid_file: ""'
}

write_fallback_config() {
  cat > "$CONFIG_FILE" <<'CONFIG'
server:
  listen: "0.0.0.0:9185"
  metrics_path: "/metrics"
  static_metrics_path: "/metrics/static"

identity:
  user_name: ""
  device_name: ""
  machine_uuid: ""
  machine_uuid_file: ""

registration:
  enabled: false
  registry_addr: ""
  enrollment_token: ""
  device_id_file: ""
  heartbeat_interval_seconds: 30
  scrape_port: 9185
  advertised_addr: ""

collection:
  scrape_cache_stale_after_seconds: 60
  temperature_interval_seconds: 15
  sensor_rescan_interval_seconds: 300
  gpu_interval_seconds: 15
  static_info_interval_seconds: 300

adaptive_sampling:
  enabled: true
  levels:
    normal_seconds: 15
    warm_seconds: 10
    hot_seconds: 5
    critical_seconds: 1
  temperature:
    enabled: true
    warm_celsius: 60
    hot_celsius: 75
    critical_celsius: 85
  cooldown_seconds: 60

collectors:
  linux_hwmon:
    enabled: true
    root: "/sys/class/hwmon"
    include_unknown_sensors: false
    nvme_enrichment_enabled: true
    expose_storage_model: true
    sensor_allowlist: []
    sensor_denylist: []
  nvidia_nvml:
    enabled: true
    library_paths: []
    expose_gpu_name: true
    expose_gpu_uuid: false
    fan_speed_enabled: true

logging:
  level: "info"
CONFIG
}

install_config() {
  install -d -m 0755 "$CONFIG_DIR"

  if [ -f "$CONFIG_FILE" ] && [ "$FORCE_CONFIG" != "true" ]; then
    log "preserving existing config $CONFIG_FILE"
  else
    if [ -f "$CONFIG_FILE" ]; then
      local backup
      backup="${CONFIG_FILE}.bak.$(date -u +%Y%m%dT%H%M%SZ)"
      cp -p "$CONFIG_FILE" "$backup"
      log "backed up existing config to $backup"
    fi

    if [ -f "$SCRIPT_DIR/config.example.yml" ]; then
      install -m 0644 "$SCRIPT_DIR/config.example.yml" "$CONFIG_FILE"
    else
      write_fallback_config
      chmod 0644 "$CONFIG_FILE"
    fi
    log "installed config $CONFIG_FILE"
  fi

  ensure_registration_config_shape
}

configure_registration_config() {
  if [ -z "$REGISTRY_SERVER" ]; then
    return 0
  fi

  update_config_value "identity" "user_name" "$USER_NAME"
  update_config_value "identity" "device_name" "$DEVICE_NAME"
  if [ -n "$MACHINE_UUID" ]; then
    update_config_value "identity" "machine_uuid" "$MACHINE_UUID"
  fi
  update_config_value "registration" "enabled" "true" "raw"
  update_config_value "registration" "registry_addr" "$REGISTRY_SERVER"
  update_config_value "registration" "enrollment_token" "$ENROLLMENT_TOKEN"
  update_config_value "registration" "device_id_file" "$STATE_DIR/device-id"
  if [ -n "$ADVERTISED_ADDR" ]; then
    update_config_value "registration" "advertised_addr" "$ADVERTISED_ADDR"
  fi

  if [ -z "$PROMETHEUS_SERVER_IP" ]; then
    PROMETHEUS_SERVER_IP="${REGISTRY_SERVER%%:*}"
  fi

  log "configured registry enrollment for $REGISTRY_SERVER"
}

ensure_tmp_work_dir() {
  if [ -z "$TMP_WORK_DIR" ]; then
    TMP_WORK_DIR="$(mktemp -d)"
  fi
}

download_release_artifact() {
  local output
  ensure_tmp_work_dir
  case "$RELEASE_URL" in
    *.tar.gz|*.tgz)
      output="$TMP_WORK_DIR/release-artifact.tar.gz"
      ;;
    *)
      output="$TMP_WORK_DIR/release-artifact"
      ;;
  esac

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$RELEASE_URL" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$output" "$RELEASE_URL"
  else
    die "downloading --release-url requires curl or wget"
  fi
  printf '%s\n' "$output"
}

find_default_artifact() {
  local candidate
  [ -z "$SOURCE_ARTIFACT" ] || return 0
  [ -z "$RELEASE_URL" ] || return 0

  if [ -f "$SCRIPT_DIR/telemon-exporter" ]; then
    SOURCE_ARTIFACT="$SCRIPT_DIR/telemon-exporter"
    log "using bundled binary $SOURCE_ARTIFACT"
    return 0
  fi

  [ -d "$SCRIPT_DIR/dist/current" ] || return 0

  set -- "$SCRIPT_DIR"/dist/current/telemon-exporter-v*-linux-*.tar.gz
  if [ "$#" -eq 1 ] && [ -f "$1" ]; then
    candidate="$1"
    SOURCE_ARTIFACT="$candidate"
    log "using local release artifact $SOURCE_ARTIFACT"
  fi
}

extract_artifact_binary() {
  local artifact="$1"
  local extract_dir candidate

  require_command find
  require_command tar
  ensure_tmp_work_dir
  extract_dir="$TMP_WORK_DIR/artifact"
  install -d -m 0755 "$extract_dir"
  tar -xzf "$artifact" -C "$extract_dir"
  candidate="$(find "$extract_dir" -type f -name "telemon-exporter" -print -quit)"
  [ -n "$candidate" ] || die "artifact does not contain telemon-exporter"
  printf '%s\n' "$candidate"
}

resolve_source_binary() {
  local input
  find_default_artifact

  if [ -n "$RELEASE_URL" ]; then
    input="$(download_release_artifact)"
  else
    input="$SOURCE_ARTIFACT"
  fi

  [ -n "$input" ] || die "provide a prebuilt artifact or binary path; target installs should not require Cargo"
  [ -f "$input" ] || die "artifact or binary does not exist: $input"

  case "$input" in
    *.tar.gz|*.tgz)
      SOURCE_BINARY="$(extract_artifact_binary "$input")"
      ;;
    *)
      SOURCE_BINARY="$input"
      ;;
  esac

  [ -f "$SOURCE_BINARY" ] || die "resolved binary does not exist: $SOURCE_BINARY"
}

install_binary() {
  resolve_source_binary

  if [ -n "$PERSISTENT_BINARY" ]; then
    install -d -m 0755 "$(dirname "$PERSISTENT_BINARY")"
    install -m 0644 "$SOURCE_BINARY" "$PERSISTENT_BINARY"
    install -d -m 0755 "$(dirname "$TARGET_BINARY")"
    install -m 0755 "$PERSISTENT_BINARY" "$TARGET_BINARY"
    log "stored persistent binary $PERSISTENT_BINARY"
  else
    install -d -m 0755 "$(dirname "$TARGET_BINARY")"
    install -m 0755 "$SOURCE_BINARY" "$TARGET_BINARY"
  fi

  [ -x "$TARGET_BINARY" ] || die "installed binary is not executable: $TARGET_BINARY"
  log "installed runtime binary $TARGET_BINARY"
}

ensure_service_account() {
  if ! command -v getent >/dev/null 2>&1 || ! command -v groupadd >/dev/null 2>&1 || ! command -v useradd >/dev/null 2>&1; then
    warn "cannot create service account; systemd service will run as root"
    return 1
  fi

  if ! getent group telemon >/dev/null 2>&1; then
    groupadd --system telemon
  fi
  if ! id -u telemon >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin --gid telemon telemon
  fi
  return 0
}

install_state_dir() {
  if id -u telemon >/dev/null 2>&1; then
    install -d -m 0750 -o telemon -g telemon "$STATE_DIR"
  else
    install -d -m 0750 "$STATE_DIR"
  fi
}

install_systemd_service() {
  local temp_unit
  require_command systemctl

  if ensure_service_account; then
    install_state_dir
    temp_unit="$(mktemp)"
    if [ -f "$SCRIPT_DIR/packaging/linux/telemon-exporter.service" ]; then
      sed "s|/usr/bin/telemon-exporter|$TARGET_BINARY|" \
        "$SCRIPT_DIR/packaging/linux/telemon-exporter.service" > "$temp_unit"
    else
      cat > "$temp_unit" <<UNIT
[Unit]
Description=Telemon Exporter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=telemon
Group=telemon
ExecStart=$TARGET_BINARY run --config $CONFIG_FILE
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
    fi
  else
    install_state_dir
    temp_unit="$(mktemp)"
    cat > "$temp_unit" <<UNIT
[Unit]
Description=Telemon Exporter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$TARGET_BINARY run --config $CONFIG_FILE
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
  fi

  install -m 0644 "$temp_unit" "$UNIT_FILE"
  rm -f "$temp_unit"

  systemctl daemon-reload
  systemctl enable "$SERVICE_NAME.service"
  systemctl restart "$SERVICE_NAME.service"
  log "installed and restarted systemd service $SERVICE_NAME"
}

write_unraid_runner() {
  [ -n "$RUN_SCRIPT" ] || return 0
  [ -n "$PERSISTENT_BINARY" ] || die "internal error: missing persistent binary path for Unraid runner"

  install -d -m 0755 "$(dirname "$RUN_SCRIPT")"

  cat > "$RUN_SCRIPT" <<RUNNER
#!/usr/bin/env bash
set -euo pipefail
install -d -m 0755 "$(dirname "$TARGET_BINARY")"
install -m 0755 "$PERSISTENT_BINARY" "$TARGET_BINARY"
exec "$TARGET_BINARY" run --config "$CONFIG_FILE"
RUNNER
  chmod 0755 "$RUN_SCRIPT"
  [ -x "$RUN_SCRIPT" ] || die "installed Unraid runner is not executable: $RUN_SCRIPT"
  log "installed Unraid runner $RUN_SCRIPT"

  if [ -n "$RUN_SCRIPT_ALIAS" ]; then
    cp "$RUN_SCRIPT" "$RUN_SCRIPT_ALIAS"
    chmod 0755 "$RUN_SCRIPT_ALIAS"
    [ -x "$RUN_SCRIPT_ALIAS" ] || die "installed Unraid runner alias is not executable: $RUN_SCRIPT_ALIAS"
    log "installed Unraid runner alias $RUN_SCRIPT_ALIAS"
  fi
}

firewall_scrape_port() {
  local port
  port="$(awk '
    /^[^[:space:]][^:]*:/ {
      current = $1
      sub(":", "", current)
    }
    current == "registration" && $1 == "scrape_port:" {
      print $2
    }
  ' "$CONFIG_FILE" 2>/dev/null | tail -n 1)"

  case "$port" in
    ''|*[!0-9]*|0)
      printf '9185\n'
      ;;
    *)
      printf '%s\n' "$port"
      ;;
  esac
}

write_firewall_state() {
  local prometheus_ip="$1"
  local scrape_port="$2"
  local managed="$3"

  install -d -m 0755 "$CONFIG_DIR"
  printf '%s\n' "$prometheus_ip" > "$PROMETHEUS_IP_FILE"
  chmod 0644 "$PROMETHEUS_IP_FILE"
  cat > "$PROMETHEUS_FIREWALL_FILE" <<STATE
backend=ufw
source_ip=$prometheus_ip
port=$scrape_port
managed=$managed
STATE
  chmod 0644 "$PROMETHEUS_FIREWALL_FILE"
}

firewall_value() {
  local key="$1"
  awk -F= -v key="$key" '$1 == key { print substr($0, length(key) + 2) }' "$PROMETHEUS_FIREWALL_FILE" 2>/dev/null | tail -n 1
}

previous_firewall_rule_was_managed() {
  local prometheus_ip="$1"
  local scrape_port="$2"

  [ -f "$PROMETHEUS_FIREWALL_FILE" ] || return 1
  [ "$(firewall_value source_ip)" = "$prometheus_ip" ] || return 1
  [ "$(firewall_value port)" = "$scrape_port" ] || return 1
  [ "$(firewall_value managed)" = "true" ] || return 1
}

configure_ufw_for_prometheus() {
  local prometheus_ip="$1"
  local scrape_port output managed
  [ -n "$prometheus_ip" ] || return 0

  scrape_port="$(firewall_scrape_port)"

  if ! command -v ufw >/dev/null 2>&1; then
    warn "UFW not found; allow Prometheus manually: TCP $scrape_port from $prometheus_ip"
    return 0
  fi

  output="$(ufw allow from "$prometheus_ip" to any port "$scrape_port" proto tcp comment 'telemon prometheus scrape' 2>&1)"
  printf '%s\n' "$output"
  managed="true"
  case "$output" in
    *'Skipping adding existing rule'*)
      if previous_firewall_rule_was_managed "$prometheus_ip" "$scrape_port"; then
        managed="true"
      else
        managed="false"
      fi
      ;;
  esac
  write_firewall_state "$prometheus_ip" "$scrape_port" "$managed"

  if ufw status 2>/dev/null | grep -q '^Status: active'; then
    ufw reload
  else
    warn "UFW rule added, but UFW is inactive; not enabling UFW automatically"
  fi
}

print_manual_steps() {
  local command_text

  if [ -n "$RUN_SCRIPT" ]; then
    command_text="nohup $RUN_SCRIPT >> /var/log/telemon-exporter.log 2>&1 &"
    cat <<STEPS

[telemon] systemd was not used. Run the exporter now with:
  $command_text

For Unraid boot persistence, add this line to /boot/config/go:
  $command_text

The installer does not edit /boot/config/go automatically.
STEPS
  else
    command_text="nohup $TARGET_BINARY run --config $CONFIG_FILE >> /var/log/telemon-exporter.log 2>&1 &"
    cat <<STEPS

[telemon] systemd is not available. Run the exporter manually with:
  $command_text
STEPS
  fi

  cat <<STEPS

Health checks:
  curl http://127.0.0.1:9185/healthz
  curl http://127.0.0.1:9185/metrics

Installed paths:
  Binary: $TARGET_BINARY
  Config: $CONFIG_FILE
  State:  $STATE_DIR
STEPS

  if [ -n "$PERSISTENT_BINARY" ]; then
    cat <<STEPS
  Persistent binary copy: $PERSISTENT_BINARY
STEPS
  fi

  if [ -n "$RUN_SCRIPT_ALIAS" ]; then
    cat <<STEPS
  Runner alias: $RUN_SCRIPT_ALIAS
STEPS
  fi
}

main() {
  require_command awk
  require_command chmod
  require_command cp
  require_command date
  require_command dirname
  require_command grep
  require_command id
  require_command install
  require_command mktemp
  require_command rm
  require_command sed

  log "compatibility/bootstrap installer; prefer the .deb on Debian/Ubuntu when possible"
  detect_environment

  if [ "$DRY_RUN" = "true" ]; then
    log "dry run enabled; no files will be installed"
    prompt_registration_config
    resolve_source_binary
    cat <<STEPS

[telemon] dry-run resolved inputs:
  Source binary: $SOURCE_BINARY
  Runtime binary: $TARGET_BINARY
  Config: $CONFIG_FILE
  State:  $STATE_DIR
  Registry server: ${REGISTRY_SERVER:-<disabled>}
  Device name: ${DEVICE_NAME:-<default>}
  Machine UUID: ${MACHINE_UUID:-<auto-generated>}
STEPS
    if [ -n "$PERSISTENT_BINARY" ]; then
      cat <<STEPS
  Persistent binary copy: $PERSISTENT_BINARY
STEPS
    fi
    if [ -n "$RUN_SCRIPT" ]; then
      cat <<STEPS
  Runner: $RUN_SCRIPT
STEPS
    elif systemd_available; then
      cat <<STEPS
  Service: $UNIT_FILE
STEPS
    fi
    return 0
  fi

  require_root
  prompt_registration_config
  install_binary
  install_config
  install_state_dir
  configure_registration_config
  if [ -n "$RUN_SCRIPT" ]; then
    write_unraid_runner
  fi
  configure_ufw_for_prometheus "$PROMETHEUS_SERVER_IP"

  if [ -n "$RUN_SCRIPT" ]; then
    print_manual_steps
  elif systemd_available; then
    install_systemd_service
    cat <<STEPS

[telemon] installed paths:
  Binary: $TARGET_BINARY
  Config: $CONFIG_FILE
  State:  $STATE_DIR

Useful commands:
  systemctl status $SERVICE_NAME
  journalctl -u $SERVICE_NAME -n 100
  curl http://127.0.0.1:9185/healthz
  curl http://127.0.0.1:9185/metrics
STEPS
  else
    print_manual_steps
  fi
}

main "$@"
