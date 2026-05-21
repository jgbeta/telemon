#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="/usr/local/libexec/telemon"
CONFIG_DIR="/Library/Application Support/Telemon"
STATE_DIR="$CONFIG_DIR/state"
LOG_DIR="/Library/Logs/Telemon"
CONFIG_FILE="$CONFIG_DIR/exporter.yml"
PLIST="/Library/LaunchDaemons/com.telemon.exporter.plist"
PF_ANCHOR="/etc/pf.anchors/com.telemon.exporter"
PF_CONF="/etc/pf.conf"
PF_MARKER_BEGIN="# BEGIN telemon exporter"
PF_MARKER_END="# END telemon exporter"
PROMETHEUS_SERVER_IP=""
REGISTRY_SERVER=""
ENROLLMENT_TOKEN=""
USER_NAME=""
DEVICE_NAME="${HOSTNAME:-}"
ADVERTISED_ADDR="${TELEMON_ADVERTISED_ADDR:-}"
MACHINE_UUID="${TELEMON_MACHINE_UUID:-}"
SOURCE_BINARY="target/release/telemon-exporter"

usage() {
  cat <<'USAGE'
Usage: sudo packaging/macos/install.sh [options] [binary-path]

Options:
  --registry-server HOST:PORT      Registry server used for UUID enrollment.
  --enrollment-token TOKEN         Shared registry enrollment token.
  --user-name NAME                 Human user label for this device.
  --device-name NAME               Human device label; defaults to hostname.
  --advertised-addr HOST_OR_IP     Scrape host/IP published to Prometheus SD.
  --machine-uuid UUID              Physical machine UUID shared by multi-OS installs.
  --prometheus-server-ip IP        Source allowed to scrape TCP 9185.

Installs telemon-exporter as a LaunchDaemon. When --prometheus-server-ip
is provided, the installer adds a pf rule that allows TCP 9185 only from that IP.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --prometheus-server-ip)
      if [ "$#" -lt 2 ]; then
        echo "--prometheus-server-ip requires an IP address" >&2
        exit 1
      fi
      PROMETHEUS_SERVER_IP="$2"
      shift 2
      ;;
    --registry-server)
      if [ "$#" -lt 2 ]; then
        echo "--registry-server requires HOST:PORT" >&2
        exit 1
      fi
      REGISTRY_SERVER="$2"
      shift 2
      ;;
    --enrollment-token)
      if [ "$#" -lt 2 ]; then
        echo "--enrollment-token requires a token" >&2
        exit 1
      fi
      ENROLLMENT_TOKEN="$2"
      shift 2
      ;;
    --user-name)
      if [ "$#" -lt 2 ]; then
        echo "--user-name requires a value" >&2
        exit 1
      fi
      USER_NAME="$2"
      shift 2
      ;;
    --device-name)
      if [ "$#" -lt 2 ]; then
        echo "--device-name requires a value" >&2
        exit 1
      fi
      DEVICE_NAME="$2"
      shift 2
      ;;
    --advertised-addr)
      if [ "$#" -lt 2 ]; then
        echo "--advertised-addr requires a host or IP" >&2
        exit 1
      fi
      ADVERTISED_ADDR="$2"
      shift 2
      ;;
    --machine-uuid)
      if [ "$#" -lt 2 ]; then
        echo "--machine-uuid requires a UUID" >&2
        exit 1
      fi
      MACHINE_UUID="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    -*)
      echo "unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      SOURCE_BINARY="$1"
      shift
      if [ "$#" -gt 0 ]; then
        echo "unexpected extra argument: $1" >&2
        usage >&2
        exit 1
      fi
      ;;
  esac
done

if [ "$(id -u)" -ne 0 ]; then
  echo "install.sh must be run as root" >&2
  exit 1
fi

if [ ! -x "$SOURCE_BINARY" ]; then
  echo "missing executable $SOURCE_BINARY" >&2
  echo "build with: cargo build --release" >&2
  exit 1
fi

configure_pf_for_prometheus() {
  local prometheus_ip="$1"
  local temp_pf_conf

  if [ -z "$prometheus_ip" ]; then
    return
  fi

  cat > "$PF_ANCHOR" <<PFANCHOR
pass in proto tcp from $prometheus_ip to any port 9185
PFANCHOR
  chmod 0644 "$PF_ANCHOR"

  temp_pf_conf="$(mktemp)"
  awk -v begin="$PF_MARKER_BEGIN" -v end="$PF_MARKER_END" '
    $0 == begin { skip = 1; next }
    $0 == end { skip = 0; next }
    skip != 1 { print }
  ' "$PF_CONF" > "$temp_pf_conf"
  cat >> "$temp_pf_conf" <<PFCONF

$PF_MARKER_BEGIN
anchor "com.telemon.exporter"
load anchor "com.telemon.exporter" from "$PF_ANCHOR"
$PF_MARKER_END
PFCONF
  install -m 0644 "$temp_pf_conf" "$PF_CONF"
  rm -f "$temp_pf_conf"

  pfctl -f "$PF_CONF"
  pfctl -E >/dev/null 2>&1 || true
}

prompt_registration_config() {
  if [ -z "$REGISTRY_SERVER" ] && [ -t 0 ]; then
    read -r -p "Registry server HOST:PORT (blank to disable registration): " REGISTRY_SERVER
  fi

  if [ -z "$REGISTRY_SERVER" ]; then
    echo "registration disabled; no registry server provided" >&2
    return
  fi

  if [ -z "$ENROLLMENT_TOKEN" ] && [ -t 0 ]; then
    read -r -s -p "Enrollment token: " ENROLLMENT_TOKEN
    echo
  fi
  if [ -z "$USER_NAME" ] && [ -t 0 ]; then
    read -r -p "User name label: " USER_NAME
  fi
  if [ -z "$DEVICE_NAME" ] && [ -t 0 ]; then
    read -r -p "Device name label [$HOSTNAME]: " DEVICE_NAME
    DEVICE_NAME="${DEVICE_NAME:-${HOSTNAME:-unknown-device}}"
  fi
  if [ -z "$ADVERTISED_ADDR" ] && [ -t 0 ]; then
    read -r -p "Advertised scrape host/IP (blank to let registry observe): " ADVERTISED_ADDR
  fi
  if [ -z "$MACHINE_UUID" ] && [ -t 0 ]; then
    read -r -p "Machine UUID (blank for auto-generated local machine UUID): " MACHINE_UUID
  fi

  if [ -z "$ENROLLMENT_TOKEN" ] || [ -z "$USER_NAME" ]; then
    echo "registration disabled; registry server requires enrollment token and user name" >&2
    REGISTRY_SERVER=""
  fi
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
    return
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

  ensure_config_key "identity" "machine_uuid" '  machine_uuid: ""'
  ensure_config_key "identity" "machine_uuid_file" '  machine_uuid_file: ""'
  ensure_config_key "registration" "advertised_addr" '  advertised_addr: ""'
}

configure_registration_config() {
  if [ -z "$REGISTRY_SERVER" ]; then
    return
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
}

install -d -m 0755 "$INSTALL_DIR"
install -d -m 0755 "$CONFIG_DIR"
install -d -m 0755 "$STATE_DIR"
install -d -m 0755 "$LOG_DIR"
install -m 0755 "$SOURCE_BINARY" "$INSTALL_DIR/telemon-exporter"
prompt_registration_config

if [ ! -f "$CONFIG_FILE" ]; then
  cat > "$CONFIG_FILE" <<'CONFIG'
server:
  listen: "0.0.0.0:9185"
  metrics_path: "/metrics"

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

collectors:
  linux_hwmon:
    enabled: false
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
  chmod 0644 "$CONFIG_FILE"
fi
ensure_registration_config_shape
configure_registration_config

install -m 0644 "packaging/macos/com.telemon.exporter.plist" "$PLIST"
chown root:wheel "$PLIST"
chmod 0644 "$PLIST"

if launchctl print system/com.telemon.exporter >/dev/null 2>&1; then
  launchctl bootout system "$PLIST" || true
fi
launchctl bootstrap system "$PLIST"
launchctl enable system/com.telemon.exporter
launchctl kickstart -k system/com.telemon.exporter
configure_pf_for_prometheus "$PROMETHEUS_SERVER_IP"

echo "telemon-exporter LaunchDaemon installed"
