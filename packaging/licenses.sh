#!/usr/bin/env bash
# Generate THIRD_PARTY_LICENSES.md from `cargo metadata` and flag copyleft/unknown licenses.
# Usage: licenses.sh [output-file]   (default: target/dist/THIRD_PARTY_LICENSES.md)
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/common.sh"
OUT=${1:-$DIST/THIRD_PARTY_LICENSES.md}
mkdir -p "$(dirname "$OUT")"
cd "$REPO_ROOT"
cargo metadata --format-version 1 --offline --locked 2>/dev/null | python3 -I "$PK_DIR/licenses.py" "$VERSION" > "$OUT"
say "wrote $OUT"
