#!/usr/bin/env bash
set -euo pipefail

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "dpkg-deb is required to build the Debian package" >&2
  exit 1
fi

VERSION="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n 1)"
DEB_ARCH="$(dpkg --print-architecture)"
PACKAGE_NAME="telemon-exporter"
ROOT="target/package/deb-root"
DIST_DIR="dist/linux"
PACKAGE_FILE="$DIST_DIR/${PACKAGE_NAME}_${VERSION}_${DEB_ARCH}.deb"

cargo build --release
rm -rf "$ROOT"
install -d -m 0755 "$ROOT/DEBIAN"
install -d -m 0755 "$ROOT/usr/bin"
install -d -m 0755 "$ROOT/etc/telemon"
install -d -m 0755 "$ROOT/lib/systemd/system"
mkdir -p "$DIST_DIR"

install -m 0755 target/release/telemon-exporter "$ROOT/usr/bin/telemon-exporter"
install -m 0755 packaging/linux/telemon-exporter-setup "$ROOT/usr/bin/telemon-exporter-setup"

sed "s/^Architecture:.*/Architecture: ${DEB_ARCH}/; s/^Version:.*/Version: ${VERSION}/" \
  packaging/linux/deb/control > "$ROOT/DEBIAN/control"
install -m 0644 packaging/linux/deb/conffiles "$ROOT/DEBIAN/conffiles"
install -m 0755 packaging/linux/deb/postinst "$ROOT/DEBIAN/postinst"
install -m 0755 packaging/linux/deb/prerm "$ROOT/DEBIAN/prerm"
install -m 0755 packaging/linux/deb/postrm "$ROOT/DEBIAN/postrm"

install -m 0640 config.example.yml "$ROOT/etc/telemon/exporter.yml"
sed -i 's/listen: "127.0.0.1:9185"/listen: "0.0.0.0:9185"/' "$ROOT/etc/telemon/exporter.yml"
install -m 0644 packaging/linux/telemon-exporter.service \
  "$ROOT/lib/systemd/system/telemon-exporter.service"

dpkg-deb --root-owner-group --build "$ROOT" "$PACKAGE_FILE"
echo "Debian package written to $PACKAGE_FILE"
