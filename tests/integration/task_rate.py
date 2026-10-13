"""Live HTTP task rate, progress/ETA, pause and persisted cap integration test."""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = hashlib.sha256(bytes(range(256)) * (8 * 1024 * 1024 // 256)).hexdigest()


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-task-rate-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *args],
                                    env=env, capture_output=True, text=True, timeout=5)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout)

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def task():
            return cli("list")["tasks"][0]

        def wait_for(predicate, timeout=10):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                current = task()
                if predicate(current):
                    return current
                time.sleep(0.1)
            raise AssertionError(current)

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            daemon = start()
            cli("queue", "stop")
            output = base / "limited.bin"
            task_id = str(cli("submit", f"http://127.0.0.1:{port}/limited.bin", str(output))["task_id"])
            cli("rate", task_id, "512")
            cli("rate", task_id, "0", success=False)
            cli("queue", "start")
            wait_for(lambda row: row["status"] == "running" and (row["downloaded"] or 0) > 0)
            time.sleep(1.5)
            limited = task()
            assert limited["total"] == 8 * 1024 * 1024
            assert 0 < limited["downloaded"] < 2 * 1024 * 1024, limited
            assert 0 < limited["bytes_per_second"] < 900 * 1024, limited
            assert limited["eta_seconds"] > 0, limited
            cli("rate", task_id, "256")
            assert task()["rate_limit"] == 256 * 1024
            cli("pause", task_id)
            wait_for(lambda row: row["status"] == "paused")
            assert Path(str(output) + ".part.meta").exists()
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert task()["rate_limit"] == 256 * 1024 and task()["status"] == "paused"
            cli("resume", task_id)
            wait_for(lambda row: row["status"] == "running")
            cli("rate", task_id, "unlimited")
            complete = wait_for(lambda row: row["status"] == "complete")
            assert complete["rate_limit"] is None and complete["bytes_per_second"] is None
            assert hashlib.sha256(output.read_bytes()).hexdigest() == EXPECTED
            print("live progress/ETA, dynamic cap, pause, persisted cap, unlimited resume and hash passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
