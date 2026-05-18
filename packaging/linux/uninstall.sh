#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "uninstall.sh must be run as root" >&2
  exit 1
fi

UNIT_FILE="/etc/systemd/system/telemon-exporter.service"
PROMETHEUS_IP_FILE="/etc/telemon/prometheus-server-ip"

remove_ufw_rule() {
  if [ ! -f "$PROMETHEUS_IP_FILE" ]; then
    return
  fi

  prometheus_ip="$(tr -d '[:space:]' < "$PROMETHEUS_IP_FILE")"
  if [ -n "$prometheus_ip" ] && command -v ufw >/dev/null 2>&1; then
    ufw delete allow from "$prometheus_ip" to any port 9185 proto tcp >/dev/null 2>&1 || true
    if ufw status 2>/dev/null | grep -q '^Status: active'; then
      ufw reload >/dev/null 2>&1 || true
    fi
  fi

  rm -f "$PROMETHEUS_IP_FILE"
}

systemctl stop telemon-exporter.service >/dev/null 2>&1 || true
systemctl disable telemon-exporter.service >/dev/null 2>&1 || true
rm -f "$UNIT_FILE"
remove_ufw_rule
systemctl daemon-reload

echo "telemon-exporter service removed"
echo "configuration remains at /etc/telemon/exporter.yml"
