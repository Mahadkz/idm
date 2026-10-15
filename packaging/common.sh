# Shared helpers; source from packaging scripts.
PK_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$PK_DIR/.." && pwd)
DIST="${XDM_DIST:-$REPO_ROOT/target/dist}"
TOOLS="${XDM_PK_TOOLS:-$HOME/.local/share/xdm-toolchain/pk}"
# Single version source: [workspace.package] version in the root Cargo.toml.
xdm_version() {
    awk '/^\[workspace.package\]/{f=1;next} /^\[/{f=0} f&&/^version[ ]*=/{gsub(/[" ]/,"",$0);sub(/version=/,"");print;exit}' "$REPO_ROOT/Cargo.toml"
}
VERSION=${XDM_VERSION:-$(xdm_version)}
[ -n "$VERSION" ] || { echo "error: cannot read workspace version" >&2; exit 1; }
CHROMIUM_ID=$(cat "$PK_DIR/extensions/chromium-id.txt")
say() { printf '[%s] %s\n' "$(basename "$0" .sh)" "$*"; }
# Fixed mtime for reproducible archives.
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$REPO_ROOT" log -1 --format=%ct 2>/dev/null || echo 0)}
umask 022
