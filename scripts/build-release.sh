#!/usr/bin/env bash
set -euo pipefail

VERSION="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n 1)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
DIST_DIR="dist/current"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

cargo build --release --workspace
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

case "$OS" in
  linux)
    PLATFORM="linux-${ARCH}"
    EXE_SUFFIX=""
    ;;
  darwin)
    PLATFORM="macos-${ARCH}"
    EXE_SUFFIX=""
    ;;
  mingw*|msys*|cygwin*)
    PLATFORM="windows-${ARCH}"
    EXE_SUFFIX=".exe"
    ;;
  *)
    PLATFORM="${OS}-${ARCH}"
    EXE_SUFFIX=""
    ;;
esac

STAGING_PARENT="$DIST_DIR/staging"
rm -rf "$STAGING_PARENT"

package_binary() {
  local name="$1"
  local source="$TARGET_DIR/release/${name}${EXE_SUFFIX}"
  local archive_basename="${name}-v${VERSION}-${PLATFORM}"
  local archive_name="${archive_basename}.tar.gz"
  local bundle_dir="$STAGING_PARENT/$archive_basename"

  if [ ! -f "$source" ] && [ -f "$TARGET_DIR/release/${name}.exe" ]; then
    source="$TARGET_DIR/release/${name}.exe"
  fi
  [ -f "$source" ] || {
    echo "missing built binary: $source" >&2
    exit 1
  }

  install -d -m 0755 "$bundle_dir"
  install -m 0755 "$source" "$bundle_dir/${name}${EXE_SUFFIX}"

  cat > "$bundle_dir/MANIFEST.txt" <<MANIFEST
$name
Version: $VERSION
Platform: $PLATFORM

Files:
  ${name}${EXE_SUFFIX}
MANIFEST

  case "$name" in
    telemon-exporter)
      install -m 0644 "config.example.yml" "$bundle_dir/config.example.yml"
      install -m 0644 "docs/install-bootstrap.md" "$bundle_dir/README.install.md"
      {
        echo "  config.example.yml"
        echo "  README.install.md"
      } >> "$bundle_dir/MANIFEST.txt"
      if [ "$OS" = "linux" ]; then
        install -m 0755 "install.sh" "$bundle_dir/install.sh"
        install -m 0755 "scripts/install-steamdeck.sh" "$bundle_dir/install-steamdeck.sh"
        install -m 0644 "docs/steamdeck-install.md" "$bundle_dir/README.steamdeck.md"
        install -m 0644 "packaging/steamdeck/telemon-exporter.service.template" "$bundle_dir/telemon-exporter.steamdeck.service.template"
        {
          echo "  install.sh"
          echo "  install-steamdeck.sh"
          echo "  README.steamdeck.md"
          echo "  telemon-exporter.steamdeck.service.template"
        } >> "$bundle_dir/MANIFEST.txt"
      fi
      ;;
    telemon-registry)
      install -m 0644 "deploy/registry/config.yml" "$bundle_dir/registry.example.yml"
      echo "  registry.example.yml" >> "$bundle_dir/MANIFEST.txt"
      ;;
  esac

  tar -czf "$DIST_DIR/$archive_name" -C "$STAGING_PARENT" "$archive_basename"
  install -m 0755 "$source" "$DIST_DIR/${name}_${VERSION}_${PLATFORM}${EXE_SUFFIX}"
  echo "release archive written to $DIST_DIR/$archive_name"
}

package_binary "telemon"
package_binary "telemon-exporter"
package_binary "telemon-registry"

(
  cd "$DIST_DIR"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum telemon-v*.tar.gz telemon-exporter-v*.tar.gz telemon-registry-v*.tar.gz > SHA256SUMS
  else
    shasum -a 256 telemon-v*.tar.gz telemon-exporter-v*.tar.gz telemon-registry-v*.tar.gz > SHA256SUMS
  fi
)

rm -rf "$STAGING_PARENT"
echo "checksum written to $DIST_DIR/SHA256SUMS"
