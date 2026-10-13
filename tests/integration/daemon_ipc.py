"""Linux end-to-end daemon/native-host contract; run after cargo build --workspace."""

import hashlib
import json
import os
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = hashlib.sha256(bytes(range(256)) * (8 * 1024 * 1024 // 256)).hexdigest()


def exchange(writer, payload, reader=None):
    reader = reader or writer
    data = json.dumps(payload).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-daemon-test-") as temp:
        base = Path(temp)
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"), str(base / "http"), "--delay", "0.01"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        daemon = None
        try:
            port = json.loads(fixture.stdout.readline())["port"]
            env = os.environ.copy()
            env.update({"XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")})
            (base / "home").mkdir()
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()

            with socket.socket(socket.AF_UNIX) as raw:
                raw.connect(str(base / "state/daemon.sock"))
                with raw.makefile("rwb") as conn:
                    denied = exchange(conn, {"version": 1, "token": "invalid", "command": {"type": "list"}})
                    assert not denied["ok"]

            native = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            assert exchange(native.stdin, {"version": 1, "type": "hello"}, native.stdout)["type"] == "hello.result"
            request = {"version": 1, "requestId": "test-native-1", "type": "download.submit", "payload": {"url": f"http://127.0.0.1:{port}/queue-a.bin", "filenameHint": "from-browser.bin", "requestContext": {"headers": [{"name": "User-Agent", "value": "xdm-integration"}]}}}
            first = exchange(native.stdin, request, native.stdout)
            second = exchange(native.stdin, request, native.stdout)
            assert first["accepted"] and second["accepted"] and first["taskId"] == second["taskId"]
            invalid = json.loads(json.dumps(request)); invalid["requestId"] = "bad-name"; invalid["payload"]["filenameHint"] = "../escape"
            assert not exchange(native.stdin, invalid, native.stdout)["accepted"]
            video_request = json.loads(json.dumps(request)); video_request["requestId"] = "test-video-category"; video_request["payload"]["filenameHint"] = "clip.mp4"
            assert exchange(native.stdin, video_request, native.stdout)["accepted"]
            native.stdin.close(); assert native.wait(timeout=5) == 0

            cli_env = env.copy()
            deadline = time.monotonic() + 20
            status = None
            while time.monotonic() < deadline:
                result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "list"], env=cli_env, text=True, capture_output=True, timeout=5)
                assert result.returncode == 0, result.stderr
                tasks = json.loads(result.stdout)["tasks"]
                status = tasks[0]["status"]
                if len(tasks) == 2 and all(task["status"] == "complete" for task in tasks): break
                time.sleep(0.1)
            output = base / "home/Downloads/from-browser.bin"
            assert status == "complete" and hashlib.sha256(output.read_bytes()).hexdigest() == EXPECTED
            video_output = base / "home/Downloads/Video/clip.mp4"
            assert hashlib.sha256(video_output.read_bytes()).hexdigest() == EXPECTED
            assert tasks[0]["category"] == "Video" and tasks[1]["category"] == "Other"
            invalid_limit = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "limit", "0"], env=env, text=True, capture_output=True, timeout=5)
            assert invalid_limit.returncode != 0
            set_limit = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "limit", "3"], env=env, text=True, capture_output=True, timeout=5)
            assert set_limit.returncode == 0, set_limit.stderr

            def cli(*arguments):
                return subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *arguments], env=env, text=True, capture_output=True, timeout=5)
            assert cli("queue", "stop").returncode == 0
            queued = cli("submit", f"http://127.0.0.1:{port}/queue-a.bin", str(base / "priority.bin"))
            assert queued.returncode == 0, queued.stderr
            queued_id = str(json.loads(queued.stdout)["task_id"])
            assert cli("priority", queued_id, "immediate").returncode == 0
            invalid_priority = cli("priority", queued_id, "urgent")
            assert invalid_priority.returncode != 0
            selected = next(task for task in json.loads(cli("list").stdout)["tasks"] if str(task["id"]) == queued_id)
            assert selected["priority"] == 2 and selected["status"] == "queued"

            daemon.terminate(); daemon.wait(timeout=5)
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            persisted = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "list"], env=env, text=True, capture_output=True, timeout=5)
            persisted_tasks = json.loads(persisted.stdout)["tasks"]
            assert persisted.returncode == 0
            assert next(task for task in persisted_tasks if str(task["id"]) == queued_id)["status"] == "queued"
            assert sum(task["status"] == "complete" for task in persisted_tasks) == 2
            with sqlite3.connect(base / "state/tasks.sqlite3") as database:
                assert database.execute("SELECT value FROM settings WHERE key='max_concurrent'").fetchone()[0] == "3"
                assert database.execute("SELECT priority FROM tasks WHERE id=?", (int(queued_id),)).fetchone()[0] == 2
            print(json.dumps({"result": "passed", "cases": ["native framing", "authorized local IPC", "idempotent submission", "filename guard", "hash", "category path", "SQLite restart persistence", "concurrency setting", "priority setting"]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
