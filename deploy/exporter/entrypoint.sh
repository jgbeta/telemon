#!/bin/sh
set -eu

CONFIG_DIR="${TELEMON_CONFIG_DIR:-/config}"
STATE_DIR="${TELEMON_STATE_DIR:-$CONFIG_DIR/state}"
CONFIG_FILE="${TELEMON_CONFIG_FILE:-$CONFIG_DIR/generated-exporter.yml}"

LISTEN="${TELEMON_LISTEN:-0.0.0.0:9185}"
METRICS_PATH="${TELEMON_METRICS_PATH:-/metrics}"
STATIC_METRICS_PATH="${TELEMON_STATIC_METRICS_PATH:-/metrics/static}"
USER_NAME="${TELEMON_USER_NAME:-unknown-user}"
DEVICE_NAME="${TELEMON_DEVICE_NAME:-${HOSTNAME:-unknown-device}}"
MACHINE_UUID="${TELEMON_MACHINE_UUID:-}"
MACHINE_UUID_FILE="${TELEMON_MACHINE_UUID_FILE:-}"
REGISTRY_SERVER="${TELEMON_REGISTRY_SERVER:-}"
ENROLLMENT_TOKEN="${TELEMON_ENROLLMENT_TOKEN:-}"
ADVERTISED_ADDR="${TELEMON_ADVERTISED_ADDR:-}"
HEARTBEAT_INTERVAL_SECONDS="${TELEMON_HEARTBEAT_INTERVAL_SECONDS:-30}"
SCRAPE_PORT="${TELEMON_SCRAPE_PORT:-9185}"
HWMON_ROOT="${TELEMON_HWMON_ROOT:-/host/sys/class/hwmon}"
FAKE_ENABLED="${TELEMON_FAKE_ENABLED:-true}"
LINUX_HWMON_ENABLED="${TELEMON_LINUX_HWMON_ENABLED:-true}"
LINUX_HWMON_INCLUDE_UNKNOWN="${TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN:-false}"
NVIDIA_NVML_ENABLED="${TELEMON_NVIDIA_NVML_ENABLED:-true}"
NVIDIA_EXPOSE_GPU_NAME="${TELEMON_NVIDIA_EXPOSE_GPU_NAME:-true}"
NVIDIA_EXPOSE_GPU_UUID="${TELEMON_NVIDIA_EXPOSE_GPU_UUID:-false}"
NVIDIA_FAN_SPEED_ENABLED="${TELEMON_NVIDIA_FAN_SPEED_ENABLED:-true}"
ADAPTIVE_ENABLED="${TELEMON_ADAPTIVE_ENABLED:-true}"
ADAPTIVE_NORMAL_SECONDS="${TELEMON_ADAPTIVE_NORMAL_SECONDS:-15}"
ADAPTIVE_WARM_SECONDS="${TELEMON_ADAPTIVE_WARM_SECONDS:-10}"
ADAPTIVE_HOT_SECONDS="${TELEMON_ADAPTIVE_HOT_SECONDS:-5}"
ADAPTIVE_CRITICAL_SECONDS="${TELEMON_ADAPTIVE_CRITICAL_SECONDS:-1}"
ADAPTIVE_TEMPERATURE_ENABLED="${TELEMON_ADAPTIVE_TEMPERATURE_ENABLED:-true}"
ADAPTIVE_TEMPERATURE_WARM_CELSIUS="${TELEMON_ADAPTIVE_TEMPERATURE_WARM_CELSIUS:-60}"
ADAPTIVE_TEMPERATURE_HOT_CELSIUS="${TELEMON_ADAPTIVE_TEMPERATURE_HOT_CELSIUS:-75}"
ADAPTIVE_TEMPERATURE_CRITICAL_CELSIUS="${TELEMON_ADAPTIVE_TEMPERATURE_CRITICAL_CELSIUS:-85}"
ADAPTIVE_COOLDOWN_SECONDS="${TELEMON_ADAPTIVE_COOLDOWN_SECONDS:-60}"

yaml_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

bool_value() {
  case "$1" in
    true|false)
      printf '%s\n' "$1"
      ;;
    TRUE|True|1|yes|YES|Yes|on|ON|On)
      printf 'true\n'
      ;;
    FALSE|False|0|no|NO|No|off|OFF|Off)
      printf 'false\n'
      ;;
    *)
      printf 'invalid boolean value: %s\n' "$1" >&2
      exit 1
      ;;
  esac
}

yaml_csv_list() {
  value="$1"
  if [ -z "$value" ]; then
    printf '[]\n'
    return
  fi

  old_ifs="$IFS"
  IFS=','
  set -- $value
  IFS="$old_ifs"

  printf '\n'
  for item in "$@"; do
    trimmed="$(printf '%s' "$item" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
    [ -n "$trimmed" ] || continue
    printf '    - "%s"\n' "$(yaml_escape "$trimmed")"
  done
}

if [ -n "$REGISTRY_SERVER" ] && [ -z "$ENROLLMENT_TOKEN" ]; then
  printf 'TELEMON_ENROLLMENT_TOKEN is required when TELEMON_REGISTRY_SERVER is set\n' >&2
  exit 1
fi

mkdir -p "$CONFIG_DIR" "$STATE_DIR"

if [ -n "$REGISTRY_SERVER" ]; then
  REGISTRATION_ENABLED=true
else
  REGISTRATION_ENABLED=false
fi

cat > "$CONFIG_FILE" <<CONFIG
server:
  listen: "$(yaml_escape "$LISTEN")"
  metrics_path: "$(yaml_escape "$METRICS_PATH")"
  static_metrics_path: "$(yaml_escape "$STATIC_METRICS_PATH")"

identity:
  user_name: "$(yaml_escape "$USER_NAME")"
  device_name: "$(yaml_escape "$DEVICE_NAME")"
  machine_uuid: "$(yaml_escape "$MACHINE_UUID")"
  machine_uuid_file: "$(yaml_escape "$MACHINE_UUID_FILE")"

registration:
  enabled: $REGISTRATION_ENABLED
  registry_addr: "$(yaml_escape "$REGISTRY_SERVER")"
  enrollment_token: "$(yaml_escape "$ENROLLMENT_TOKEN")"
  device_id_file: "$(yaml_escape "$STATE_DIR/device-id")"
  heartbeat_interval_seconds: $HEARTBEAT_INTERVAL_SECONDS
  scrape_port: $SCRAPE_PORT
  advertised_addr: "$(yaml_escape "$ADVERTISED_ADDR")"

collection:
  scrape_cache_stale_after_seconds: ${TELEMON_SCRAPE_CACHE_STALE_AFTER_SECONDS:-60}
  fake_interval_seconds: ${TELEMON_FAKE_INTERVAL_SECONDS:-5}
  temperature_interval_seconds: ${TELEMON_TEMPERATURE_INTERVAL_SECONDS:-15}
  sensor_rescan_interval_seconds: ${TELEMON_SENSOR_RESCAN_INTERVAL_SECONDS:-300}
  gpu_interval_seconds: ${TELEMON_GPU_INTERVAL_SECONDS:-15}
  static_info_interval_seconds: ${TELEMON_STATIC_INFO_INTERVAL_SECONDS:-300}

adaptive_sampling:
  enabled: $(bool_value "$ADAPTIVE_ENABLED")
  levels:
    normal_seconds: $ADAPTIVE_NORMAL_SECONDS
    warm_seconds: $ADAPTIVE_WARM_SECONDS
    hot_seconds: $ADAPTIVE_HOT_SECONDS
    critical_seconds: $ADAPTIVE_CRITICAL_SECONDS
  temperature:
    enabled: $(bool_value "$ADAPTIVE_TEMPERATURE_ENABLED")
    warm_celsius: $ADAPTIVE_TEMPERATURE_WARM_CELSIUS
    hot_celsius: $ADAPTIVE_TEMPERATURE_HOT_CELSIUS
    critical_celsius: $ADAPTIVE_TEMPERATURE_CRITICAL_CELSIUS
  cooldown_seconds: $ADAPTIVE_COOLDOWN_SECONDS

collectors:
  fake:
    enabled: $(bool_value "$FAKE_ENABLED")
  linux_hwmon:
    enabled: $(bool_value "$LINUX_HWMON_ENABLED")
    root: "$(yaml_escape "$HWMON_ROOT")"
    include_unknown_sensors: $(bool_value "$LINUX_HWMON_INCLUDE_UNKNOWN")
    sensor_allowlist: $(yaml_csv_list "${TELEMON_LINUX_HWMON_ALLOWLIST:-}")
    sensor_denylist: $(yaml_csv_list "${TELEMON_LINUX_HWMON_DENYLIST:-}")
  nvidia_nvml:
    enabled: $(bool_value "$NVIDIA_NVML_ENABLED")
    library_paths: $(yaml_csv_list "${TELEMON_NVIDIA_LIBRARY_PATHS:-}")
    expose_gpu_name: $(bool_value "$NVIDIA_EXPOSE_GPU_NAME")
    expose_gpu_uuid: $(bool_value "$NVIDIA_EXPOSE_GPU_UUID")
    fan_speed_enabled: $(bool_value "$NVIDIA_FAN_SPEED_ENABLED")

logging:
  level: "$(yaml_escape "${TELEMON_LOG_LEVEL:-info}")"
CONFIG

exec telemon-exporter "$@"
