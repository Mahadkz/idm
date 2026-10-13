"""Task details, fresh redownload, removal preserving files, and durable receipts."""
import hashlib
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = hashlib.sha256(bytes(range(256)) * (8 * 1024 * 1024 // 256)).hexdigest()


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-actions-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, text=True, capture_output=True, timeout=5)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout)

        def rpc(kind, **payload):
            request = {"version": 1, "token": (base / "state/ipc-token").read_text(),
                       "command": {"type": kind, "payload": payload}}
            data = json.dumps(request).encode()
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(5)
                stream.connect(str(base / "state/daemon.sock"))
                stream.sendall(struct.pack("=I", len(data)) + data)
                with stream.makefile("rb") as reader:
                    size = struct.unpack("=I", reader.read(4))[0]
                    return json.loads(reader.read(size))

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def info(task_id):
            return cli("info", task_id)["tasks"][0]

        def wait_for(task_id, predicate):
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                row = info(task_id)
                if predicate(row):
                    return row
                time.sleep(0.1)
            raise AssertionError(row)

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            daemon = start()
            cli("queue", "stop")
            original = base / "original.bin"
            submission = {"request_id": "original-action-test", "url": f"http://127.0.0.1:{port}/original.bin",
                          "output": str(original), "headers": [["User-Agent", "xdm-actions-fixture"]],
                          "priority": 2, "connections": 8}
            source = rpc("submit", **submission)["task_id"]
            cli("rate", source, "8192")
            detail = info(source)
            assert detail["connections"] == 8 and detail["created_at"] and detail["updated_at"]
            assert "headers" not in detail and "headers_json" not in detail
            cli("queue", "start")
            wait_for(source, lambda row: row["status"] == "complete")
            cli("queue", "stop")
            copy = base / "copy.bin"
            request = {"task_id": source, "request_id": "redownload-action-test", "output": str(copy)}
            first = rpc("redownload", **request)
            duplicate = rpc("redownload", **request)
            assert first["ok"] and duplicate["task_id"] == first["task_id"]
            clone = first["task_id"]
            detail = info(clone)
            assert detail["status"] == "queued" and detail["priority"] == 2
            assert detail["connections"] == 8 and detail["rate_limit"] == 8192 * 1024
            assert detail["url"] == submission["url"] and not copy.exists()
            assert not rpc("redownload", **{**request, "output": str(base / "different.bin")})["ok"]
            cli("redownload", source, original, success=False)
            stale = base / "stale.bin"
            Path(str(stale) + ".part.meta").write_text("existing checkpoint")
            cli("redownload", source, stale, success=False)
            cli("redownload", source, base / "missing" / ".." / "original.bin", success=False)
            cli("queue", "start")
            wait_for(clone, lambda row: row["status"] == "complete")
            assert hashlib.sha256(original.read_bytes()).hexdigest() == EXPECTED
            assert hashlib.sha256(copy.read_bytes()).hexdigest() == EXPECTED
            requests = [json.loads(line) for line in (base / "http/requests.jsonl").read_text().splitlines()]
            assert all({key.lower(): value for key, value in row["headers"].items()}["user-agent"] == "xdm-actions-fixture" for row in requests if row["event"] == "request")
            cli("queue", "stop")
            partial = base / "partial.bin"
            paused_id = cli("submit", f"http://127.0.0.1:{port}/partial.bin", partial)["task_id"]
            cli("rate", paused_id, "256")
            cli("queue", "start")
            wait_for(paused_id, lambda row: row["status"] == "running" and (row["downloaded"] or 0) > 0)
            cli("remove", paused_id, success=False)
            cli("redownload", paused_id, base / "active-copy.bin", success=False)
            cli("pause", paused_id)
            wait_for(paused_id, lambda row: row["status"] == "paused")
            cli("remove", paused_id)
            cli("info", paused_id, success=False)
            assert Path(str(partial) + ".part").exists() and Path(str(partial) + ".part.meta").exists()
            cli("remove", source)
            assert not rpc("submit", **submission)["ok"]
            assert original.exists() and copy.exists()
            cli("queue", "stop")
            newer = cli("submit", f"http://127.0.0.1:{port}/newer.bin", base / "newer.bin")["task_id"]
            assert newer > paused_id
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert not rpc("submit", **submission)["ok"]
            assert info(clone)["status"] == "complete"
            assert all(row["id"] not in [source, paused_id] for row in cli("list")["tasks"])
            print("details, header-preserving redownload, duplicate receipt, active guards, kept files/checkpoints and restart passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
