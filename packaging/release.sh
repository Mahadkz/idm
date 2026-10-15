#!/usr/bin/env bash
# Build every release artifact into target/dist and write SHA256SUMS.
# Version comes from [workspace.package] in Cargo.toml (override: XDM_VERSION).
# Options: --no-build (reuse existing target/ binaries)
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
BUILD=1; [ "${1:-}" = "--no-build" ] && BUILD=0
cd "$REPO_ROOT"
if [ "$BUILD" = 1 ]; then
    cargo build --release --workspace --offline
    PATH="${XDM_CROSS_BIN:-/nonexistent}:$PATH" cargo build --release --workspace --target x86_64-pc-windows-gnu --offline
fi
rm -rf "$DIST"; mkdir -p "$DIST"
"$PK_DIR/licenses.sh"
"$PK_DIR/extensions/build.sh"
"$PK_DIR/linux/build-tarball.sh" --version "$VERSION" --no-build
"$PK_DIR/linux/build-deb.sh"
"$PK_DIR/linux/build-appimage.sh"
"$PK_DIR/windows/build-installer.sh"
(cd "$DIST" && find . -maxdepth 1 -type f ! -name SHA256SUMS -printf '%f\n' | LC_ALL=C sort | xargs sha256sum > SHA256SUMS)
say "artifacts in $DIST"; cat "$DIST/SHA256SUMS"
