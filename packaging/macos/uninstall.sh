#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "uninstall.sh must be run as root" >&2
  exit 1
fi

PLIST="/Library/LaunchDaemons/com.telemon.exporter.plist"
PF_ANCHOR="/etc/pf.anchors/com.telemon.exporter"
PF_CONF="/etc/pf.conf"
PF_MARKER_BEGIN="# BEGIN telemon exporter"
PF_MARKER_END="# END telemon exporter"

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
remove_pf_rule

echo "telemon-exporter LaunchDaemon removed"
echo "configuration remains at /Library/Application Support/Telemon/exporter.yml"
