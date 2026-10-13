"""Preview, atomic batch import, receipts, and portable exports without request headers."""
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
    with tempfile.TemporaryDirectory(prefix="xdm-lists-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=10)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return result.stdout

        def rpc(kind, **payload):
            data = json.dumps({"version": 1, "token": (base / "state/ipc-token").read_text(),
                               "command": {"type": kind, "payload": payload}}).encode()
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(10)
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

        def rows():
            return json.loads(cli("list"))["tasks"]

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            urls = base / "urls.txt"
            urls.write_text(f"# fixture list\nhttp://127.0.0.1:{port}/a.bin\nhttp://127.0.0.1:{port}/a.bin\nhttp://127.0.0.1:{port}/folder/a.bin\nhttp://127.0.0.1:{port}/b.bin\n")
            preview = json.loads(cli("preview", urls))
            assert [entry["filename"] for entry in preview["entries"]] == ["a.bin", "a (2).bin", "b.bin"]
            daemon = start()
            cli("queue", "stop")
            imported = json.loads(cli("import", urls, base / "paused"))
            assert len(imported["task_ids"]) == 3 and all(row["status"] == "paused" for row in rows())
            before = len(rows())
            cli("import", urls, base / "paused", success=False)
            assert len(rows()) == before
            blocked = base / "blocked"
            blocked.mkdir(); (blocked / "a (2).bin").write_text("keep this file")
            cli("import", urls, blocked, success=False)
            assert len(rows()) == before and not (blocked / "a.bin").exists()
            bad = base / "unsafe.json"
            bad_data = json.loads(json.dumps(preview))
            bad_data["entries"][0]["filename"] = "../escape"
            bad.write_text(json.dumps(bad_data))
            cli("import", bad, base / "unsafe", success=False)
            assert len(rows()) == before
            secret = "session-private-fixture"
            source = rpc("submit", request_id="secret-task", url=f"http://user:password@127.0.0.1:{port}/private.bin",
                         output=str(base / "private.bin"), headers=[["Cookie", secret], ["Authorization", "private-auth"]],
                         priority=2, connections=4)
            assert source["ok"]
            cli("rate", source["task_id"], 8192)
            archive = base / "tasks.json"
            cli("export", archive, "json")
            text = archive.read_text()
            assert secret not in text and "private-auth" not in text and "password" not in text
            assert '"headers"' not in text and str(base) not in text
            exported = json.loads(text)
            private = next(entry for entry in exported["entries"] if entry["filename"] == "private.bin")
            assert private["connections"] == 4 and private["rate_limit"] == 8192 * 1024 and private["priority"] == 2
            cli("export", archive, "json", success=False)
            assert archive.read_text() == text
            plain = base / "exported.txt"
            cli("export", plain, "urls")
            assert len(plain.read_text().splitlines()) == 4 and "password" not in plain.read_text()
            started = json.loads(cli("import", archive, base / "running", "--start"))
            assert len(started["task_ids"]) == 4
            cli("queue", "start")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                tasks = {row["id"]: row for row in rows()}
                if all(tasks[task_id]["status"] == "complete" for task_id in started["task_ids"]):
                    break
                time.sleep(0.1)
            assert all(tasks[task_id]["status"] == "complete" for task_id in started["task_ids"])
            for task_id in started["task_ids"]:
                assert hashlib.sha256(Path(tasks[task_id]["output"]).read_bytes()).hexdigest() == EXPECTED
            cli("queue", "stop")
            batch = {"request_id": "stable-batch", "directory": str(base / "stable"), "entries": preview["entries"], "enqueue": False}
            first = rpc("import_list", **batch)
            assert first["ok"] and rpc("import_list", **batch)["task_ids"] == first["task_ids"]
            assert not rpc("import_list", **{**batch, "enqueue": True})["ok"]
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert rpc("import_list", **batch)["task_ids"] == first["task_ids"]
            cli("remove", first["task_ids"][0])
            assert not rpc("import_list", **batch)["ok"]
            media = rpc("submit_media", request_id="media-list-fixture", url=f"http://127.0.0.1:{port}/master.m3u8",
                        output=str(base / "media-source.mp4"), video=2, audio=3, priority=1)
            assert media["ok"]
            media_archive = base / "media-list.json"
            cli("export", media_archive, "json")
            media_entry = next(entry for entry in json.loads(media_archive.read_text())["entries"] if entry["kind"] == "media")
            assert media_entry["video"] == 2 and media_entry["audio"] == 3 and media_entry["connections"] == 1
            restored = json.loads(cli("import", media_archive, base / "media-restored"))
            media_rows = [row for row in rows() if row["id"] in restored["task_ids"] and row["kind"] == "media"]
            assert len(media_rows) == 1 and media_rows[0]["media_video"] == 2 and media_rows[0]["media_audio"] == 3
            print("preview/dedup, atomic conflicts, paused/start import, private-header omission, JSON round trip, hashes and persisted receipts passed")

            batch = json.loads(cli("batch", f"http://127.0.0.1:{port}/part[08-12:2].bin", base / "batch"))
            names = sorted(Path(t["output"]).name for t in rows() if t["id"] in batch["task_ids"])
            assert names == ["part08.bin", "part10.bin", "part12.bin"], names
            cli("batch", f"http://127.0.0.1:{port}/[1-a].bin", base / "bad-batch", success=False)
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
