"""Range-corruption parking in restart_required, TaskRestart from byte zero, the stall-timeout setting and its persistence."""
import hashlib
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
FIXTURE = Path(__file__).resolve().parent / "http_fixture.py"
SIZE = 4 * 1024 * 1024
BLOCK = bytes(range(256)) * 256
EXPECTED = hashlib.sha256(BLOCK * (SIZE // len(BLOCK))).hexdigest()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-restart-required-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}

        def spawn_fixture(name):
            log = base / f"{name}.jsonl"
            server = subprocess.Popen([sys.executable, str(FIXTURE), log, "--delay", "0.005"],
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            port = json.loads(server.stdout.readline())["port"]
            return server, log, f"http://127.0.0.1:{port}"

        direct_server, _, direct_url = spawn_fixture("direct")
        daemon_server, daemon_log, daemon_url = spawn_fixture("daemon")
        daemon = None

        def run(*args, success=True):
            result = subprocess.run([str(CLI), *map(str, args)], env=env, text=True,
                                    capture_output=True, timeout=90)
            assert (result.returncode == 0) == success, (args, result.returncode, result.stderr)
            return result

        def cli(*args, success=True):
            result = subprocess.run([str(CLI), "daemon", *map(str, args)], env=env, text=True,
                                    capture_output=True, timeout=10)
            assert (result.returncode == 0) == success, (args, result.returncode, result.stderr + result.stdout)
            return json.loads(result.stdout) if result.stdout.strip() else None

        def start():
            process = subprocess.Popen([str(DAEMON)], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def info(task_id):
            return cli("info", task_id)["tasks"][0]

        def wait_for(task_id, predicate, timeout=60):
            deadline = time.monotonic() + timeout
            row = None
            while time.monotonic() < deadline:
                row = info(task_id)
                if predicate(row):
                    return row
                time.sleep(0.1)
            raise AssertionError(row)

        def daemon_requests(path):
            return [json.loads(line) for line in daemon_log.read_text().splitlines()
                    if json.loads(line)["path"] == path]

        try:
            # Direct download: --stall-timeout 1 abandons a stalled connection in
            # about a second instead of the 15 s default.
            out = base / "direct-stall.bin"
            started = time.monotonic()
            run(f"{direct_url}/stall.bin", out, "--stall-timeout", "1")
            elapsed = time.monotonic() - started
            assert digest(out) == EXPECTED, "stalled direct download lost data"
            assert elapsed < 10, f"1 s stall timeout abandoned too late: {elapsed:.1f}s"
            run(f"{direct_url}/stall.bin", base / "never0.bin", "--stall-timeout", "0", success=False)
            run(f"{direct_url}/stall.bin", base / "never601.bin", "--stall-timeout", "601", success=False)
            assert not (base / "never0.bin").exists()

            daemon = start()

            # Setting: default, validation and the stored value.
            assert cli("stall-timeout", "get")["stall_timeout_seconds"] == 15
            cli("stall-timeout", "0", success=False)
            cli("stall-timeout", "601", success=False)
            assert cli("stall-timeout", "1")["stall_timeout_seconds"] == 1
            assert cli("stall-timeout", "get")["stall_timeout_seconds"] == 1

            # Infinite queue retries must never loop over a restart-required task.
            cli("queue", "retry", 1, "infinite", "0")
            flip = cli("submit", f"{daemon_url}/flip.bin", base / "flip.bin")["task_id"]
            row = wait_for(flip, lambda task: task["status"] == "restart_required")
            assert row["restart_reason"] and "byte ranges" in row["restart_reason"], row
            assert row["error"] and "stopped honoring byte ranges" in row["error"], row
            assert (base / "flip.bin.part").exists(), "restart parking must keep the partial data"
            time.sleep(1.5)
            row = info(flip)
            assert row["status"] == "restart_required" and row["retry_count"] == 0, row

            # TaskRestart deletes the task's own partial data and requeues it.
            before = len(daemon_requests("/flip.bin"))
            assert cli("restart", flip)["ok"]
            row = wait_for(flip, lambda task: task["status"] == "complete")
            assert digest(base / "flip.bin") == EXPECTED
            after = daemon_requests("/flip.bin")[before:]
            assert after and after[0]["range"] == "bytes=0-", after
            assert row["restart_reason"] is None and row["error"] is None, row
            cli("restart", flip, success=False)
            cli("restart", 999, success=False)

            # The daemon-wide 1 s stall timeout carries every HTTP task through a stall.
            stall = cli("submit", f"{daemon_url}/stall.bin", base / "stall.bin")["task_id"]
            started = time.monotonic()
            wait_for(stall, lambda task: task["status"] == "complete")
            elapsed = time.monotonic() - started
            assert digest(base / "stall.bin") == EXPECTED
            assert elapsed < 10, f"1 s daemon stall timeout abandoned too late: {elapsed:.1f}s"

            # The setting persists across a daemon restart.
            daemon.terminate()
            daemon.wait(timeout=5)
            daemon = start()
            assert cli("stall-timeout", "get")["stall_timeout_seconds"] == 1
            assert info(flip)["status"] == "complete"

            print(json.dumps({"result": "passed", "cases": [
                "direct --stall-timeout recovery and bounds",
                "stall-timeout setting default, bounds and value",
                "flip.bin parks in restart_required with reason",
                "restart policy never loops on restart_required",
                "TaskRestart requeues from byte zero",
                "daemon stall timeout applied to tasks",
                "setting persists across daemon restart",
            ]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            for server in [direct_server, daemon_server]:
                server.terminate()
                server.wait(timeout=5)


if __name__ == "__main__":
    main()
