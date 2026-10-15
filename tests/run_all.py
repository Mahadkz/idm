#!/usr/bin/env python3
"""Run every integration suite in tests/integration with a per-suite timeout.

Usage: python3 tests/run_all.py [--only NAME...] [--skip NAME...] [--timeout SECONDS] [--list]

Run after `cargo build --workspace`. A suite whose prerequisite is missing is reported as
SKIP with the reason (never as a pass). Exit status is non-zero if any suite FAILs or times
out; with --strict, SKIPs also fail (use on CI runners that must have every prerequisite).
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUITES = ROOT / "tests/integration"
# Helpers imported by suites or fixtures, not suites themselves.
HELPERS = {"http_fixture.py", "proxy_fixture.py"}
WINDOWS = os.name == "nt"
EXE = ".exe" if WINDOWS else ""


def extended_path():
    extra = [str(Path.home() / ".local/bin")]
    return os.pathsep.join([os.environ.get("PATH", ""), *extra])


def find(binary):
    return shutil.which(binary, path=extended_path())


def find_chrome():
    configured = os.environ.get("XDM_CHROME_FOR_TESTING")
    return configured if configured and Path(configured).exists() else None


def prerequisite_problem(path, text):
    """Return a SKIP reason, or None when the suite can run here."""
    if path.suffix == ".mjs" and not find("node"):
        return "node not found on PATH"
    if "AF_UNIX" in text and WINDOWS:
        return "suite talks to the daemon over a Unix socket; Windows uses a named pipe (no Python client yet)"
    if "Xvfb" in text and (WINDOWS or not find("Xvfb")):
        return "Xvfb not available (Linux with xvfb installed required)"
    if re.search(r"ffmpeg|ffprobe", text) and not (find("ffmpeg") and find("ffprobe")):
        return "ffmpeg/ffprobe not found on PATH"
    if path.name == "chromium_real.mjs" and not find_chrome():
        return "XDM_CHROME_FOR_TESTING unset or missing (needs Chrome for Testing; see .github/workflows/browsers.yml)"
    if path.name == "firefox_real.mjs" and not (os.environ.get("XDM_FIREFOX") and Path(os.environ["XDM_FIREFOX"]).exists()):
        return "XDM_FIREFOX unset or missing (needs Firefox)"
    return None


def discover():
    found = [p for p in SUITES.iterdir() if p.suffix in (".py", ".mjs") and p.name not in HELPERS]
    return sorted(found, key=lambda p: p.name)


def run_suite(path, timeout, env):
    command = ["node", str(path)] if path.suffix == ".mjs" else [sys.executable, str(path)]
    started = time.monotonic()
    try:
        done = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True,
                              timeout=timeout, errors="replace")
        status = "PASS" if done.returncode == 0 else "FAIL"
        output = done.stdout + done.stderr
    except subprocess.TimeoutExpired as error:
        status = "TIMEOUT"
        output = "".join(x.decode(errors="replace") if isinstance(x, bytes) else (x or "") for x in (error.stdout, error.stderr))
    return status, time.monotonic() - started, output


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", nargs="+", default=[], metavar="NAME", help="run only these suites (name or stem)")
    parser.add_argument("--skip", nargs="+", default=[], metavar="NAME", help="skip these suites (name or stem)")
    parser.add_argument("--timeout", type=int, default=600, help="per-suite timeout in seconds (default 600)")
    parser.add_argument("--strict", action="store_true", help="treat prerequisite SKIPs as failures")
    parser.add_argument("--list", action="store_true", help="list suites and exit")
    args = parser.parse_args()

    suites = discover()
    names = {p.name for p in suites} | {p.stem for p in suites}
    unknown = [n for n in args.only + args.skip if n not in names]
    if unknown:
        parser.error(f"unknown suite(s): {', '.join(unknown)}")
    if args.list:
        print("\n".join(p.name for p in suites))
        return 0

    for binary in ("xdm-daemon", "xdm-cli", "xdm-native-host"):
        if not (ROOT / "target/debug" / (binary + EXE)).exists():
            print(f"error: target/debug/{binary}{EXE} missing; run cargo build --workspace first", file=sys.stderr)
            return 2

    env = os.environ.copy()
    env["PATH"] = extended_path()
    results = []
    for path in suites:
        selected = (not args.only or path.name in args.only or path.stem in args.only)
        if not selected:
            continue
        if path.name in args.skip or path.stem in args.skip:
            results.append((path.name, "SKIP", 0.0, "skipped by --skip"))
            continue
        reason = prerequisite_problem(path, path.read_text(errors="replace"))
        if reason:
            results.append((path.name, "SKIP", 0.0, reason))
            print(f"SKIP {path.name}: {reason}", flush=True)
            continue
        print(f"RUN  {path.name} ...", flush=True)
        status, elapsed, output = run_suite(path, args.timeout, env)
        detail = "" if status == "PASS" else f"exceeded {args.timeout}s" if status == "TIMEOUT" else "non-zero exit"
        results.append((path.name, status, elapsed, detail))
        print(f"{status} {path.name} ({elapsed:.1f}s)", flush=True)
        if status != "PASS":
            print("---- output tail ----\n" + "\n".join(output.splitlines()[-40:]) + "\n---------------------", flush=True)

    width = max((len(r[0]) for r in results), default=10)
    print("\n" + "suite".ljust(width), "status ", "time    detail")
    for name, status, elapsed, detail in results:
        print(name.ljust(width), status.ljust(7), f"{elapsed:6.1f}s", detail)
    counts = {s: sum(1 for r in results if r[1] == s) for s in ("PASS", "FAIL", "TIMEOUT", "SKIP")}
    print("\n" + ", ".join(f"{v} {k}" for k, v in counts.items()))
    bad = counts["FAIL"] + counts["TIMEOUT"] + (counts["SKIP"] if args.strict else 0)
    if not any(r[1] == "PASS" for r in results) and results:
        bad += 1  # nothing actually ran: never report success
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
