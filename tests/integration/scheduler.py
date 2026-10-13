"""Persistent one-shot queue-start and manual queue-control integration test."""

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"


def command(env, *args):
    result = subprocess.run([str(CLI), "daemon", *args], env=env, capture_output=True, text=True, timeout=5)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def start_daemon(env):
    process = subprocess.Popen([str(DAEMON)], env=env, stderr=subprocess.PIPE, text=True)
    assert "listening locally" in process.stderr.readline()
    return process


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-schedule-") as directory:
        base = Path(directory)
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.15"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None
        try:
            port = json.loads(fixture.stdout.readline())["port"]
            env = os.environ.copy()
            env["XDM_DATA_DIR"] = str(base / "state")
            daemon = start_daemon(env)
            command(env, "queue", "stop")
            output = base / "later.bin"
            command(env, "submit", f"http://127.0.0.1:{port}/queue-a.bin", str(output))
            first = command(env, "list")
            assert first["queue_running"] is False and first["tasks"][0]["status"] == "queued"
            at = int(time.time()) + 4
            command(env, "queue", "at", str(at))
            assert command(env, "list")["queue_start_at"] == at
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start_daemon(env)
            assert command(env, "list")["tasks"][0]["status"] == "queued"
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                state = command(env, "list")
                if state["tasks"][0]["status"] == "complete":
                    break
                assert state["tasks"][0]["status"] in ("queued", "running"), state
                time.sleep(0.15)
            assert state["queue_running"] is True and state["queue_start_at"] is None
            assert state["tasks"][0]["status"] == "complete" and output.stat().st_size == 8 * 1024 * 1024
            # Arming a future start must not stop running downloads: IDM arms
            # the timer and leaves active transfers alone, so a task that is
            # already downloading keeps running through `queue at`. The old
            # stop-everything-now behavior is deliberately no longer asserted.
            armed = base / "armed.bin"
            command(env, "submit", f"http://127.0.0.1:{port}/queue-armed.bin", str(armed))
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and command(env, "list")["tasks"][0]["status"] != "running":
                time.sleep(0.1)
            assert command(env, "list")["tasks"][0]["status"] == "running"
            at = int(time.time()) + 2
            command(env, "queue", "at", str(at))
            state = command(env, "list")
            assert state["queue_start_at"] == at and state["queue_running"] is True
            assert state["tasks"][0]["status"] == "running", state
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                state = command(env, "list")
                if state["tasks"][0]["status"] == "complete":
                    break
                time.sleep(0.1)
            assert state["tasks"][0]["status"] == "complete" and armed.stat().st_size == 8 * 1024 * 1024
            assert state["queue_start_at"] is None and state["queue_running"] is True
            command(env, "queue", "stop")
            assert command(env, "list")["queue_running"] is False
            command(env, "queue", "start")
            assert command(env, "list")["queue_running"] is True
            interrupted = base / "interrupted.bin"
            command(env, "submit", f"http://127.0.0.1:{port}/queue-b.bin", str(interrupted))
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and command(env, "list")["tasks"][0]["status"] != "running":
                time.sleep(0.1)
            assert command(env, "list")["tasks"][0]["status"] == "running"
            stop_at = int(time.time()) + 2
            command(env, "queue", "stop-at", str(stop_at))
            assert command(env, "list")["queue_stop_at"] == stop_at
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                stopped = command(env, "list")
                if stopped["queue_running"] is False and stopped["tasks"][0]["status"] == "queued":
                    break
                time.sleep(0.1)
            assert stopped["queue_running"] is False and stopped["tasks"][0]["status"] == "queued", stopped
            assert not interrupted.exists() and Path(str(interrupted) + ".part.meta").exists()
            command(env, "queue", "start")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                resumed = command(env, "list")
                if resumed["tasks"][0]["status"] == "complete":
                    break
                time.sleep(0.1)
            assert resumed["tasks"][0]["status"] == "complete" and interrupted.stat().st_size == 8 * 1024 * 1024
            print("persisted timed start, timed stop, active checkpoint, resume, and manual controls passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
