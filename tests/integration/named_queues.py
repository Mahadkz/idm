"""Named queue limits, isolated stop/checkpoint, atomic moves, restart and priority."""
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
    with tempfile.TemporaryDirectory(prefix="xdm-named-queues-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=5)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout)

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def until(predicate, timeout=15):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                state = cli("list")
                if predicate(state):
                    return state
                time.sleep(0.1)
            raise AssertionError(state)

        def queue(state, queue_id):
            return next(row for row in state["queues"] if row["id"] == queue_id)

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            daemon = start()
            cli("queue", "stop")
            cli("limit", 3)
            a = cli("queue", "create", "Documents", 1)["queue_id"]
            b = cli("queue", "create", "Video", 2)["queue_id"]
            assert not queue(cli("list"), a)["running"]
            cli("queue", "create", "video", success=False)
            cli("queue", "create", "Invalid", 0, success=False)
            cli("queue", "remove", 1, success=False)
            cli("queue", "rename", 1, "Other", success=False)
            groups = {}
            for queue_id, label in [(a, "a"), (b, "b")]:
                groups[queue_id] = []
                for index in range(3):
                    task_id = cli("submit", f"http://127.0.0.1:{port}/{label}-{index}.bin", base / f"{label}-{index}.bin")["task_id"]
                    cli("rate", task_id, 512)
                    groups[queue_id].append(task_id)
                cli("queue", "assign", queue_id, *groups[queue_id])
                cli("queue", "enable", queue_id)
            cli("queue", "start")
            state = until(lambda state: queue(state, a)["active"] == 1 and queue(state, b)["active"] == 2)
            assert sum(row["active"] for row in state["queues"]) == 3
            cli("queue", "remove", a, success=False)
            active_b = [row for row in state["tasks"] if row["queue_id"] == b and row["status"] == "running"]
            cli("queue", "assign", a, active_b[0]["id"], success=False)
            cli("queue", "assign", b, groups[a][2], 99999999, success=False)
            assert cli("info", groups[a][2])["tasks"][0]["queue_id"] == a
            cli("queue", "disable", a)
            stopped = until(lambda state: queue(state, a)["active"] == 0)
            assert queue(stopped, b)["active"] == 2
            assert Path(str(base / "a-0.bin") + ".part.meta").exists()
            before = sum(row["downloaded"] or 0 for row in stopped["tasks"] if row["queue_id"] == b)
            time.sleep(0.5)
            after = sum(row["downloaded"] or 0 for row in cli("list")["tasks"] if row["queue_id"] == b)
            assert after > before
            cli("queue", "rename", b, "Movies")
            cli("queue", "limit", b, 1)
            cli("queue", "disable", b)
            until(lambda state: queue(state, b)["active"] == 0)
            cli("queue", "stop")
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            persisted = cli("list")
            assert queue(persisted, b)["name"] == "Movies" and queue(persisted, b)["limit"] == 1
            assert not queue(persisted, a)["running"] and not queue(persisted, b)["running"]
            cli("queue", "limit", b, 2)
            cli("priority", groups[a][2], "immediate")
            cli("queue", "enable", a)
            cli("queue", "enable", b)
            assert all(row["active"] == 0 for row in cli("list")["queues"])
            cli("queue", "start")
            priority = until(lambda state: any(row["id"] == groups[a][2] and row["status"] == "running" for row in state["tasks"]))
            assert queue(priority, a)["active"] == 1
            for ids in groups.values():
                for task_id in ids:
                    cli("rate", task_id, "unlimited")
            complete = until(lambda state: all(row["status"] == "complete" for row in state["tasks"]))
            for row in complete["tasks"]:
                assert hashlib.sha256(Path(row["output"]).read_bytes()).hexdigest() == EXPECTED
            cli("queue", "assign", 1, *groups[a])
            cli("queue", "remove", a)
            newer = cli("queue", "create", "New queue", 1)["queue_id"]
            assert newer > b
            print("per-queue/global limits, isolated stop, atomic moves, restart, priority, file hashes and queue guards passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
