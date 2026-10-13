#!/usr/bin/env python3
"""Register the XDM native host for Firefox (allowed_extensions, not allowed_origins)."""
import argparse
import json
import os
import re
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--host", type=Path, required=True, help="Absolute built xdm-native-host binary")
parser.add_argument("--extension-id", action="append", default=None, help="Firefox add-on ID (default xdm@xdm.local)")
parser.add_argument("--dry-run", action="store_true")
args = parser.parse_args()
host = args.host.resolve(strict=True)
if not host.is_file() or not os.access(host, os.X_OK):
    parser.error("--host must name an executable file")
identifiers = sorted(set(args.extension_id or ["xdm@xdm.local"]))
if any(not re.fullmatch(r"[A-Za-z0-9._+-]+@[A-Za-z0-9._-]+|\{[0-9a-fA-F-]{36}\}", identifier) for identifier in identifiers):
    parser.error("each extension ID must be an email-style or {UUID} add-on ID")

manifest = {
    "name": "org.xdm.browser",
    "description": "XDM native messaging bridge",
    "path": str(host),
    "type": "stdio",
    "allowed_extensions": identifiers,
}

path = Path.home() / ".mozilla/native-messaging-hosts/org.xdm.browser.json"
print(f"firefox: {path}")
if not args.dry_run:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)
