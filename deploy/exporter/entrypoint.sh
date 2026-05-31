#!/bin/sh
set -eu

CONFIG_DIR="${TELEMON_CONFIG_DIR:-/config}"
STATE_DIR="${TELEMON_STATE_DIR:-$CONFIG_DIR/state}"
CONFIG_FILE="${TELEMON_CONFIG_FILE:-$CONFIG_DIR/generated-exporter.yml}"

LISTEN="${TELEMON_LISTEN:-0.0.0.0:9185}"
METRICS_PATH="${TELEMON_METRICS_PATH:-/metrics}"
STATIC_METRICS_PATH="${TELEMON_STATIC_METRICS_PATH:-/metrics/static}"
FPS_METRICS_PATH="${TELEMON_FPS_METRICS_PATH:-/fps}"
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
LINUX_HWMON_ENABLED="${TELEMON_LINUX_HWMON_ENABLED:-true}"
LINUX_HWMON_INCLUDE_UNKNOWN="${TELEMON_LINUX_HWMON_INCLUDE_UNKNOWN:-false}"
LINUX_HWMON_NVME_ENRICHMENT_ENABLED="${TELEMON_LINUX_HWMON_NVME_ENRICHMENT_ENABLED:-true}"
LINUX_HWMON_EXPOSE_STORAGE_MODEL="${TELEMON_LINUX_HWMON_EXPOSE_STORAGE_MODEL:-true}"
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
  fps_metrics_path: "$(yaml_escape "$FPS_METRICS_PATH")"

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
  linux_hwmon:
    enabled: $(bool_value "$LINUX_HWMON_ENABLED")
    root: "$(yaml_escape "$HWMON_ROOT")"
    include_unknown_sensors: $(bool_value "$LINUX_HWMON_INCLUDE_UNKNOWN")
    nvme_enrichment_enabled: $(bool_value "$LINUX_HWMON_NVME_ENRICHMENT_ENABLED")
    expose_storage_model: $(bool_value "$LINUX_HWMON_EXPOSE_STORAGE_MODEL")
    sensor_allowlist: $(yaml_csv_list "${TELEMON_LINUX_HWMON_ALLOWLIST:-}")
    sensor_denylist: $(yaml_csv_list "${TELEMON_LINUX_HWMON_DENYLIST:-}")
  steam_deck_game_state:
    enabled: $(bool_value "${TELEMON_STEAM_DECK_GAME_STATE_ENABLED:-false}")
    poll_interval_seconds: ${TELEMON_STEAM_DECK_GAME_STATE_POLL_INTERVAL_SECONDS:-1}
    stop_debounce_seconds: ${TELEMON_STEAM_DECK_GAME_STATE_STOP_DEBOUNCE_SECONDS:-5}
    xprop_path: "$(yaml_escape "${TELEMON_STEAM_DECK_GAME_STATE_XPROP_PATH:-xprop}")"
    display: "$(yaml_escape "${TELEMON_STEAM_DECK_GAME_STATE_DISPLAY:-:0}")"
    auto_discover_steam_display: $(bool_value "${TELEMON_STEAM_DECK_GAME_STATE_AUTO_DISCOVER_STEAM_DISPLAY:-true}")
    desktop_fallback_enabled: $(bool_value "${TELEMON_STEAM_DECK_GAME_STATE_DESKTOP_FALLBACK_ENABLED:-true}")
    process_fallback_enabled: $(bool_value "${TELEMON_STEAM_DECK_GAME_STATE_PROCESS_FALLBACK_ENABLED:-true}")
  steam_deck_fps:
    enabled: $(bool_value "${TELEMON_STEAM_DECK_FPS_ENABLED:-false}")
    windows_seconds: [${TELEMON_STEAM_DECK_FPS_WINDOWS_SECONDS:-1, 5, 60}]
    include_appid_label: $(bool_value "${TELEMON_STEAM_DECK_FPS_INCLUDE_APPID_LABEL:-true}")
    include_game_name_label: $(bool_value "${TELEMON_STEAM_DECK_FPS_INCLUDE_GAME_NAME_LABEL:-true}")
    max_frame_time_milliseconds: ${TELEMON_STEAM_DECK_FPS_MAX_FRAME_TIME_MILLISECONDS:-1000}
    poll_interval_milliseconds: ${TELEMON_STEAM_DECK_FPS_POLL_INTERVAL_MILLISECONDS:-100}
    max_messages_per_poll: ${TELEMON_STEAM_DECK_FPS_MAX_MESSAGES_PER_POLL:-512}
    gamescope_mangoapp:
      enabled: $(bool_value "${TELEMON_STEAM_DECK_FPS_MANGOAPP_ENABLED:-false}")
      ftok_path: "$(yaml_escape "${TELEMON_STEAM_DECK_FPS_MANGOAPP_FTOK_PATH:-mangoapp}")"
      project_id: ${TELEMON_STEAM_DECK_FPS_MANGOAPP_PROJECT_ID:-65}
    steam_library_roots: $(yaml_csv_list "${TELEMON_STEAM_DECK_FPS_STEAM_LIBRARY_ROOTS:-}")
  nvidia_nvml:
    enabled: $(bool_value "$NVIDIA_NVML_ENABLED")
    library_paths: $(yaml_csv_list "${TELEMON_NVIDIA_LIBRARY_PATHS:-}")
    expose_gpu_name: $(bool_value "$NVIDIA_EXPOSE_GPU_NAME")
    expose_gpu_uuid: $(bool_value "$NVIDIA_EXPOSE_GPU_UUID")
    fan_speed_enabled: $(bool_value "$NVIDIA_FAN_SPEED_ENABLED")

diagnostics:
  enabled: true
  scrape_gap_threshold_seconds: 30
  scheduler_lag_threshold_seconds: 5
  log_scrape_gaps: true
  log_scheduler_lag: true
  log_scrape_interval_changes: true

logging:
  level: "$(yaml_escape "${TELEMON_LOG_LEVEL:-info}")"
CONFIG

exec telemon-exporter "$@"
