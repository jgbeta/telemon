#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "uninstall.sh must be run as root" >&2
  exit 1
fi

INSTALL_DIR="/usr/local/libexec/telemon"
CONFIG_DIR="/Library/Application Support/Telemon"
LOG_DIR="/Library/Logs/Telemon"
PLIST="/Library/LaunchDaemons/com.telemon.exporter.plist"
PF_ANCHOR="/etc/pf.anchors/com.telemon.exporter"
PF_CONF="/etc/pf.conf"
PF_MARKER_BEGIN="# BEGIN telemon exporter"
PF_MARKER_END="# END telemon exporter"
PRESERVE_FIREWALL=false
REMOVE_FILES=false

usage() {
  cat <<'USAGE'
Usage: sudo packaging/macos/uninstall.sh [options]

Options:
  --preserve-firewall   Remove the LaunchDaemon but keep Telemon pf rules.
  --remove-files        Remove installed binary, config/state, and logs.
  --help, -h            Show this help.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --preserve-firewall)
      PRESERVE_FIREWALL=true
      shift
      ;;
    --remove-files)
      REMOVE_FILES=true
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      echo "unknown option: $1" >&2
      exit 1
      ;;
  esac
done

remove_pf_rule() {
  local temp_pf_conf

  if [ ! -f "$PF_ANCHOR" ] && ! grep -qF "$PF_MARKER_BEGIN" "$PF_CONF"; then
    return
  fi

  temp_pf_conf="$(mktemp)"
  awk -v begin="$PF_MARKER_BEGIN" -v end="$PF_MARKER_END" '
    $0 == begin { skip = 1; next }
    $0 == end { skip = 0; next }
    skip != 1 { print }
  ' "$PF_CONF" > "$temp_pf_conf"
  install -m 0644 "$temp_pf_conf" "$PF_CONF"
  rm -f "$temp_pf_conf"
  rm -f "$PF_ANCHOR"
  pfctl -f "$PF_CONF" >/dev/null 2>&1 || true
}

if [ -f "$PLIST" ]; then
  launchctl bootout system "$PLIST" >/dev/null 2>&1 || true
  rm -f "$PLIST"
fi

if [ "$PRESERVE_FIREWALL" = "true" ]; then
  echo "preserved Telemon pf rules"
else
  remove_pf_rule
fi

if [ "$REMOVE_FILES" = "true" ]; then
  rm -rf "$INSTALL_DIR" "$CONFIG_DIR" "$LOG_DIR"
  echo "removed Telemon binary, config/state, and logs"
else
  echo "configuration remains at /Library/Application Support/Telemon/exporter.yml"
fi

echo "telemon-exporter LaunchDaemon removed"
