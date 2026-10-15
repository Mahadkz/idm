#!/usr/bin/env bash
# Build xdm-chromium-<v>.zip and xdm-firefox-<v>.xpi with the workspace version stamped in.
set -euo pipefail
. "$(dirname -- "${BASH_SOURCE[0]}")/../common.sh"
mkdir -p "$DIST"
STAGE=$(mktemp -d); trap 'rm -rf "$STAGE"' EXIT
python3 -I - "$REPO_ROOT/extensions" "$STAGE" "$VERSION" "$(cat "$PK_DIR/extensions/chromium-key.b64")" <<'PY'
import json, shutil, sys, re
from pathlib import Path
src, stage, version, key = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3], sys.argv[4]
assert re.fullmatch(r"\d+(\.\d+){0,3}", version), f"bad extension version {version}"
for browser, mv, req in (("chromium", 3, ["name", "permissions", "background"]), ("firefox", 2, ["name", "permissions", "background", "browser_specific_settings"])):
    dst = stage / browser
    shutil.copytree(src / browser, dst)
    mf = dst / "manifest.json"
    data = json.loads(mf.read_text())
    for k in ["manifest_version", "version"] + req:
        assert k in data, f"{browser} manifest missing {k}"
    assert data["manifest_version"] == mv, f"{browser} manifest_version"
    data["version"] = version
    if browser == "chromium":
        data["key"] = key  # stable extension ID for native-host allowed_origins
    mf.write_text(json.dumps(data, indent=2) + "\n")
    for rel in [data.get("background", {}).get("service_worker")] + data.get("background", {}).get("scripts", []):
        if rel:
            assert (dst / rel).is_file(), f"{browser}: background file {rel} missing"
print("manifests valid")
PY
mk() { # dir out
    out="$DIST/$2"; rm -f "$out"
    (cd "$1" && find . -type f | LC_ALL=C sort | sed 's|^\./||' | TZ=UTC zip -X -q -9 "$out" -@)
    say "built $out"
}
find "$STAGE" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
mk "$STAGE/chromium" "xdm-chromium-$VERSION.zip"
mk "$STAGE/firefox" "xdm-firefox-$VERSION.xpi"
if [ -z "${XDM_SKIP_WEBEXT:-}" ]; then
    if timeout 60 npx --offline --no-install web-ext --version >/dev/null 2>&1; then
        npx --offline --no-install web-ext lint -s "$STAGE/firefox" || say "web-ext lint reported problems"
    else
        say "web-ext lint skipped: web-ext not in offline npx cache"
    fi
fi
