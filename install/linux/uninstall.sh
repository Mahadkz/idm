#!/usr/bin/env bash
# Remove a user-level XDM installation using the manifest written by install.sh.
# Removes exactly the recorded paths. User downloads and the daemon data
# directory survive unless --purge is given.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: uninstall.sh [options]

  --dry-run   Print what would be removed without removing anything
  --purge     Also delete the XDM data directory (task database, tokens).
              Downloaded files are never touched.
  -h, --help  Show this help

Environment:
  XDM_NO_SYSTEMD=1     Never invoke systemctl (used by tests and sandboxes)
  XDG_*                Follow the XDG base directory spec as usual
EOF
}

note() { printf '[uninstall] %s\n' "$*"; }
warn() { printf '[uninstall] warning: %s\n' "$*" >&2; }

die() {
    printf '[uninstall] error: %s\n' "$*" >&2
    exit 1
}

DRY_RUN=0
PURGE=0

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --purge) PURGE=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done

[ -n "${HOME:-}" ] || die "HOME is not set"

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/xdm"
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/xdm"
SYSTEMD_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
MANIFEST="$STATE_DIR/install-manifest"

remove_path() {
    local path=$1
    if [ ! -e "$path" ] && [ ! -L "$path" ]; then
        return 0
    fi
    if [ -d "$path" ] && [ ! -L "$path" ]; then
        note "remove directory $path (if empty)"
        if [ "$DRY_RUN" -eq 0 ]; then
            rmdir -- "$path" 2>/dev/null || note "kept non-empty directory: $path"
        fi
    else
        note "remove $path"
        [ "$DRY_RUN" -eq 1 ] || rm -f -- "$path"
    fi
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
    if [ "$DRY_RUN" -eq 1 ]; then
        note "dry-run: systemctl --user $*"
        return 0
    fi
    systemctl --user "$@"
}

[ -f "$MANIFEST" ] || die "no install manifest at $MANIFEST; nothing to uninstall"

mapfile -t ENTRIES < <(grep -v '^#' "$MANIFEST" | grep -v '^[[:space:]]*$' || true)

# Stop the daemon before its unit file disappears, then drop unit symlinks.
if printf '%s\n' "${ENTRIES[@]}" | grep -qxF "$SYSTEMD_DIR/xdm-daemon.service"; then
    systemctl_user disable --now xdm-daemon.service 2>/dev/null || true
fi

# The manifest is the newest install artifact and already read into memory,
# so drop it first. The rest lists directories before the files inside them;
# walk it in reverse so files go first and recorded directories empty out.
remove_path "$MANIFEST"
for ((i = ${#ENTRIES[@]} - 1; i >= 0; i--)); do
    remove_path "${ENTRIES[$i]}"
done

if [ "$PURGE" -eq 1 ]; then
    if [ -d "$DATA_DIR" ]; then
        note "purge data directory $DATA_DIR"
        if [ "$DRY_RUN" -eq 0 ]; then
            rm -rf -- "$DATA_DIR"
        fi
    fi
else
    if [ -d "$DATA_DIR" ]; then
        note "kept data directory $DATA_DIR (use --purge to delete it)"
    fi
fi

[ "$DRY_RUN" -eq 1 ] || systemctl_user daemon-reload
if [ "$DRY_RUN" -eq 1 ]; then
    note "dry-run complete; nothing was removed"
else
    note "uninstall complete"
fi
