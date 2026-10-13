#!/usr/bin/env python3
"""Register the XDM native host for selected Chromium-family browser profiles."""
import argparse
import json
import os
import re
from pathlib import Path

DIRECTORIES = {
    "chrome": ".config/google-chrome/NativeMessagingHosts",
    "chromium": ".config/chromium/NativeMessagingHosts",
    "edge": ".config/microsoft-edge/NativeMessagingHosts",
    "brave": ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
}

parser = argparse.ArgumentParser()
parser.add_argument("--host", type=Path, required=True, help="Absolute built xdm-native-host binary")
parser.add_argument("--extension-id", action="append", required=True, help="Installed extension ID; repeat for multiple stores")
parser.add_argument("--browser", choices=DIRECTORIES, action="append", required=True)
parser.add_argument("--dry-run", action="store_true")
args = parser.parse_args()
host = args.host.resolve(strict=True)
if not host.is_file() or not os.access(host, os.X_OK):
    parser.error("--host must name an executable file")
if any(not re.fullmatch(r"[a-p]{32}", identifier) for identifier in args.extension_id):
    parser.error("each extension ID must be 32 lowercase letters a-p")

manifest = {
    "name": "org.xdm.browser",
    "description": "XDM native messaging bridge",
    "path": str(host),
    "type": "stdio",
    "allowed_origins": [f"chrome-extension://{identifier}/" for identifier in sorted(set(args.extension_id))],
}

for browser in dict.fromkeys(args.browser):
    path = Path.home() / DIRECTORIES[browser] / "org.xdm.browser.json"
    print(f"{browser}: {path}")
    if args.dry_run:
        continue
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)
