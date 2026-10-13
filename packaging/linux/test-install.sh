#!/usr/bin/env bash
# Exercise install.sh/uninstall.sh against a disposable HOME with fake
# binaries. Never touches the real HOME, real systemd, or browser profiles.
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd)
INSTALL_DIR="$REPO_ROOT/install/linux"

TMP=$(mktemp -d)
cleanup() { rm -rf -- "$TMP"; }
trap cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

pass() { echo "ok: $*"; }

assert_file() { [ -f "$1" ] || fail "expected file: $1"; }
assert_no_path() { [ ! -e "$1" ] && [ ! -L "$1" ] || fail "expected absent: $1"; }
assert_grep() { grep -q -- "$2" "$1" || fail "expected '$2' in $1"; }

for script in "$INSTALL_DIR/install.sh" "$INSTALL_DIR/uninstall.sh" \
    "$REPO_ROOT/packaging/linux/build-tarball.sh"; do
    bash -n "$script" || fail "bash -n $script"
done
pass "bash -n syntax checks"

if command -v pwsh >/dev/null 2>&1; then
    for ps in "$REPO_ROOT"/install/windows/*.ps1; do
        pwsh -NoProfile -NonInteractive -Command '
            $errs = $null
            [System.Management.Automation.Language.Parser]::ParseFile($args[0], [ref]$null, [ref]$errs) | Out-Null
            if ($errs) { $errs | ForEach-Object { Write-Error $_ }; exit 1 }
        ' "$ps" || fail "pwsh parse $ps"
    done
    pass "pwsh parse checks"
else
    echo "note: pwsh not installed; Windows scripts are untested"
fi

# A stub systemctl that records any invocation so we can prove that
# XDM_NO_SYSTEMD=1 never reaches systemd.
mkdir -p -- "$TMP/stub-bin"
SYSTEMCTL_LOG="$TMP/systemctl.log"
cat >"$TMP/stub-bin/systemctl" <<EOF
#!/usr/bin/env bash
echo "systemctl \$*" >>"$SYSTEMCTL_LOG"
exit 0
EOF
chmod +x -- "$TMP/stub-bin/systemctl"
export PATH="$TMP/stub-bin:$PATH"

HOME_DIR="$TMP/home"
SRC="$TMP/src"
mkdir -p -- "$HOME_DIR" "$SRC"
for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"$SRC/$name"
    chmod +x -- "$SRC/$name"
done

export HOME="$HOME_DIR"
export XDM_NO_SYSTEMD=1
unset XDG_DATA_HOME XDG_STATE_HOME XDG_CONFIG_HOME XDG_CACHE_HOME || true

EXT_ID="abcdefghijklmnopabcdefghijklmnop"
install_args=(--source-dir "$SRC" --autostart --extension-id "$EXT_ID")

bash "$INSTALL_DIR/install.sh" "${install_args[@]}" >/dev/null || fail "install.sh exited nonzero"
pass "install.sh runs"

BIN="$HOME_DIR/.local/bin"
APPS="$HOME_DIR/.local/share/applications/xdm.desktop"
ICON="$HOME_DIR/.local/share/icons/hicolor/scalable/apps/xdm.svg"
UNIT="$HOME_DIR/.config/systemd/user/xdm-daemon.service"
MANIFEST="$HOME_DIR/.local/state/xdm/install-manifest"
DATA_DIR="$HOME_DIR/.local/share/xdm"

for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    assert_file "$BIN/$name"
    [ -x "$BIN/$name" ] || fail "$BIN/$name not executable"
done
assert_file "$APPS"
assert_grep "$APPS" "Exec=$BIN/xdm-gui"
assert_file "$ICON"
assert_file "$UNIT"
assert_grep "$UNIT" "ExecStart=$BIN/xdm-daemon"
assert_file "$HOME_DIR/.config/chromium/NativeMessagingHosts/org.xdm.browser.json"
assert_grep "$HOME_DIR/.config/chromium/NativeMessagingHosts/org.xdm.browser.json" "$BIN/xdm-native-host"
assert_file "$HOME_DIR/.config/google-chrome/NativeMessagingHosts/org.xdm.browser.json"
assert_file "$HOME_DIR/.mozilla/native-messaging-hosts/org.xdm.browser.json"
assert_grep "$HOME_DIR/.mozilla/native-messaging-hosts/org.xdm.browser.json" "xdm@xdm.local"
assert_file "$MANIFEST"
pass "installed files, desktop entry, unit, and browser hosts"

while read -r line; do
    case "$line" in '' | '#'*) continue ;; esac
    [ -e "$line" ] || fail "manifest path missing: $line"
done <"$MANIFEST"
pass "manifest paths exist"

bash "$INSTALL_DIR/install.sh" "${install_args[@]}" >/dev/null || fail "re-install exited nonzero"
pass "re-run is idempotent"

DRY_HOME="$TMP/dry-home"
mkdir -p -- "$DRY_HOME"
HOME="$DRY_HOME" bash "$INSTALL_DIR/install.sh" --source-dir "$SRC" --dry-run >/dev/null \
    || fail "dry-run install exited nonzero"
assert_no_path "$DRY_HOME/.local"
pass "--dry-run install writes nothing"

mkdir -p -- "$DATA_DIR"
printf 'db\n' >"$DATA_DIR/xdm.sqlite3"
mkdir -p -- "$HOME_DIR/Downloads"
printf 'bytes\n' >"$HOME_DIR/Downloads/file.bin"

bash "$INSTALL_DIR/uninstall.sh" --dry-run >/dev/null || fail "dry-run uninstall exited nonzero"
assert_file "$BIN/xdm-daemon"
pass "--dry-run uninstall removes nothing"

bash "$INSTALL_DIR/uninstall.sh" >/dev/null || fail "uninstall.sh exited nonzero"
for name in xdm-cli xdm-daemon xdm-gui xdm-native-host; do
    assert_no_path "$BIN/$name"
done
assert_no_path "$APPS"
assert_no_path "$ICON"
assert_no_path "$UNIT"
assert_no_path "$MANIFEST"
assert_no_path "$HOME_DIR/.config/chromium/NativeMessagingHosts/org.xdm.browser.json"
assert_no_path "$HOME_DIR/.mozilla/native-messaging-hosts/org.xdm.browser.json"
assert_file "$DATA_DIR/xdm.sqlite3"
assert_file "$HOME_DIR/Downloads/file.bin"
pass "uninstall removes installed files, keeps data dir and downloads"

bash "$INSTALL_DIR/install.sh" "${install_args[@]}" >/dev/null || fail "second install exited nonzero"
bash "$INSTALL_DIR/uninstall.sh" --purge >/dev/null || fail "purge uninstall exited nonzero"
assert_no_path "$DATA_DIR"
assert_file "$HOME_DIR/Downloads/file.bin"
pass "--purge removes the data dir but not downloads"

if [ -s "$SYSTEMCTL_LOG" ]; then
    fail "systemctl was invoked despite XDM_NO_SYSTEMD=1: $(cat "$SYSTEMCTL_LOG")"
fi
pass "no systemctl calls under XDM_NO_SYSTEMD=1"

echo "all install/uninstall tests passed"
