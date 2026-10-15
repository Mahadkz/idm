#!/usr/bin/env bash
# Build XDM-<version>-setup.exe (per-user NSIS) from cross-compiled release binaries.
# Prereq: PATH=$XDM_CROSS_BIN:$PATH cargo build --release --workspace --target x86_64-pc-windows-gnu --offline
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/../common.sh"
BIN="$REPO_ROOT/target/x86_64-pc-windows-gnu/release"
for n in xdm-cli xdm-daemon xdm-gui xdm-native-host; do [ -f "$BIN/$n.exe" ] || { echo "error: missing $BIN/$n.exe" >&2; exit 1; }; done
MAKENSIS="${MAKENSIS:-$TOOLS/nsis/usr/bin/makensis}"; [ -x "$MAKENSIS" ] || MAKENSIS=$(command -v makensis)
export NSISDIR="${NSISDIR:-$TOOLS/nsis/usr/share/nsis}"
LIC="$DIST/THIRD_PARTY_LICENSES.md"; [ -f "$LIC" ] || "$PK_DIR/licenses.sh"
[ -f "$DIST/xdm-chromium-$VERSION.zip" ] || "$PK_DIR/extensions/build.sh"
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
mkdir -p "$W/src/assets" "$W/ext"
cp "$REPO_ROOT/LICENSE" "$W/src/"; cp "$LIC" "$W/src/"; cp "$REPO_ROOT/assets/xdm.ico" "$W/src/assets/"
cp "$DIST/xdm-chromium-$VERSION.zip" "$DIST/xdm-firefox-$VERSION.xpi" "$W/ext/"
cat > "$W/ext/org.xdm.browser.chrome.json" <<EOF
{
  "name": "org.xdm.browser",
  "description": "XDM native messaging bridge",
  "path": "xdm-native-host.exe",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$CHROMIUM_ID/"]
}
EOF
cat > "$W/ext/org.xdm.browser.firefox.json" <<'EOF'
{
  "name": "org.xdm.browser",
  "description": "XDM native messaging bridge",
  "path": "xdm-native-host.exe",
  "type": "stdio",
  "allowed_extensions": ["xdm@xdm.local"]
}
EOF
OUT="$DIST/XDM-$VERSION-setup.exe"
mkdir -p "$DIST"; rm -f "$OUT"
"$MAKENSIS" -V2 -DVERSION="$VERSION" -DBINDIR="$BIN" -DEXTDIR="$W/ext" -DSRCDIR="$W/src" -DOUTFILE="$OUT" "$PK_DIR/windows/xdm.nsi"
say "built $OUT (untested on real Windows)"
