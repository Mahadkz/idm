"""Native host batch.submit over stdin/stdout framing against a real daemon on temp paths."""

import json
import os
import socket
import struct
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
seen = {}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        seen[self.path] = {"cookie": self.headers.get("Cookie"), "referer": self.headers.get("Referer")}
        body = (self.path * 50).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def exchange(writer, payload, reader=None):
    reader = reader or writer
    data = json.dumps(payload).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-batch-") as temp:
        base = Path(temp)
        httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        port = httpd.server_address[1]
        home = base / "home"
        home.mkdir()
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(home)}
        env.pop("XDG_DOWNLOAD_DIR", None)
        daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
        host = None
        try:
            assert "listening locally" in daemon.stderr.readline()
            host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env, stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            def send(message):
                return exchange(host.stdin, message, host.stdout)

            hello = send({"version": 1, "type": "hello"})
            assert "batch.submit" in hello["capabilities"], hello

            def item(i, name, mime=None):
                value = {"requestId": f"b-{i}", "url": f"http://127.0.0.1:{port}/{name}", "mime": mime or "",
                         "requestContext": {"referrer": "https://page.test/list",
                                            "cookies": [{"header": "session=abc123"}]}}
                return value

            items = [item(0, "a.zip"), item(1, "b.mp4"), item(2, "a.zip"), item(3, "c.pdf")]
            reply = send({"version": 1, "requestId": "batch-1", "type": "batch.submit",
                          "payload": {"batchId": "batch-1", "count": 4, "items": items}})
            assert reply["type"] == "batch.result" and reply["accepted"] is True, reply
            assert reply["count"] == 4 and reply["acceptedCount"] == 3 and len(reply["taskIds"]) == 3, reply
            assert [d["index"] for d in reply["skippedDuplicates"]] == [2], reply
            assert reply["skippedDuplicates"][0]["duplicateOfIndex"] == 0, reply

            # Idempotent replay of the same request.
            again = send({"version": 1, "requestId": "batch-1", "type": "batch.submit",
                          "payload": {"batchId": "batch-1", "count": 4, "items": items}})
            assert again["accepted"] and again["taskIds"] == reply["taskIds"], again

            deadline = time.monotonic() + 30
            downloads = home / "Downloads"
            expected = [downloads / "Compressed/a.zip", downloads / "Video/b.mp4", downloads / "Documents/c.pdf"]
            while time.monotonic() < deadline and not all(p.exists() and p.stat().st_size for p in expected):
                time.sleep(0.1)
            assert all(p.exists() for p in expected), [str(p) for p in expected if not p.exists()]
            assert (downloads / "Compressed/a.zip").read_bytes() == ("/a.zip" * 50).encode()
            # Browser cookies and referrer ride along with every task.
            assert seen["/a.zip"] == {"cookie": "session=abc123", "referer": "https://page.test/list"}, seen

            # A later batch reports URLs that already belong to tasks, by item index.
            second = send({"version": 1, "requestId": "batch-2", "type": "batch.submit",
                           "payload": {"batchId": "batch-2", "count": 2, "items": [item(0, "a.zip"), item(1, "d.bin")]}})
            assert second["accepted"] and second["acceptedCount"] == 1, second
            dup = second["skippedDuplicates"]
            assert len(dup) == 1 and dup[0]["index"] == 0 and dup[0]["duplicateOf"] == reply["taskIds"][0], second

            # Rejections are error replies (the extension then falls back to per-link submission) and add nothing.
            for bad in [
                [item(0, "x.bin"), {"requestId": "b-1", "url": "file:///etc/passwd"}],
                [item(0, "x.bin"), {"requestId": "b-1", "url": "ftp://127.0.0.1/y.bin"}],
                [item(0, "x.bin"), {"requestId": "b-1", "url": f"http://127.0.0.1:{port}/z.bin", "method": "POST", "body": "q=1"}],
                [],
            ]:
                rejected = send({"version": 1, "requestId": "batch-bad", "type": "batch.submit",
                                 "payload": {"batchId": "batch-bad", "count": len(bad), "items": bad}})
                assert rejected["type"] == "error" and rejected.get("reason"), rejected
            oversized = [item(i, f"f{i}.bin") for i in range(2001)]
            rejected = send({"version": 1, "requestId": "batch-big", "type": "batch.submit",
                             "payload": {"items": oversized}})
            assert rejected["type"] == "error", rejected["type"]

            with socket.socket(socket.AF_UNIX) as raw:
                raw.connect(str(base / "state/daemon.sock"))
                token = (base / "state/ipc-token").read_text().strip()
                listed = exchange(raw.makefile("rwb"), {"version": 1, "token": token, "command": {"type": "list"}})
            names = sorted(Path(t["output"]).name for t in listed["tasks"])
            assert names == ["a.zip", "b.mp4", "c.pdf", "d.bin"], names
            host.stdin.close()
            assert host.wait(timeout=5) == 0
            print(json.dumps({"result": "passed", "cases": ["batch accepted", "in-batch duplicates", "idempotent replay",
                                                            "cookies and referrer", "existing-task duplicates",
                                                            "rejections are errors", "size cap"]}))
        finally:
            if host and host.poll() is None:
                host.kill()
            daemon.terminate(); daemon.wait(timeout=5)
            httpd.shutdown(); httpd.server_close()


if __name__ == "__main__":
    main()
