#!/usr/bin/env bash
# Build XDM-<version>-x86_64.AppImage from target/release binaries (run `cargo build --release --workspace --offline` first, or pass --build).
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/../common.sh"
[ "${1:-}" = "--build" ] && (cd "$REPO_ROOT" && cargo build --release --workspace --offline)
BIN="$REPO_ROOT/target/release"
for n in xdm-cli xdm-daemon xdm-gui xdm-native-host; do [ -x "$BIN/$n" ] || { echo "error: missing $BIN/$n" >&2; exit 1; }; done
LIC="$DIST/THIRD_PARTY_LICENSES.md"; [ -f "$LIC" ] || "$PK_DIR/licenses.sh"
APPIMAGETOOL="${APPIMAGETOOL:-$TOOLS/appimagetool}"
[ -x "$APPIMAGETOOL" ] || { echo "error: appimagetool not found at $APPIMAGETOOL" >&2; exit 1; }
mkdir -p "$DIST"
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
A="$W/XDM.AppDir"
mkdir -p "$A/usr/bin" "$A/usr/share/applications" "$A/usr/share/icons/hicolor/256x256/apps" "$A/usr/share/icons/hicolor/scalable/apps" "$A/usr/share/doc/xdm"
for n in xdm-cli xdm-daemon xdm-gui xdm-native-host; do install -m755 "$BIN/$n" "$A/usr/bin/$n"; done
install -m644 "$REPO_ROOT/assets/xdm.png" "$A/usr/share/icons/hicolor/256x256/apps/xdm.png"
install -m644 "$REPO_ROOT/assets/xdm.svg" "$A/usr/share/icons/hicolor/scalable/apps/xdm.svg"
install -m644 "$REPO_ROOT/assets/xdm.png" "$A/xdm.png"
install -m644 "$REPO_ROOT/LICENSE" "$A/usr/share/doc/xdm/LICENSE"
install -m644 "$LIC" "$A/usr/share/doc/xdm/THIRD_PARTY_LICENSES.md"
cp -a "$REPO_ROOT/install/linux/xdm.desktop" "$A/usr/share/applications/xdm.desktop"
sed -i 's|^Exec=.*|Exec=xdm-gui %u|' "$A/usr/share/applications/xdm.desktop"
cp "$A/usr/share/applications/xdm.desktop" "$A/xdm.desktop"
cat > "$A/AppRun" <<'EOF'
#!/bin/sh
# Start the GUI; make sure one per-user daemon is running first.
HERE=$(dirname "$(readlink -f "$0")")
export PATH="$HERE/usr/bin:$PATH"
case "${1:-}" in
    xdm-cli|xdm-daemon|xdm-native-host|xdm-gui) b=$1; shift; exec "$HERE/usr/bin/$b" "$@" ;;
esac
if ! pgrep -u "$(id -u)" -x xdm-daemon >/dev/null 2>&1; then
    nohup "$HERE/usr/bin/xdm-daemon" >/dev/null 2>&1 &
    sleep 1
fi
exec "$HERE/usr/bin/xdm-gui" "$@"
EOF
chmod 755 "$A/AppRun"
find "$A" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
OUT="$DIST/XDM-$VERSION-x86_64.AppImage"
rm -f "$OUT"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" --no-appstream "$A" "$OUT" >"$W/log" 2>&1 || { cat "$W/log"; exit 1; }
say "built $OUT"
