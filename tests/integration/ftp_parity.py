"""FTP parity with HTTP: MDTM server time and the daemon temp directory, over real binaries."""

import hashlib
import json
import os
import socket
import socketserver
import struct
import subprocess
import tempfile
import threading
import time
from pathlib import Path

import protocol_tasks as pt

ROOT = pt.ROOT
MDTM_EPOCH = 1767225600  # 2026-01-01T00:00:00Z, what the fixture reports for MDTM


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-ftp-parity-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        (base / "home").mkdir()
        server_class = type("S", (socketserver.ThreadingTCPServer,), {"allow_reuse_address": True, "daemon_threads": True})
        ftp = server_class(("127.0.0.1", 0), pt.FtpHandler)
        threading.Thread(target=ftp.serve_forever, daemon=True).start()
        daemon = None
        try:
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            token = (base / "state/ipc-token").read_text().strip()

            def rpc(kind, payload=None):
                command = {"type": kind}
                if payload is not None:
                    command["payload"] = payload
                with socket.socket(socket.AF_UNIX) as raw:
                    raw.settimeout(45)
                    raw.connect(str(base / "state/daemon.sock"))
                    return pt.exchange(raw.makefile("rwb"), {"version": 1, "token": token, "command": command})

            def submit(request_id, output, **extra):
                payload = {"request_id": request_id, "url": f"ftp://127.0.0.1:{ftp.server_address[1]}/{request_id}.bin",
                           "output": str(output), "headers": [], "method": "GET", "body": None, "priority": 1,
                           "connections": 1, "name_from_server": False, "ignore_modification_date": False,
                           "allow_duplicate": True}
                payload.update(extra)
                reply = rpc("submit_ex", payload)
                assert reply["ok"], reply
                return reply["task_id"]

            def row(task_id):
                return rpc("get_task", {"task_id": task_id})["tasks"][0]

            scratch = base / "scratch"
            stored = rpc("set_temp_dir", {"path": str(scratch)})
            assert stored["ok"], stored
            out = base / "out"

            # Partials of an FTP task live in the temp directory, never beside the destination.
            task = submit("with-time", out / "a.bin")
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 4096, what="running")
            assert rpc("pause", {"task_id": task})["ok"]
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "paused", what="paused")
            assert (row(task)["downloaded"] or 0) > 0, "paused progress is read from the temp directory"
            parts = sorted(p.name for p in scratch.iterdir())
            assert any(n.endswith(".part") for n in parts) and any(n.endswith(".part.ftp") for n in parts), parts
            assert not (out / "a.bin").exists() and not list(out.glob("a.bin.*")), list(out.iterdir())
            assert rpc("resume", {"task_id": task})["ok"]
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "complete", what="complete")
            assert hashlib.sha256((out / "a.bin").read_bytes()).hexdigest() == pt.EXPECTED
            assert int((out / "a.bin").stat().st_mtime) == MDTM_EPOCH
            assert list(scratch.iterdir()) == [], list(scratch.iterdir())

            # preserve_server_time=false leaves the completion time alone.
            task = submit("no-time", out / "b.bin", preserve_server_time=False)
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "complete", what="complete b")
            assert abs((out / "b.bin").stat().st_mtime - time.time()) < 300

            # Clearing the setting returns new tasks to partials beside the destination.
            assert rpc("set_temp_dir", {"path": None})["ok"]
            task = submit("beside", out / "c.bin")
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 4096, what="running c")
            assert rpc("pause", {"task_id": task})["ok"]
            pt.wait_for(lambda: row(task), lambda r: r["status"] == "paused", what="paused c")
            assert (out / "c.bin.part").exists() and (out / "c.bin.part.ftp").exists()
            assert list(scratch.iterdir()) == []
            print(json.dumps({"result": "passed", "cases": ["ftp temp dir partials", "ftp mdtm mtime", "ftp preserve off", "ftp temp cleared"]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            ftp.shutdown(); ftp.server_close()


if __name__ == "__main__":
    main()
