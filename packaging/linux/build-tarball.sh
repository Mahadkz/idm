#!/usr/bin/env bash
# Build release binaries offline and assemble a relocatable Linux tarball:
# binaries, browser extensions, install/uninstall scripts, and LICENSE.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

usage() {
    cat <<'EOF'
Usage: build-tarball.sh [options]

  --version VERSION  Version label for the tarball name (default: 0.1.0)
  --target TRIPLE    Rust target triple (default: host; sets the tar suffix)
  --no-build         Package existing binaries without running cargo build
  -h, --help         Show this help

Environment:
  XDM_TARBALL_OUT    Output directory (default: <repo>/target/dist)
EOF
}

VERSION="0.1.0"
TARGET=""
BUILD=1

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || { echo "error: --version needs a value" >&2; exit 1; }
            VERSION=$2
            shift 2
            ;;
        --target)
            [ $# -ge 2 ] || { echo "error: --target needs a value" >&2; exit 1; }
            TARGET=$2
            shift 2
            ;;
        --no-build) BUILD=0; shift ;;
        -h | --help) usage; exit 0 ;;
        *) echo "error: unknown option: $1 (see --help)" >&2; exit 1 ;;
    esac
done

say() { printf '[tarball] %s\n' "$*"; }

cd "$REPO_ROOT"
if [ "$BUILD" -eq 1 ]; then
    say "building release binaries (offline)"
    cargo build --workspace --release --offline
else
    say "packaging existing binaries without building"
fi

if [ -n "$TARGET" ]; then
    BIN_DIR="$REPO_ROOT/target/$TARGET/release"
    SUFFIX="$TARGET"
else
    BIN_DIR="$REPO_ROOT/target/release"
    SUFFIX="$(uname -m)-linux"
fi
for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    [ -f "$BIN_DIR/$name" ] || { echo "error: missing binary $BIN_DIR/$name" >&2; exit 1; }
done

OUT_DIR="${XDM_TARBALL_OUT:-$REPO_ROOT/target/dist}"
STAGE_NAME="xdm-$VERSION-$SUFFIX"
STAGE="$OUT_DIR/$STAGE_NAME"
TARBALL="$OUT_DIR/$STAGE_NAME.tar.gz"

say "staging $STAGE"
rm -rf -- "$STAGE"
mkdir -p -- "$STAGE/bin"

for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    install -m 755 -- "$BIN_DIR/$name" "$STAGE/bin/$name"
done
cp -a -- "$REPO_ROOT/extensions" "$STAGE/extensions"
mkdir -p -- "$STAGE/install/linux" "$STAGE/install/windows"
for f in "$REPO_ROOT"/install/linux/*; do
    install -m "$(stat -c %a -- "$f")" -- "$f" "$STAGE/install/linux/"
done
cp -a -- "$REPO_ROOT"/install/windows/*.ps1 "$STAGE/install/windows/"
install -m 644 -- "$REPO_ROOT/LICENSE" "$STAGE/LICENSE"
[ -f "$OUT_DIR/THIRD_PARTY_LICENSES.md" ] || "$REPO_ROOT/packaging/licenses.sh" "$OUT_DIR/THIRD_PARTY_LICENSES.md"
install -m 644 -- "$OUT_DIR/THIRD_PARTY_LICENSES.md" "$STAGE/THIRD_PARTY_LICENSES.md"

say "creating $TARBALL"
tar -czf "$TARBALL" -C "$OUT_DIR" "$STAGE_NAME"
rm -rf -- "$STAGE"
say "done: $TARBALL"
