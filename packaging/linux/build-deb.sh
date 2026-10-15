#!/usr/bin/env bash
# Build xdm_<version>_amd64.deb from target/release binaries.
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/../common.sh"
[ "${1:-}" = "--build" ] && (cd "$REPO_ROOT" && cargo build --release --workspace --offline)
BIN="$REPO_ROOT/target/release"
for n in xdm-cli xdm-daemon xdm-gui xdm-native-host; do [ -x "$BIN/$n" ] || { echo "error: missing $BIN/$n" >&2; exit 1; }; done
LIC="$DIST/THIRD_PARTY_LICENSES.md"; [ -f "$LIC" ] || "$PK_DIR/licenses.sh"
[ -f "$DIST/xdm-chromium-$VERSION.zip" ] || "$PK_DIR/extensions/build.sh"
mkdir -p "$DIST"
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
R="$W/root"
D="$R/usr/share/doc/xdm"
mkdir -p "$R/DEBIAN" "$R/usr/bin" "$R/usr/share/applications" "$R/usr/share/icons/hicolor/256x256/apps" "$R/usr/share/icons/hicolor/scalable/apps" "$D" "$R/usr/share/xdm/extensions" "$R/usr/lib/systemd/user" "$R/usr/lib/mozilla/native-messaging-hosts"
for n in xdm-cli xdm-daemon xdm-gui xdm-native-host; do install -m755 "$BIN/$n" "$R/usr/bin/$n"; done
install -m644 "$REPO_ROOT/install/linux/xdm.desktop" "$R/usr/share/applications/xdm.desktop"
install -m644 "$REPO_ROOT/assets/xdm.png" "$R/usr/share/icons/hicolor/256x256/apps/xdm.png"
install -m644 "$REPO_ROOT/assets/xdm.svg" "$R/usr/share/icons/hicolor/scalable/apps/xdm.svg"
install -m644 "$REPO_ROOT/LICENSE" "$D/copyright"
install -m644 "$LIC" "$D/THIRD_PARTY_LICENSES.md"
install -m644 "$DIST/xdm-chromium-$VERSION.zip" "$DIST/xdm-firefox-$VERSION.xpi" "$R/usr/share/xdm/extensions/"
sed 's|@XDM_DAEMON@|/usr/bin/xdm-daemon|' "$REPO_ROOT/install/linux/xdm-daemon.service" > "$R/usr/lib/systemd/user/xdm-daemon.service"
chmod 644 "$R/usr/lib/systemd/user/xdm-daemon.service"
printf 'xdm (%s) stable; urgency=low\n\n  * Release %s.\n\n -- XDM Maintainers <noreply@xdm.local>  %s\n' "$VERSION" "$VERSION" "$(date -u -d "@$SOURCE_DATE_EPOCH" '+%a, %d %b %Y %H:%M:%S +0000')" | gzip -9n > "$D/changelog.gz"
# Native messaging manifests (system-wide).
cat > "$R/usr/lib/mozilla/native-messaging-hosts/org.xdm.browser.json" <<EOF
{
  "name": "org.xdm.browser",
  "description": "XDM native messaging bridge",
  "path": "/usr/bin/xdm-native-host",
  "type": "stdio",
  "allowed_extensions": ["xdm@xdm.local"]
}
EOF
for dir in etc/opt/chrome etc/chromium etc/opt/edge; do
    mkdir -p "$R/$dir/native-messaging-hosts"
    cat > "$R/$dir/native-messaging-hosts/org.xdm.browser.json" <<EOF
{
  "name": "org.xdm.browser",
  "description": "XDM native messaging bridge",
  "path": "/usr/bin/xdm-native-host",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$CHROMIUM_ID/"]
}
EOF
done
find "$R/usr" "$R/etc" -type f -name '*.json' -exec chmod 644 {} +
cat > "$R/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
# Per-user daemons are never started, enabled or stopped system-wide.
if [ "$1" = configure ]; then
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
exit 0
EOF
cat > "$R/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
# Running per-user xdm-daemon processes are left alone; they exit on their own
# and user data under ~/.local/share/xdm is never removed.
exit 0
EOF
cat > "$R/DEBIAN/postrm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ] || [ "$1" = purge ]; then
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
exit 0
EOF
chmod 755 "$R/DEBIAN/postinst" "$R/DEBIAN/prerm" "$R/DEBIAN/postrm"
SIZE=$(du -sk --exclude=DEBIAN "$R" | cut -f1)
cat > "$R/DEBIAN/control" <<EOF
Package: xdm
Version: $VERSION
Section: net
Priority: optional
Architecture: amd64
Depends: libc6, libfontconfig1, libfreetype6
Recommends: ffmpeg
Installed-Size: $SIZE
Maintainer: XDM Maintainers <noreply@xdm.local>
Homepage: https://github.com/xdm
Description: Download manager with browser integration
 XDM downloads files with parallel connections, resumes interrupted
 transfers and integrates with Chrome, Chromium, Edge and Firefox through a
 native messaging host. Includes the xdm-gui, xdm-cli, xdm-daemon and
 xdm-native-host programs. ffmpeg is used for merging streamed media.
EOF
find "$R" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
OUT="$DIST/xdm_${VERSION}_amd64.deb"
rm -f "$OUT"
dpkg-deb --root-owner-group -Zxz -b "$R" "$OUT" >/dev/null
say "built $OUT"
