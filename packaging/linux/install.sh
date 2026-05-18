#!/usr/bin/env bash
set -euo pipefail

TARGET_BINARY="/usr/local/bin/telemon-exporter"
CONFIG_DIR="/etc/telemon"
CONFIG_FILE="$CONFIG_DIR/exporter.yml"
PROMETHEUS_IP_FILE="$CONFIG_DIR/prometheus-server-ip"
STATE_DIR="/var/lib/telemon/exporter"
UNIT_FILE="/etc/systemd/system/telemon-exporter.service"
PROMETHEUS_SERVER_IP=""
REGISTRY_SERVER=""
ENROLLMENT_TOKEN=""
USER_NAME=""
DEVICE_NAME="${HOSTNAME:-}"
ADVERTISED_ADDR="${TELEMON_ADVERTISED_ADDR:-}"
SOURCE_BINARY="target/release/telemon-exporter"

usage() {
  cat <<'USAGE'
Usage: sudo bash packaging/linux/install.sh [options] [binary-path]

Options:
  --registry-server HOST:PORT      Registry server used for UUID enrollment.
  --enrollment-token TOKEN         Shared registry enrollment token.
  --user-name NAME                 Human user label for this device.
  --device-name NAME               Human device label; defaults to hostname.
  --advertised-addr HOST_OR_IP     Scrape host/IP published to Prometheus SD.
  --prometheus-server-ip IP        Source allowed to scrape TCP 9185.

Installs telemon-exporter as a systemd service. When --prometheus-server-ip
is provided and UFW is installed, the installer adds a narrow inbound allow rule:
  ufw allow from IP to any port 9185 proto tcp comment 'telemon prometheus scrape'
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
  echo "or pass a binary path: sudo bash packaging/linux/install.sh ./target/debug/telemon-exporter" >&2
  exit 1
fi

configure_ufw_for_prometheus() {
  local prometheus_ip="$1"

  if [ -z "$prometheus_ip" ]; then
    return
  fi

  printf '%s\n' "$prometheus_ip" > "$PROMETHEUS_IP_FILE"
  chmod 0644 "$PROMETHEUS_IP_FILE"

  if ! command -v ufw >/dev/null 2>&1; then
    echo "UFW not found; allow Prometheus manually: TCP 9185 from $prometheus_ip" >&2
    return
  fi

  ufw allow from "$prometheus_ip" to any port 9185 proto tcp comment 'telemon prometheus scrape'
  if ufw status 2>/dev/null | grep -q '^Status: active'; then
    ufw reload
  else
    echo "UFW rule added, but UFW is inactive; not enabling UFW automatically" >&2
  fi
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
}

configure_registration_config() {
  if [ -z "$REGISTRY_SERVER" ]; then
    return
  fi

  update_config_value "identity" "user_name" "$USER_NAME"
  update_config_value "identity" "device_name" "$DEVICE_NAME"
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

if ! getent group telemon >/dev/null 2>&1; then
  groupadd --system telemon
fi

if ! id -u telemon >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin --gid telemon telemon
fi

prompt_registration_config

install -m 0755 "$SOURCE_BINARY" "$TARGET_BINARY"
install -d -m 0755 "$CONFIG_DIR"
install -d -m 0750 -o telemon -g telemon "$STATE_DIR"

if [ ! -f "$CONFIG_FILE" ]; then
  if [ -f "config.example.yml" ]; then
    install -m 0644 "config.example.yml" "$CONFIG_FILE"
    sed -i 's/listen: "127.0.0.1:9185"/listen: "0.0.0.0:9185"/' "$CONFIG_FILE"
  else
    cat > "$CONFIG_FILE" <<'CONFIG'
server:
  listen: "0.0.0.0:9185"
  metrics_path: "/metrics"

collection:
  scrape_cache_stale_after_seconds: 60
  fake_interval_seconds: 5
  temperature_interval_seconds: 15
  sensor_rescan_interval_seconds: 300
  gpu_interval_seconds: 15

identity:
  user_name: ""
  device_name: ""

registration:
  enabled: false
  registry_addr: ""
  enrollment_token: ""
  device_id_file: ""
  heartbeat_interval_seconds: 30
  scrape_port: 9185
  advertised_addr: ""

collectors:
  fake:
    enabled: true
  linux_hwmon:
    enabled: true
    root: "/sys/class/hwmon"
    include_unknown_sensors: false
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
fi

sed 's|/usr/bin/telemon-exporter|/usr/local/bin/telemon-exporter|' \
  "packaging/linux/telemon-exporter.service" > "$UNIT_FILE"
chmod 0644 "$UNIT_FILE"
ensure_registration_config_shape
configure_registration_config
configure_ufw_for_prometheus "$PROMETHEUS_SERVER_IP"
systemctl daemon-reload
systemctl enable --now telemon-exporter.service

echo "telemon-exporter installed"
