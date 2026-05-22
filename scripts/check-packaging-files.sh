#!/usr/bin/env bash
set -euo pipefail

required_files=(
  "install.sh"
  "scripts/build-release.sh"
  "scripts/package-all-local.sh"
  "packaging/README.md"
  "packaging/linux/package-deb.sh"
  "packaging/linux/telemon-exporter-setup"
  "packaging/linux/deb/control"
  "packaging/linux/deb/conffiles"
  "packaging/linux/deb/postinst"
  "packaging/linux/deb/prerm"
  "packaging/linux/deb/postrm"
  "packaging/windows/install-service.ps1"
  "packaging/windows/config.default.yml"
  "packaging/windows/uninstall-service.ps1"
  "packaging/windows/service-smoke-test.ps1"
  "packaging/macos/com.telemon.exporter.plist"
  "packaging/macos/install.sh"
  "packaging/macos/uninstall.sh"
  "packaging/macos/service-smoke-test.sh"
  "docs/install-bootstrap.md"
  "docs/install-linux.md"
  "docs/install-windows.md"
  "docs/install-macos.md"
  "deploy/docker-compose.yml"
  "deploy/docker-compose.local-build.yml"
  "deploy/prometheus/prometheus.yml"
  "deploy/registry/config.yml"
  "deploy/registry/Dockerfile"
  "deploy/exporter/Dockerfile"
  "deploy/exporter/entrypoint.sh"
  "deploy/exporter/docker-compose.yml"
  "deploy/exporter/docker-compose.production.yml"
  "deploy/exporter/docker-compose.unraid-test.yml"
  "deploy/exporter/README.md"
  "deploy/exporter/UNRAID_OMV_VALIDATION.md"
  "deploy/exporter/unraid-template.xml"
  "deploy/grafana/dashboards/telemon-overview.json"
  "deploy/grafana/dashboards/telemon-temperature.json"
  "deploy/grafana/dashboards/telemon-gpu.json"
)

for file in "${required_files[@]}"; do
  if [ ! -f "$file" ]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
done

bash -n scripts/build-release.sh
bash -n scripts/package-all-local.sh
bash -n scripts/check-packaging-files.sh
bash -n install.sh
bash -n packaging/linux/install.sh
bash -n packaging/linux/uninstall.sh
bash -n packaging/linux/package-deb.sh
bash -n packaging/linux/telemon-exporter-setup
bash -n packaging/linux/deb/postinst
bash -n packaging/linux/deb/prerm
bash -n packaging/linux/deb/postrm
bash -n packaging/macos/install.sh
bash -n packaging/macos/uninstall.sh
bash -n packaging/macos/service-smoke-test.sh
bash -n deploy/exporter/entrypoint.sh

if [ -d "dist/current" ]; then
  artifact="$(find dist/current -maxdepth 1 -type f -name 'telemon-exporter-v*-linux-*.tar.gz' | sort | head -n 1)"
  if [ -n "$artifact" ]; then
    bash install.sh \
      --dry-run \
      --artifact "$artifact" \
      --registry-server 127.0.0.1:9186 \
      --enrollment-token test-token \
      --user-name test-user \
      --device-name test-device >/dev/null
  fi
fi

if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool deploy/grafana/dashboards/telemon-overview.json >/dev/null
  python3 -m json.tool deploy/grafana/dashboards/telemon-temperature.json >/dev/null
  python3 -m json.tool deploy/grafana/dashboards/telemon-gpu.json >/dev/null
  python3 - <<'PY'
import xml.etree.ElementTree as ET
ET.parse("packaging/macos/com.telemon.exporter.plist")
ET.parse("deploy/exporter/unraid-template.xml")
PY
fi

echo "packaging files look complete"
