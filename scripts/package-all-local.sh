#!/usr/bin/env bash
set -euo pipefail

bash scripts/build-release.sh

if [ -x packaging/linux/package-deb.sh ]; then
  bash packaging/linux/package-deb.sh
else
  echo "skipping Linux .deb: packaging/linux/package-deb.sh is missing or not executable"
fi

if [ -f packaging/windows/install-service.ps1 ]; then
  echo "Windows package artifact is the versioned release bundle plus packaging/windows/*.ps1 scripts"
else
  echo "skipping Windows scripts: packaging/windows/install-service.ps1 is missing"
fi

if [ -x packaging/macos/install.sh ]; then
  echo "macOS package artifact is the versioned release bundle plus packaging/macos scripts and plist"
else
  echo "skipping macOS scripts: packaging/macos/install.sh is missing or not executable"
fi

if [ -f deploy/exporter/Dockerfile ]; then
  echo "Docker exporter image can be built with: docker build -f deploy/exporter/Dockerfile -t telemon-exporter:dev ."
fi
