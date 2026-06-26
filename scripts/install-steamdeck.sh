#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="telemon-exporter"
LISTEN_PORT="9185"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ARTIFACT="${TELEMON_ARTIFACT:-}"
SOURCE_BINARY="${TELEMON_BINARY:-}"
TMP_WORK_DIR=""

CONFIG_ROOT="${XDG_CONFIG_HOME:-$HOME/.config}"
STATE_ROOT="${XDG_STATE_HOME:-$HOME/.local/state}"
BIN_DIR="$HOME/.local/bin"
CONFIG_DIR="$CONFIG_ROOT/telemon"
CONFIG_FILE="$CONFIG_DIR/exporter.yml"
STATE_DIR="$STATE_ROOT/telemon/exporter"
TARGET_BINARY="$BIN_DIR/telemon-exporter"
USER_UNIT_DIR="$CONFIG_ROOT/systemd/user"
USER_UNIT_FILE="$USER_UNIT_DIR/${SERVICE_NAME}.service"

REGISTRY_SERVER="${TELEMON_REGISTRY_SERVER:-}"
ENROLLMENT_TOKEN="${TELEMON_ENROLLMENT_TOKEN:-}"
USER_NAME="${TELEMON_USER_NAME:-}"
DEVICE_NAME="${TELEMON_DEVICE_NAME:-steam-deck}"
MACHINE_UUID="${TELEMON_MACHINE_UUID:-}"
ADVERTISED_ADDR="${TELEMON_ADVERTISED_ADDR:-}"
FORCE_CONFIG=false
ENABLE_LINGER=false
DRY_RUN=false
ENABLE_FPS="${TELEMON_ENABLE_FPS:-false}"

log() {
  printf '[telemon-steamdeck] %s\n' "$*"
}

warn() {
  printf '[telemon-steamdeck] warning: %s\n' "$*" >&2
}

set_config_permissions() {
  [ -f "$CONFIG_FILE" ] || return 0
  chmod 0600 "$CONFIG_FILE" 2>/dev/null || true
}

die() {
  printf '[telemon-steamdeck] error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "${TMP_WORK_DIR:-}" ] && [ -d "$TMP_WORK_DIR" ]; then
    rm -rf "$TMP_WORK_DIR"
  fi
}
trap cleanup EXIT

usage() {
  cat <<'USAGE'
Telemon Steam Deck compatibility installer.

This is a user-space Steam Deck bootstrap path for a prebuilt telemon-exporter
binary. It does not require Cargo, pacman, sudo, or disabling the SteamOS
read-only filesystem.

Usage:
  bash scripts/install-steamdeck.sh --artifact PATH [options]
  bash install-steamdeck.sh [PATH_TO_ARTIFACT_OR_BINARY] [options]

Options:
  --artifact PATH              Prebuilt telemon-exporter Linux x86_64 tar.gz.
  --binary PATH                Prebuilt telemon-exporter binary.
  --registry-server HOST:PORT  Telemon registry/core server. Adds http:// if omitted.
  --enrollment-token TOKEN     Enrollment token for registry registration.
  --user-name NAME             User label stored in Telemon identity metrics.
  --device-name NAME           Device label. Default: steam-deck.
  --machine-uuid UUID          Optional existing machine UUID for dual-boot grouping.
  --advertised-addr HOST       Optional scrape host/IP sent to the registry.
  --force-config               Replace existing config after writing a timestamped backup.
  --enable-linger              Run sudo loginctl enable-linger deck after install.
  --enable-fps                 Enable experimental /fps Gamescope/MangoApp frame metrics.
  --dry-run                    Resolve inputs and print planned paths without installing.
  -h, --help                   Show this help.

Environment fallbacks:
  TELEMON_ARTIFACT, TELEMON_BINARY, TELEMON_REGISTRY_SERVER,
  TELEMON_ENROLLMENT_TOKEN, TELEMON_USER_NAME, TELEMON_DEVICE_NAME,
  TELEMON_MACHINE_UUID, TELEMON_ADVERTISED_ADDR, TELEMON_ENABLE_FPS
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --artifact)
      [ "$#" -ge 2 ] || die "--artifact requires a path"
      SOURCE_ARTIFACT="$2"
      shift 2
      ;;
    --binary)
      [ "$#" -ge 2 ] || die "--binary requires a path"
      SOURCE_BINARY="$2"
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
      FORCE_CONFIG=true
      shift
      ;;
    --enable-linger)
      ENABLE_LINGER=true
      shift
      ;;
    --enable-fps)
      ENABLE_FPS=true
      shift
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --*)
      die "unknown option: $1"
      ;;
    *)
      if [ -n "$SOURCE_ARTIFACT" ] || [ -n "$SOURCE_BINARY" ]; then
        die "unexpected positional argument: $1"
      fi
      case "$1" in
        *.tar.gz|*.tgz)
          SOURCE_ARTIFACT="$1"
          ;;
        *)
          SOURCE_BINARY="$1"
          ;;
      esac
      shift
      ;;
  esac
done

case "$ENABLE_FPS" in
  true|false)
    ;;
  TRUE|True|1|yes|YES|Yes|on|ON|On)
    ENABLE_FPS=true
    ;;
  FALSE|False|0|no|NO|No|off|OFF|Off)
    ENABLE_FPS=false
    ;;
  *)
    die "invalid TELEMON_ENABLE_FPS value: $ENABLE_FPS"
    ;;
esac

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_deck_user() {
  if [ "$DRY_RUN" = true ]; then
    return
  fi

  local current_user
  current_user="$(id -un)"
  if [ "$current_user" != "deck" ]; then
    die "run this installer from Desktop Mode as the deck user, not as $current_user"
  fi

  if [ "$(id -u)" -eq 0 ]; then
    die "do not run this installer with sudo; it installs a user service for deck"
  fi
}

default_user_name() {
  id -un 2>/dev/null || printf 'deck\n'
}

prompt_value() {
  local prompt="$1"
  local default_value="$2"
  local value

  if [ -n "$default_value" ]; then
    printf '%s [%s]: ' "$prompt" "$default_value" >&2
  else
    printf '%s: ' "$prompt" >&2
  fi
  IFS= read -r value
  if [ -z "$value" ]; then
    printf '%s\n' "$default_value"
  else
    printf '%s\n' "$value"
  fi
}

prompt_secret() {
  local prompt="$1"
  local value

  printf '%s: ' "$prompt" >&2
  IFS= read -r -s value
  printf '\n' >&2
  printf '%s\n' "$value"
}

prompt_config_inputs() {
  if [ "$DRY_RUN" = true ]; then
    [ -n "$USER_NAME" ] || USER_NAME="$(default_user_name)"
    [ -n "$DEVICE_NAME" ] || DEVICE_NAME="steam-deck"
    return
  fi

  if [ -z "$REGISTRY_SERVER" ] && [ -t 0 ]; then
    REGISTRY_SERVER="$(prompt_value 'Registry server HOST:PORT (blank to disable registration)' '')"
  fi

  if [ -n "$REGISTRY_SERVER" ] && [ -z "$ENROLLMENT_TOKEN" ]; then
    if [ -t 0 ]; then
      ENROLLMENT_TOKEN="$(prompt_secret 'Enrollment token')"
    else
      die "--enrollment-token is required when --registry-server is set"
    fi
  fi

  if [ -z "$USER_NAME" ]; then
    if [ -t 0 ]; then
      USER_NAME="$(prompt_value 'User name label' "$(default_user_name)")"
    else
      USER_NAME="$(default_user_name)"
    fi
  fi

  if [ -z "$DEVICE_NAME" ]; then
    if [ -t 0 ]; then
      DEVICE_NAME="$(prompt_value 'Device name label' 'steam-deck')"
    else
      DEVICE_NAME="steam-deck"
    fi
  fi

  if [ -z "$ADVERTISED_ADDR" ] && [ -n "$REGISTRY_SERVER" ] && [ -t 0 ]; then
    ADVERTISED_ADDR="$(prompt_value 'Advertised scrape host/IP (blank to let registry observe)' '')"
  fi

  if [ -z "$MACHINE_UUID" ] && [ -t 0 ]; then
    MACHINE_UUID="$(prompt_value 'Machine UUID (blank for auto-generated local machine UUID)' '')"
  fi
}

find_first_artifact() {
  local dir="$1"
  if [ -d "$dir" ]; then
    find "$dir" -maxdepth 1 -type f -name 'telemon-exporter-v*-linux-x86_64.tar.gz' | sort | tail -n 1
  fi
}

find_default_artifact() {
  local candidate

  if [ -n "$SOURCE_ARTIFACT" ] || [ -n "$SOURCE_BINARY" ]; then
    return
  fi

  if [ -f "$SCRIPT_DIR/telemon-exporter" ]; then
    SOURCE_BINARY="$SCRIPT_DIR/telemon-exporter"
    return
  fi

  candidate="$(find_first_artifact "$SCRIPT_DIR/../dist/current")"
  if [ -n "$candidate" ]; then
    SOURCE_ARTIFACT="$candidate"
    return
  fi

  candidate="$(find_first_artifact "$SCRIPT_DIR/dist/current")"
  if [ -n "$candidate" ]; then
    SOURCE_ARTIFACT="$candidate"
    return
  fi

  candidate="$(find_first_artifact "$PWD/dist/current")"
  if [ -n "$candidate" ]; then
    SOURCE_ARTIFACT="$candidate"
  fi
}

resolve_source_binary() {
  find_default_artifact

  if [ -n "$SOURCE_BINARY" ]; then
    [ -f "$SOURCE_BINARY" ] || die "binary not found: $SOURCE_BINARY"
    return
  fi

  [ -n "$SOURCE_ARTIFACT" ] || die "provide --artifact PATH or --binary PATH"
  [ -f "$SOURCE_ARTIFACT" ] || die "artifact not found: $SOURCE_ARTIFACT"

  case "$SOURCE_ARTIFACT" in
    *.tar.gz|*.tgz)
      require_command tar
      require_command find
      TMP_WORK_DIR="$(mktemp -d)"
      tar -xzf "$SOURCE_ARTIFACT" -C "$TMP_WORK_DIR"
      SOURCE_BINARY="$(find "$TMP_WORK_DIR" -type f -name 'telemon-exporter' | sort | head -n 1)"
      [ -n "$SOURCE_BINARY" ] || die "artifact does not contain a telemon-exporter binary: $SOURCE_ARTIFACT"
      ;;
    *)
      die "unsupported artifact format: $SOURCE_ARTIFACT"
      ;;
  esac
}

normalize_registry_addr() {
  local value="$1"
  case "$value" in
    http://*|https://*)
      printf '%s\n' "$value"
      ;;
    '')
      printf '\n'
      ;;
    *)
      printf 'http://%s\n' "$value"
      ;;
  esac
}

yaml_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

install_binary() {
  resolve_source_binary

  if [ "$DRY_RUN" = true ]; then
    log "would install binary from $SOURCE_BINARY to $TARGET_BINARY"
    return
  fi

  install -d -m 0755 "$BIN_DIR"
  install -m 0755 "$SOURCE_BINARY" "$TARGET_BINARY"
  [ -x "$TARGET_BINARY" ] || die "installed binary is not executable: $TARGET_BINARY"
  log "installed binary: $TARGET_BINARY"
}

write_config() {
  local registration_enabled="false"
  local registry_addr=""
  local timestamp

  if [ -n "$REGISTRY_SERVER" ]; then
    registration_enabled="true"
    registry_addr="$(normalize_registry_addr "$REGISTRY_SERVER")"
  fi

  if [ "$registration_enabled" = true ] && [ -z "$ENROLLMENT_TOKEN" ]; then
    die "enrollment token is required when registry registration is enabled"
  fi

  if [ "$DRY_RUN" = true ]; then
    log "would write config to $CONFIG_FILE (registration_enabled=$registration_enabled, device_name=$DEVICE_NAME, fps_enabled=$ENABLE_FPS)"
    return
  fi

  install -d -m 0755 "$CONFIG_DIR"
  install -d -m 0755 "$STATE_DIR"

  if [ -f "$CONFIG_FILE" ] && [ "$FORCE_CONFIG" != true ]; then
    log "preserving existing config: $CONFIG_FILE"
    set_config_permissions
    if [ -n "$REGISTRY_SERVER" ] || [ -n "$ENROLLMENT_TOKEN" ] || [ -n "$MACHINE_UUID" ] || [ -n "$ADVERTISED_ADDR" ] || [ "$ENABLE_FPS" = true ]; then
      warn "registration, identity, or FPS options were provided, but existing config was preserved; use --force-config to rewrite it"
    fi
    return
  fi

  if [ -f "$CONFIG_FILE" ]; then
    timestamp="$(date +%Y%m%d-%H%M%S)"
    cp "$CONFIG_FILE" "$CONFIG_FILE.bak-$timestamp"
    chmod 0600 "$CONFIG_FILE.bak-$timestamp" 2>/dev/null || true
    log "backed up existing config to $CONFIG_FILE.bak-$timestamp"
  fi

  local user_name device_name machine_uuid machine_uuid_file device_id_file advertised_addr token escaped_registry_addr
  user_name="$(yaml_escape "$USER_NAME")"
  device_name="$(yaml_escape "$DEVICE_NAME")"
  machine_uuid="$(yaml_escape "$MACHINE_UUID")"
  machine_uuid_file="$(yaml_escape "$STATE_DIR/machine-id")"
  device_id_file="$(yaml_escape "$STATE_DIR/device-id")"
  advertised_addr="$(yaml_escape "$ADVERTISED_ADDR")"
  token="$(yaml_escape "$ENROLLMENT_TOKEN")"
  escaped_registry_addr="$(yaml_escape "$registry_addr")"

  cat > "$CONFIG_FILE" <<YAML
server:
  listen: "0.0.0.0:${LISTEN_PORT}"
  metrics_path: "/metrics"
  static_metrics_path: "/metrics/static"
  fps_metrics_path: "/fps"

identity:
  user_name: "$user_name"
  device_name: "$device_name"
  machine_uuid: "$machine_uuid"
  machine_uuid_file: "$machine_uuid_file"

registration:
  enabled: $registration_enabled
  registry_addr: "$escaped_registry_addr"
  enrollment_token: "$token"
  device_id_file: "$device_id_file"
  heartbeat_interval_seconds: 30
  scrape_port: ${LISTEN_PORT}
  advertised_addr: "$advertised_addr"

collection:
  scrape_cache_stale_after_seconds: 60
  system_interval_seconds: 5
  macos_thermal_state_interval_seconds: 15
  temperature_interval_seconds: 5
  sensor_rescan_interval_seconds: 300
  gpu_interval_seconds: 5
  windows_baseline_interval_seconds: 15
  windows_inventory_interval_seconds: 300
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
  system:
    enabled: true
    cpu_enabled: true
    memory_enabled: true
    uptime_enabled: true

  macos_thermal_state:
    enabled: false

  macos_macmon:
    enabled: false
    sample_interval_seconds: 1
    sample_window_milliseconds: 1000
    stale_after_seconds: 5
    reinitialize_after_consecutive_errors: 5
    min_temperature_celsius: 1
    max_temperature_celsius: 130
    max_power_watts: 300

  macos_exact_temperature_experimental:
    enabled: false

  linux_hwmon:
    enabled: true
    root: "/sys/class/hwmon"
    include_unknown_sensors: true
    nvme_enrichment_enabled: true
    expose_storage_model: true
    sensor_allowlist: []
    sensor_denylist: []

  linux_power_supply:
    enabled: true
    root: "/sys/class/power_supply"
    derive_power_when_missing: true

  linux_amdgpu:
    enabled: true
    root: "/sys/class/drm"
    include_diagnostic_only_gpu_metrics: true

  linux_drm:
    enabled: false
    drm_root: "/sys/class/drm"
    proc_root: "/proc"
    target_pid:
    include_hwmon: true
    include_fdinfo: false

  steam_deck_game_state:
    enabled: true
    poll_interval_seconds: 1
    stop_debounce_seconds: 5
    xprop_path: "xprop"
    display: ":0"
    auto_discover_steam_display: true
    desktop_fallback_enabled: true
    process_fallback_enabled: true

  steam_deck_fps:
    enabled: $ENABLE_FPS
    windows_seconds: [1, 5, 60]
    include_appid_label: true
    include_game_name_label: true
    max_frame_time_milliseconds: 1000
    poll_interval_milliseconds: 100
    max_messages_per_poll: 512
    source_preference: ["gamescope_wayland", "mangohud_log", "gamescope_mangoapp"]
    gamescope_wayland:
      enabled: $ENABLE_FPS
      display: ""
    mangohud_log:
      enabled: $ENABLE_FPS
      paths: []
      auto_discover: true
    gamescope_mangoapp:
      enabled: false
      ftok_path: "$HOME/mangoapp"
      project_id: 65
      legacy_failed_ftok_fallback_enabled: false
      allow_destructive_read: false
    steam_library_roots: []

  nvidia_nvml:
    enabled: false
    library_paths: []
    expose_gpu_name: true
    expose_gpu_uuid: false
    fan_speed_enabled: true

  windows_baseline:
    enabled: false
    include_removable_drives: false
    include_remote_drives: false
    network_interface_allowlist: []
    network_interface_denylist:
      - "loopback"
      - "isatap"
      - "teredo"

  windows_inventory:
    enabled: false

  windows_lhm_http:
    enabled: false
    url: "http://127.0.0.1:8085/data.json"
    timeout_ms: 1500
    include_unknown_sensors: false
    sensor_allowlist: []
    sensor_denylist: []
    require_provider: false

  windows_lhm_wmi:
    enabled: false
    namespace: "root\\LibreHardwareMonitor"
    include_unknown_sensors: false
    sensor_allowlist: []
    sensor_denylist: []
    require_provider: false

diagnostics:
  enabled: true
  scrape_gap_threshold_seconds: 30
  scheduler_lag_threshold_seconds: 5
  log_scrape_gaps: true
  log_scheduler_lag: true
  log_scrape_interval_changes: true

logging:
  level: "info"
YAML

  set_config_permissions

  if [ "$ENABLE_FPS" = true ]; then
    : > "$HOME/mangoapp"
    log "ensured MangoApp ftok marker: $HOME/mangoapp"
  fi

  log "wrote config: $CONFIG_FILE"
}

find_service_template() {
  local candidate
  for candidate in \
    "$SCRIPT_DIR/../packaging/steamdeck/telemon-exporter.service.template" \
    "$SCRIPT_DIR/packaging/steamdeck/telemon-exporter.service.template" \
    "$SCRIPT_DIR/telemon-exporter.steamdeck.service.template"; do
    if [ -f "$candidate" ]; then
      printf '%s\n' "$candidate"
      return
    fi
  done
}

write_service_unit() {
  local template

  if [ "$DRY_RUN" = true ]; then
    log "would write user service to $USER_UNIT_FILE"
    return
  fi

  install -d -m 0755 "$USER_UNIT_DIR"
  template="$(find_service_template)"
  if [ -n "$template" ]; then
    install -m 0644 "$template" "$USER_UNIT_FILE"
  else
    cat > "$USER_UNIT_FILE" <<'UNIT'
[Unit]
Description=Telemon exporter for Steam Deck
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=%h/.local/bin/telemon-exporter run --config %h/.config/telemon/exporter.yml
Restart=always
RestartSec=5
WorkingDirectory=%h
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
UNIT
  fi
  log "wrote user service: $USER_UNIT_FILE"
}

validate_config() {
  if [ "$DRY_RUN" = true ]; then
    return
  fi

  "$TARGET_BINARY" check --config "$CONFIG_FILE" >/dev/null
  log "validated exporter config"
}

enable_service() {
  if [ "$DRY_RUN" = true ]; then
    log "would run: systemctl --user daemon-reload"
    log "would run: systemctl --user enable --now ${SERVICE_NAME}.service"
    return
  fi

  if ! command -v systemctl >/dev/null 2>&1; then
    warn "systemctl is not available; start the exporter manually with:"
    printf '  %s run --config %s\n' "$TARGET_BINARY" "$CONFIG_FILE" >&2
    return
  fi

  if ! systemctl --user daemon-reload; then
    warn "could not reload the user systemd manager; start manually with:"
    printf '  %s run --config %s\n' "$TARGET_BINARY" "$CONFIG_FILE" >&2
    return
  fi

  if ! systemctl --user enable --now "${SERVICE_NAME}.service"; then
    warn "could not enable/start the user service; inspect with:"
    printf '  systemctl --user status %s.service --no-pager\n' "$SERVICE_NAME" >&2
    printf '  journalctl --user -u %s.service -n 100 --no-pager\n' "$SERVICE_NAME" >&2
    printf 'Manual start command:\n  %s run --config %s\n' "$TARGET_BINARY" "$CONFIG_FILE" >&2
    return
  fi

  log "enabled and started user service: ${SERVICE_NAME}.service"
}

enable_linger_if_requested() {
  if [ "$ENABLE_LINGER" != true ]; then
    return
  fi

  if [ "$DRY_RUN" = true ]; then
    log "would run: sudo loginctl enable-linger deck"
    return
  fi

  if ! command -v sudo >/dev/null 2>&1 || ! command -v loginctl >/dev/null 2>&1; then
    warn "sudo/loginctl unavailable; skipping linger setup"
    return
  fi

  sudo loginctl enable-linger deck
  log "enabled linger for deck"
}

print_summary() {
  cat <<SUMMARY
[telemon-steamdeck] install profile complete

Paths:
  binary:  $TARGET_BINARY
  config:  $CONFIG_FILE
  state:   $STATE_DIR
  service: $USER_UNIT_FILE

Verify on the Steam Deck:
  systemctl --user status ${SERVICE_NAME}.service --no-pager
  journalctl --user -u ${SERVICE_NAME}.service -n 100 --no-pager
  curl http://127.0.0.1:${LISTEN_PORT}/healthz
  curl http://127.0.0.1:${LISTEN_PORT}/readyz
  curl http://127.0.0.1:${LISTEN_PORT}/metrics
  curl http://127.0.0.1:${LISTEN_PORT}/metrics/static

Prometheus should scrape:
  http://<steam-deck-lan-ip>:${LISTEN_PORT}/metrics
  http://<steam-deck-lan-ip>:${LISTEN_PORT}/metrics/static

This installer does not change SteamOS firewall or router settings. If remote
scrapes fail, validate LAN reachability to port ${LISTEN_PORT} from the
Prometheus host.
SUMMARY
}

main() {
  require_command date
  require_command id
  require_command install
  require_command mktemp
  require_command rm
  require_command sed

  log "Steam Deck user-space installer; prebuilt binary required, no Cargo or pacman needed"
  require_deck_user
  prompt_config_inputs
  install_binary
  write_config
  write_service_unit
  validate_config
  enable_service
  enable_linger_if_requested
  print_summary
}

main
