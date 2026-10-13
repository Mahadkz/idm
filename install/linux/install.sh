#!/usr/bin/env bash
# User-level XDM installer for Linux. No root required. Re-runs are idempotent.
# Installed paths are recorded in a manifest so uninstall.sh removes exactly these.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)

usage() {
    cat <<'EOF'
Usage: install.sh [options]

  --source-dir DIR     Directory holding built binaries (default: target/release,
                       or bin/ next to install/ inside an extracted tarball)
  --bin-dir DIR        Install binaries here (default: ~/.local/bin)
  --autostart          Enable and start the xdm-daemon systemd --user unit
  --browser NAME       Chromium-family browser to register: chrome, chromium,
                       edge or brave (repeatable; default: all four).
                       Firefox is always registered via its own script.
  --extension-id ID    Chromium extension ID (repeatable). Default: computed
                       from the bundled unpacked extensions/chromium directory
  --dry-run            Print actions without changing anything
  -h, --help           Show this help

Environment:
  XDM_NO_SYSTEMD=1     Never invoke systemctl (used by tests and sandboxes)
  XDG_*                Follow the XDG base directory spec as usual
EOF
}

note() { printf '[install] %s\n' "$*"; }
warn() { printf '[install] warning: %s\n' "$*" >&2; }

die() {
    printf '[install] error: %s\n' "$*" >&2
    exit 1
}

DRY_RUN=0
AUTOSTART=0
SOURCE_DIR=""
BIN_DIR="${XDM_BIN_DIR:-$HOME/.local/bin}"
BROWSERS=()
EXTENSION_IDS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --source-dir)
            [ $# -ge 2 ] || die "--source-dir needs a value"
            SOURCE_DIR=$2
            shift 2
            ;;
        --bin-dir)
            [ $# -ge 2 ] || die "--bin-dir needs a value"
            BIN_DIR=$2
            shift 2
            ;;
        --autostart) AUTOSTART=1; shift ;;
        --browser)
            [ $# -ge 2 ] || die "--browser needs a value"
            case "$2" in
                chrome | chromium | edge | brave) BROWSERS+=("$2") ;;
                *) die "unknown browser: $2 (want chrome, chromium, edge or brave)" ;;
            esac
            shift 2
            ;;
        --extension-id)
            [ $# -ge 2 ] || die "--extension-id needs a value"
            EXTENSION_IDS+=("$2")
            shift 2
            ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done

[ -n "${HOME:-}" ] || die "HOME is not set"

APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/xdm"
SYSTEMD_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
MANIFEST="$STATE_DIR/install-manifest"

MANIFEST_ENTRIES=()

record() { MANIFEST_ENTRIES+=("$1"); }

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        note "dry-run: $*"
        return 0
    fi
    "$@"
}

ensure_dir() {
    local dir=$1
    if [ -d "$dir" ]; then
        return 0
    fi
    note "create directory $dir"
    [ "$DRY_RUN" -eq 1 ] || mkdir -p -- "$dir"
    record "$dir"
}

abs_dir() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$PWD" "$1" ;;
    esac
}

copy_binary() {
    local src="$SOURCE_DIR/$1" dst="$BIN_DIR/$1"
    note "install $dst"
    if [ "$DRY_RUN" -eq 0 ]; then
        cp -f -- "$src" "$dst"
        chmod 755 -- "$dst"
    fi
    record "$dst"
}

render_template() {
    local template=$1 dest=$2
    shift 2
    local sed_args=()
    while [ $# -gt 0 ]; do
        sed_args+=(-e "$1")
        shift
    done
    note "write $dest"
    if [ "$DRY_RUN" -eq 0 ]; then
        sed "${sed_args[@]}" -- "$template" >"$dest"
    fi
}

# Chromium derives unpacked-extension IDs from the absolute path of the
# extension directory: sha256(path), first 16 hex digits mapped to a-p.
default_chromium_id() {
    local ext_dir="$REPO_ROOT/extensions/chromium"
    [ -d "$ext_dir" ] || return 1
    command -v python3 >/dev/null 2>&1 || return 1
    python3 - "$ext_dir" <<'PY'
import hashlib
import sys

digest = hashlib.sha256(sys.argv[1].encode("utf-8")).hexdigest()[:16]
print("".join(chr(ord("a") + int(c, 16)) for c in digest))
PY
}

# Write the native-host manifest for a browser via the existing registration
# scripts, recording what they create so uninstall can remove it.
register_chromium_hosts() {
    local host="$BIN_DIR/xdm-native-host"
    [ ${#BROWSERS[@]} -gt 0 ] || return 0
    if ! command -v python3 >/dev/null 2>&1; then
        warn "python3 not found; skipping Chromium-family native-host registration"
        return 0
    fi
    if [ ${#EXTENSION_IDS[@]} -eq 0 ]; then
        if EXTENSION_IDS=("$(default_chromium_id)"); then
            note "derived chromium extension ID: ${EXTENSION_IDS[0]}"
        else
            warn "no --extension-id given and extensions/chromium missing; skipping Chromium-family registration"
            return 0
        fi
    fi
    local args=(--host "$host")
    local b
    for b in "${BROWSERS[@]}"; do
        args+=(--browser "$b")
    done
    for id in "${EXTENSION_IDS[@]}"; do
        args+=(--extension-id "$id")
    done
    if [ "$DRY_RUN" -eq 1 ]; then
        note "dry-run: python3 $SCRIPT_DIR/register_native_host.py ${args[*]}"
    else
        python3 "$SCRIPT_DIR/register_native_host.py" "${args[@]}"
    fi
    for b in "${BROWSERS[@]}"; do
        case "$b" in
            chrome) record "$HOME/.config/google-chrome/NativeMessagingHosts/org.xdm.browser.json" ;;
            chromium) record "$HOME/.config/chromium/NativeMessagingHosts/org.xdm.browser.json" ;;
            edge) record "$HOME/.config/microsoft-edge/NativeMessagingHosts/org.xdm.browser.json" ;;
            brave) record "$HOME/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/org.xdm.browser.json" ;;
        esac
    done
}

register_firefox_host() {
    local host="$BIN_DIR/xdm-native-host"
    if ! command -v python3 >/dev/null 2>&1; then
        warn "python3 not found; skipping Firefox native-host registration"
        return 0
    fi
    if [ "$DRY_RUN" -eq 1 ]; then
        note "dry-run: python3 $SCRIPT_DIR/register_firefox_native_host.py --host $host"
    else
        python3 "$SCRIPT_DIR/register_firefox_native_host.py" --host "$host"
    fi
    record "$HOME/.mozilla/native-messaging-hosts/org.xdm.browser.json"
}

systemctl_user() {
    if [ "${XDM_NO_SYSTEMD:-0}" = "1" ]; then
        note "XDM_NO_SYSTEMD=1; skipping systemctl --user $*"
        return 0
    fi
    if ! command -v systemctl >/dev/null 2>&1; then
        warn "systemctl not found; skipping: systemctl --user $*"
        return 0
    fi
    run systemctl --user "$@"
}

if [ -z "$SOURCE_DIR" ]; then
    if [ -f "$REPO_ROOT/bin/xdm-daemon" ]; then
        SOURCE_DIR="$REPO_ROOT/bin"
    elif [ -f "$REPO_ROOT/target/release/xdm-daemon" ]; then
        SOURCE_DIR="$REPO_ROOT/target/release"
    else
        die "no built binaries found; pass --source-dir (expected target/release or bin/)"
    fi
fi
[ -d "$SOURCE_DIR" ] || die "source dir does not exist: $SOURCE_DIR"
SOURCE_DIR=$(abs_dir "$SOURCE_DIR")
[ -f "$SOURCE_DIR/xdm-daemon" ] || die "xdm-daemon missing in $SOURCE_DIR"
[ -f "$SOURCE_DIR/xdm-native-host" ] || die "xdm-native-host missing in $SOURCE_DIR"
BIN_DIR=$(abs_dir "$BIN_DIR")

if [ ${#BROWSERS[@]} -eq 0 ]; then
    BROWSERS=(chrome chromium edge brave)
fi

ensure_dir "$BIN_DIR"
ensure_dir "$APPS_DIR"
ensure_dir "$ICON_DIR"
ensure_dir "$SYSTEMD_DIR"
ensure_dir "$STATE_DIR"

for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    if [ -f "$SOURCE_DIR/$name" ]; then
        copy_binary "$name"
    else
        warn "skipping missing binary: $name"
    fi
done

if [ -f "$SOURCE_DIR/xdm-gui" ]; then
    render_template "$SCRIPT_DIR/xdm.desktop" "$APPS_DIR/xdm.desktop" \
        "s|^Exec=.*|Exec=$BIN_DIR/xdm-gui %u|"
    record "$APPS_DIR/xdm.desktop"
else
    warn "xdm-gui not built; skipping desktop entry"
fi
run cp -f -- "$SCRIPT_DIR/xdm.svg" "$ICON_DIR/xdm.svg"
record "$ICON_DIR/xdm.svg"

render_template "$SCRIPT_DIR/xdm-daemon.service" "$SYSTEMD_DIR/xdm-daemon.service" \
    "s|@XDM_DAEMON@|$BIN_DIR/xdm-daemon|"
record "$SYSTEMD_DIR/xdm-daemon.service"
systemctl_user daemon-reload
if [ "$AUTOSTART" -eq 1 ]; then
    systemctl_user enable --now xdm-daemon.service
else
    note "daemon autostart not enabled (pass --autostart to enable xdm-daemon.service)"
fi

register_chromium_hosts
register_firefox_host

if [ "$DRY_RUN" -eq 1 ]; then
    note "dry-run complete; nothing was written"
    exit 0
fi

{
    printf '# xdm-manifest v1; paths owned by this install, one per line\n'
    printf '%s\n' "${MANIFEST_ENTRIES[@]}"
} >"$MANIFEST"
note "manifest written: $MANIFEST"
note "install complete; start the daemon with: systemctl --user start xdm-daemon.service"
