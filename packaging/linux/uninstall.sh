#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "uninstall.sh must be run as root" >&2
  exit 1
fi

UNIT_FILE="/etc/systemd/system/telemon-exporter.service"
TARGET_BINARY="/usr/local/bin/telemon-exporter"
PROMETHEUS_IP_FILE="/etc/telemon/prometheus-server-ip"
PROMETHEUS_FIREWALL_FILE="/etc/telemon/prometheus-firewall-rule"
PRESERVE_FIREWALL=false

usage() {
  cat <<'USAGE'
Usage: sudo bash packaging/linux/uninstall.sh [options]

Options:
  --preserve-firewall   Remove the bootstrap service/binary but keep recorded
                        Prometheus scrape firewall state for package migration.
  --help, -h            Show this help.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --preserve-firewall)
      PRESERVE_FIREWALL=true
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

read_firewall_value() {
  local key="$1"
  local file="$2"
  awk -F= -v key="$key" '$1 == key { print substr($0, length(key) + 2) }' "$file" 2>/dev/null | tail -n 1
}

remove_ufw_rule() {
  local prometheus_ip=""
  local scrape_port="9185"
  local managed="true"

  if [ -f "$PROMETHEUS_FIREWALL_FILE" ]; then
    prometheus_ip="$(read_firewall_value source_ip "$PROMETHEUS_FIREWALL_FILE")"
    scrape_port="$(read_firewall_value port "$PROMETHEUS_FIREWALL_FILE")"
    managed="$(read_firewall_value managed "$PROMETHEUS_FIREWALL_FILE")"
  elif [ -f "$PROMETHEUS_IP_FILE" ]; then
    prometheus_ip="$(tr -d '[:space:]' < "$PROMETHEUS_IP_FILE")"
  else
    return
  fi

  case "$scrape_port" in
    ''|*[!0-9]*|0)
      scrape_port="9185"
      ;;
  esac

  if [ "$managed" = "true" ] && [ -n "$prometheus_ip" ] && command -v ufw >/dev/null 2>&1; then
    ufw delete allow from "$prometheus_ip" to any port "$scrape_port" proto tcp >/dev/null 2>&1 || true
    if ufw status 2>/dev/null | grep -q '^Status: active'; then
      ufw reload >/dev/null 2>&1 || true
    fi
  fi

  rm -f "$PROMETHEUS_IP_FILE" "$PROMETHEUS_FIREWALL_FILE"
}

if command -v systemctl >/dev/null 2>&1; then
  systemctl stop telemon-exporter.service >/dev/null 2>&1 || true
  systemctl disable telemon-exporter.service >/dev/null 2>&1 || true
fi
rm -f "$UNIT_FILE"
rm -f "$TARGET_BINARY"

if [ "$PRESERVE_FIREWALL" = "true" ]; then
  echo "preserved Telemon firewall state under /etc/telemon"
else
  remove_ufw_rule
fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload || true
fi

echo "telemon-exporter service removed"
echo "bootstrap binary removed from $TARGET_BINARY"
echo "configuration remains at /etc/telemon/exporter.yml"
