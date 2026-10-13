"""End-to-end FTP and site-grabber tasks over the local daemon IPC protocol."""

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
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = bytes(range(256)) * (4 * 1024 * 1024 // 256)
EXPECTED = hashlib.sha256(PAYLOAD).hexdigest()
CHUNK = 4096
CHUNK_DELAY = 0.004


def exchange(writer, request, reader=None):
    reader = reader or writer
    data = json.dumps(request).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def request(token, kind, **payload):
    return {"version": 1, "token": token, "command": {"type": kind, "payload": payload}}


class FtpHandler(socketserver.StreamRequestHandler):
    """Minimal anonymous FTP control channel: USER/PASS/TYPE/SIZE/MDTM/
    EPSV/PASV/REST/RETR/QUIT with throttled data sends."""

    def setup(self):
        super().setup()
        self.data_listener = None
        self.rest_offset = 0
        self.wfile.write(b"220 xdm protocol test server\r\n")

    def send_command(self, line):
        self.wfile.write(line.encode() + b"\r\n")

    def handle(self):
        for line in self.rfile:
            line = line.strip().decode(errors="replace")
            words = line.split(" ", 1)
            verb = words[0].upper()
            argument = words[1] if len(words) > 1 else ""
            if verb == "USER":
                self.send_command("331 Password required")
            elif verb == "PASS":
                self.send_command("230 Anonymous login ok")
            elif verb == "TYPE":
                self.send_command("200 Type set to I")
            elif verb == "SIZE":
                self.send_command(f"213 {len(PAYLOAD)}")
            elif verb == "MDTM":
                self.send_command("213 20260101000000")
            elif verb == "REST":
                self.rest_offset = int(argument or 0)
                self.send_command(f"350 Restarting at {self.rest_offset}")
            elif verb == "EPSV":
                port = self.open_data()
                self.send_command(f"229 Entering Extended Passive Mode (|||{port}|)")
            elif verb == "PASV":
                port = self.open_data()
                self.send_command(f"227 Entering Passive Mode (127,0,0,1,{port // 256},{port % 256})")
            elif verb == "RETR":
                self.send_command("150 Opening BINARY mode data connection")
                self.stream()
                self.send_command("226 Transfer complete")
            elif verb == "QUIT":
                self.send_command("221 Bye")
                break
            else:
                self.send_command("502 Not implemented")
        self.close_data()

    def open_data(self):
        self.close_data()
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        self.data_listener = listener
        return listener.getsockname()[1]

    def stream(self):
        try:
            connection, _ = self.data_listener.accept()
        except (OSError, AttributeError, TypeError):
            self.close_data()
            return
        with connection:
            offset = self.rest_offset
            try:
                while offset < len(PAYLOAD):
                    piece = PAYLOAD[offset:offset + CHUNK]
                    connection.sendall(piece)
                    offset += len(piece)
                    time.sleep(CHUNK_DELAY)
            except OSError:
                pass
        self.close_data()

    def close_data(self):
        if self.data_listener is not None:
            self.data_listener.close()
            self.data_listener = None


def wait_for(examine, predicate, timeout=40, what="state"):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        current = examine()
        if predicate(current):
            return current
        time.sleep(0.1)
    raise AssertionError(f"timeout waiting for {what}: {json.dumps(current)[:2000]}")


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-protocol-tasks-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        (base / "home").mkdir()
        ftp_class = type("FtpServer", (socketserver.ThreadingTCPServer,), {"allow_reuse_address": True, "daemon_threads": True})
        ftp = ftp_class(("127.0.0.1", 0), FtpHandler)
        threading.Thread(target=ftp.serve_forever, daemon=True).start()

        site = base / "site"
        site.joinpath("files").mkdir(parents=True)
        site.joinpath("docs").mkdir()
        (site / "files" / "index.html").write_text(
            '- <a href="alpha-a.zip">a</a>\n'
            '- <a href="bravo-b.zip">b</a>\n'
            '- <a href="../docs/page2.html">page</a>\n')
        (site / "files" / "alpha-a.zip").write_bytes(b"alpha payload")
        (site / "files" / "bravo-b.zip").write_bytes(b"bravo payload " * 20)
        (site / "docs" / "page2.html").write_text('<a href="/files/charlie-c.zip">c</a>')
        (site / "files" / "charlie-c.zip").write_bytes(b"charlie payload")
        httpd = ThreadingHTTPServer(("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=str(site)))
        threading.Thread(target=httpd.serve_forever, daemon=True).start()

        daemon = None
        try:
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            token = (base / "state/ipc-token").read_text()

            def rpc(kind, **payload):
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.settimeout(45)
                    stream.connect(str(base / "state/daemon.sock"))
                    # Unit commands such as `list` carry no payload.
                    command = {"type": kind, **({"payload": payload} if payload else {})}
                    return exchange(stream.makefile("rwb"), {"version": 1, "token": token, "command": command})

            accepted = rpc("submit", request_id="ftp-all-the-bytes",
                           url=f"ftp://127.0.0.1:{ftp.server_address[1]}/all-the-bytes.bin",
                           output=str(base / "downloads/all-the-bytes.bin"),
                           headers=[], priority=1, connections=1)
            assert accepted["ok"], accepted
            task_id = accepted["task_id"]

            def info():
                return rpc("get_task", task_id=task_id)["tasks"][0]

            wait_for(info, lambda row: row["status"] == "running" and (row["downloaded"] or 0) > 4096,
                     what="running download")
            running = info()
            assert running["total"] == len(PAYLOAD) and running["kind"] == "ftp", running
            assert rpc("pause", task_id=task_id)["ok"]
            wait_for(info, lambda row: row["status"] == "paused", what="paused state")
            paused = info()
            assert 0 < paused["downloaded"] < len(PAYLOAD)
            part = Path(str(base / "downloads/all-the-bytes.bin") + ".part")
            checkpoint = Path(str(base / "downloads/all-the-bytes.bin") + ".part.ftp")
            assert part.exists() and checkpoint.exists(), (part, checkpoint)
            assert not (base / "downloads/all-the-bytes.bin").exists()

            assert rpc("resume", task_id=task_id)["ok"]
            wait_for(info, lambda row: row["status"] == "complete", what="completed download")
            assert hashlib.sha256((base / "downloads/all-the-bytes.bin").read_bytes()).hexdigest() == EXPECTED
            assert info()["downloaded"] == len(PAYLOAD)

            grab_id = "grab-site-1"
            project = {
                "start_url": f"http://127.0.0.1:{httpd.server_address[1]}/files/",
                "max_depth": 2,
                "max_pages": 10,
                "same_host_only": True,
                "file_types": ["zip"],
                "obey_robots": False,
            }
            grab = rpc("grab_site", request_id=grab_id, project=project, directory=str(base / "grabout"),
                       queue_id=1, enqueue=True)
            assert grab["ok"], grab
            assert grab["grab"]["state"] == "running", grab
            replay = rpc("grab_site", request_id=grab_id, project=project, directory=str(base / "grabout"),
                         queue_id=1, enqueue=True)
            assert replay["grab"]["state"] == "running", "replay must not spawn a second crawl"

            def grab_state():
                status = rpc("grab_status", request_id=grab_id)
                assert status["ok"], status
                return status["grab"]

            report = wait_for(grab_state, lambda row: row["state"] == "done", what="finished grab")
            assert len(report["task_ids"]) == 3, report

            listed = rpc("list")["tasks"]
            grabbed = [row["output"] for row in listed if row["id"] in report["task_ids"]]
            assert sorted(Path(name).name for name in grabbed) == ["alpha-a.zip", "bravo-b.zip", "charlie-c.zip"], grabbed

            wait_for(lambda: rpc("list")["tasks"],
                     lambda rows: all(row["status"] == "complete" for row in rows if row["id"] in report["task_ids"]),
                     what="grab task completion")
            for name in ["alpha-a.zip", "bravo-b.zip", "charlie-c.zip"]:
                path = base / "grabout" / name
                assert path.read_bytes() == payload[name], (path, path.read_bytes()[:16])
            print(json.dumps({"result": "passed",
                              "cases": ["ftp submit", "ftp live progress", "ftp pause/resume", "ftp hash",
                                        "site grab enqueue", "grab task completion"]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            ftp.shutdown()
            ftp.server_close()
            httpd.shutdown()
            httpd.server_close()


payload = {
    "alpha-a.zip": b"alpha payload",
    "bravo-b.zip": b"bravo payload " * 20,
    "charlie-c.zip": b"charlie payload",
}


if __name__ == "__main__":
    main()
