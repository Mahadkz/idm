"""Production-hardening contract: versions, handshake, logging, single instance,
graceful SIGTERM checkpoint + resume, diagnostics bundle, permissions.
Run after cargo build --workspace."""

import hashlib
import json
import os
import signal
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / "target/debug"
EXPECTED = hashlib.sha256(bytes(range(256)) * (8 * 1024 * 1024 // 256)).hexdigest()


def run(*args, env=None, check=True):
    result = subprocess.run([str(a) for a in args], env=env, text=True, capture_output=True, timeout=30)
    if check:
        assert result.returncode == 0, result.stderr
    return result


def start_daemon(env):
    daemon = subprocess.Popen([str(BIN / "xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
    assert "listening locally" in daemon.stderr.readline()
    return daemon


def tasks(env):
    return json.loads(run(BIN / "xdm-cli", "daemon", "list", env=env).stdout)["tasks"]


def main():
    for name in ["xdm-cli", "xdm-daemon", "xdm-native-host", "xdm-gui"]:
        if (BIN / name).exists():
            out = run(BIN / name, "--version").stdout
            assert out.startswith(name + " 0.") and "commit " in out and "protocol" in out, out

    with tempfile.TemporaryDirectory(prefix="xdm-ops-") as temp:
        base = Path(temp)
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"), str(base / "http"), "--delay", "0.05"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        daemon = None
        try:
            port = json.loads(fixture.stdout.readline())["port"]
            state = base / "state"
            env = os.environ.copy()
            env.update({"XDM_DATA_DIR": str(state), "HOME": str(base / "home"), "XDM_LOG": "debug"})
            (base / "home").mkdir()
            daemon = start_daemon(env)

            # Permissions.
            assert stat.S_IMODE(state.stat().st_mode) == 0o700
            assert stat.S_IMODE((state / "ipc-token").stat().st_mode) == 0o600

            # Handshake.
            assert "protocol 1" in run(BIN / "xdm-cli", "daemon", "version", env=env).stdout

            # Single instance.
            second = run(BIN / "xdm-daemon", env=env, check=False)
            assert second.returncode != 0 and "already running" in second.stderr, second.stderr

            # Slow download, then SIGTERM mid-transfer.
            secret_url = f"http://127.0.0.1:{port}/queue-a.bin?token=SECRETQUERY"
            out_file = base / "home/big.bin"
            run(BIN / "xdm-cli", "daemon", "submit", secret_url, out_file, env=env)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if tasks(env)[0]["status"] == "running" and any((base / "home").glob("big.bin.part*")):
                    break
                time.sleep(0.1)
            time.sleep(0.5)
            start = time.monotonic()
            daemon.send_signal(signal.SIGTERM)
            assert daemon.wait(timeout=20) == 0
            assert time.monotonic() - start < 16
            assert not (state / "daemon.sock").exists()
            with sqlite3.connect(state / "tasks.sqlite3") as db:
                status = db.execute("SELECT status FROM tasks").fetchone()[0]
            assert status == "queued", status
            assert not out_file.exists() and list((base / "home").glob("big.bin.part*")), "checkpoint kept"

            # Lock file is stale now; restart resumes to completion.
            daemon = start_daemon(env)
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline and tasks(env)[0]["status"] != "complete":
                time.sleep(0.2)
            assert tasks(env)[0]["status"] == "complete"
            assert hashlib.sha256(out_file.read_bytes()).hexdigest() == EXPECTED

            # Logging: file exists, no secrets.
            log = (state / "logs/xdm-daemon.log").read_text()
            assert "starting xdm-daemon" in log and "shutdown complete" in log
            assert "SECRETQUERY" not in log

            # Diagnostics bundle.
            bundle = base / "diag.zip"
            run(BIN / "xdm-cli", "daemon", "diagnostics", bundle, env=env)
            with zipfile.ZipFile(bundle) as z:
                assert z.testzip() is None
                names = z.namelist()
                blob = b"".join(z.read(n) for n in names).decode()
            assert {"version.txt", "database.txt", "settings.txt", "tasks.txt", "logs/xdm-daemon.log"} <= set(names), names
            assert "quick_check: ok" in blob and "SECRETQUERY" not in blob and "/queue-a.bin" in blob
            run(BIN / "xdm-cli", "daemon", "diagnostics", base / "diag_dir", env=env)
            assert (base / "diag_dir/version.txt").exists()

            daemon.terminate(); daemon.wait(timeout=20)
            print(json.dumps({"result": "passed", "cases": ["--version", "permissions", "handshake", "single instance", "SIGTERM checkpoint+resume", "stale lock", "log redaction", "diagnostics zip/dir"]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.kill()
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
