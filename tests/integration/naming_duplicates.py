"""N14 server-chosen filenames, the duplicate-download policy and the N13 modification-date resume rule."""
import hashlib
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"
SIZE = 4 * 1024 * 1024
BLOCK = bytes(range(256)) * 256
EXPECTED = hashlib.sha256(BLOCK * (SIZE // len(BLOCK))).hexdigest()

# Set in main(): the temp root and the wall-clock instant from which the
# resuming transfers see flipped validators (N13).
BASE = None
FLIP_AT = float("inf")
LAST_MODIFIED = "Mon, 01 Jan 2024 00:00:00 GMT"

DISPOSITIONS = {
    "/plain.bin": 'attachment; filename="plain name.pdf"',
    "/utf8.zip": "attachment; filename*=UTF-8''caf%C3%A9%20%E2%82%AC.zip",
    "/reserved-CON": 'attachment; filename="CON"',
    "/reserved-com1.bin": 'attachment; filename="com1.bin"',
    "/collision-1.txt": 'attachment; filename="same name.txt"',
    "/collision-2.txt": 'attachment; filename="same name.txt"',
}


def payload(start, end):
    chunks = []
    position = start
    while position <= end:
        offset = position % len(BLOCK)
        count = min(len(BLOCK) - offset, end - position + 1)
        chunks.append(BLOCK[offset:offset + count])
        position += count
    return b"".join(chunks)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *unused):
        pass

    def plain(self, status, headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        path = self.path.split("#")[0].split("?")[0]
        if path == "/redirect-from":
            # N14: the final redirected URL decides the filename.
            return self.plain(302, [("Location", "/final-dir/name%20from%20redirect.tar.gz?x=1")])
        etag, last_modified = '"etag-1"', LAST_MODIFIED
        if time.monotonic() >= FLIP_AT:
            last_modified = "Tue, 02 Jan 2024 00:00:00 GMT"
            if path == "/n13-etag.bin":
                etag = '"etag-2"'
        start, end = 0, SIZE - 1
        value = self.headers.get("Range")
        if value and value.startswith("bytes="):
            first, second = value[6:].split("-")
            start, end = int(first), int(second) if second else SIZE - 1
        self.send_response(206 if value else 200)
        self.send_header("ETag", etag)
        self.send_header("Last-Modified", last_modified)
        disposition = DISPOSITIONS.get(path)
        if disposition:
            self.send_header("Content-Disposition", disposition)
        self.send_header("Content-Length", str(end - start + 1))
        if value:
            self.send_header("Content-Range", f"bytes {start}-{end}/{SIZE}")
        self.end_headers()
        position = start
        try:
            while position <= end:
                count = min(64 * 1024, end - position + 1)
                self.wfile.write(payload(position, position + count - 1))
                self.wfile.flush()
                position += count
                if path.startswith("/n13"):
                    time.sleep(0.05)
        except (ConnectionError, OSError):
            pass


def exchange(writer, payload, reader=None):
    reader = reader or writer
    data = json.dumps(payload).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def main():
    global BASE, FLIP_AT
    with tempfile.TemporaryDirectory(prefix="xdm-naming-") as temp:
        BASE = base = Path(temp)
        holder = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        holder.daemon_threads = True
        threading.Thread(target=holder.serve_forever, daemon=True).start()
        url = lambda name: f"http://127.0.0.1:{holder.server_port}/{name}"
        daemon = None
        try:
            env = os.environ.copy()
            env.update({"XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")})
            (base / "home").mkdir()
            daemon = subprocess.Popen([str(DAEMON)], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()

            def cli(*args, success=True):
                result = subprocess.run([str(CLI), "daemon", *map(str, args)], env=env, text=True,
                                        capture_output=True, timeout=10)
                assert (result.returncode == 0) == success, (args, result.returncode, result.stderr, result.stdout)
                return json.loads(result.stdout) if result.stdout.strip() else None

            def rpc(kind, payload):
                with socket.socket(socket.AF_UNIX) as raw:
                    raw.connect(str(base / "state/daemon.sock"))
                    with raw.makefile("rwb") as conn:
                        token = (base / "state/ipc-token").read_text().strip()
                        return exchange(conn, {"version": 1, "token": token,
                                               "command": {"type": kind, "payload": payload}}, conn)

            def wait_for(task_id, predicate, timeout=60):
                deadline = time.monotonic() + timeout
                row = None
                while time.monotonic() < deadline:
                    row = cli("info", task_id)["tasks"][0]
                    if predicate(row):
                        return row
                    time.sleep(0.1)
                raise AssertionError(row)

            def names(directory):
                return sorted(item.name for item in directory.iterdir())

            # --- N14 naming: three submission groups share the /naming directory.
            naming_dir = base / "naming"
            naming = {}
            for request_id, url_name in [
                ("n14-plain", "plain.bin"),
                ("n14-utf8", "utf8.zip"),
                ("n14-reserved", "reserved-CON"),
                ("n14-com1", "reserved-com1.bin"),
                ("n14-redirect", "redirect-from"),
                ("n14-collision-1", "collision-1.txt"),
            ]:
                naming[request_id] = rpc("submit_ex", {
                    "request_id": request_id,
                    "url": url(url_name),
                    "output": str(naming_dir / f"{request_id}.bin"),
                    "headers": [],
                    "method": "GET",
                    "body": None,
                    "priority": 1,
                    "connections": 4,
                    "name_from_server": True,
                    "ignore_modification_date": False,
                    "allow_duplicate": False,
                })["task_id"]
            cli("queue", "start")
            for request_id, task_id in naming.items():
                wait_for(task_id, lambda row: row["status"] == "complete")
            listing = names(naming_dir)
            assert (naming_dir / "plain name.pdf").exists(), listing
            assert (naming_dir / "café €.zip").exists(), listing
            assert (naming_dir / "CON_").exists(), listing
            assert (naming_dir / "com1_.bin").exists(), listing
            # The redirected URL's last path segment owns the name.
            redirect_path = next(p for p in naming_dir.iterdir() if p.name.endswith(".tar.gz"))
            assert redirect_path.name == "name from redirect.tar.gz", listing
            assert (naming_dir / "same name.txt").exists(), listing
            assert not (naming_dir / "n14-plain.bin").exists(), listing
            for name, size in [
                ("plain name.pdf", SIZE), ("café €.zip", SIZE), ("same name.txt", SIZE),
                ("name from redirect.tar.gz", SIZE), ("CON_", SIZE), ("com1_.bin", SIZE)]:
                assert len((naming_dir / name).read_bytes()) == size, name
            # The second collision submits a different URL exposing the same
            # disposition name; the destination gets a " (2)" suffix.
            two_name = naming_dir / "same name (2).txt"
            assert not two_name.exists(), listing
            two_id = rpc("submit_ex", {
                "request_id": "n14-collision-2",
                "url": url("collision-2.txt"),
                "output": str(naming_dir / "collision-2.bin"),
                "headers": [],
                "method": "GET",
                "body": None,
                "priority": 1,
                "connections": 4,
                "name_from_server": True,
            })["task_id"]
            wait_for(two_id, lambda row: row["status"] == "complete")
            assert len(two_name.read_bytes()) == SIZE, listing

            # --- Duplicate detection (IDM): the same normalized URL is refused
            # with the owning task ID until --allow-duplicate passes.
            dup_dir = base / "dup"
            dup_dir.mkdir()
            refusal = rpc("submit_ex", {
                "request_id": "dup-plain",
                "url": url("collision-1.txt"),
                "output": str(dup_dir / "dup-plain.bin"),
                "headers": [], "method": "GET", "body": None,
                "priority": 1, "connections": 4,
                "name_from_server": False,
                "ignore_modification_date": False,
                "allow_duplicate": False,
            })
            assert not refusal["ok"], refusal
            assert refusal["duplicate_of"] == naming["n14-collision-1"], refusal
            refusal = rpc("submit_ex", {
                "request_id": "dup-fragment",
                "url": url("collision-1.txt") + "#downloaded",
                "output": str(dup_dir / "dup-fragment.bin"),
                "headers": [], "method": "GET", "body": None,
                "priority": 1, "connections": 4,
                "name_from_server": False,
                "ignore_modification_date": False,
                "allow_duplicate": False,
            })
            assert not refusal["ok"], refusal
            assert refusal["duplicate_of"] == naming["n14-collision-1"], refusal
            assert list(dup_dir.iterdir()) == [], names(dup_dir)
            # The legacy CLI submit keeps the old no-check semantics.
            legacy = cli("submit", url("collision-1.txt"), dup_dir / "legacy.bin")
            assert legacy["ok"] and legacy["task_id"] != naming["n14-collision-1"], legacy
            # The CLI flag downloads a second copy from the same URL.
            cli("queue", "stop")
            allowed = cli("submit", url("collision-1.txt"), dup_dir / "allowed.bin", "--allow-duplicate")
            assert allowed["ok"] and allowed["task_id"] and allowed["duplicate_of"] is None, allowed
            cli("queue", "start")
            wait_for(allowed["task_id"], lambda row: row["status"] == "complete")
            assert hashlib.sha256((dup_dir / "allowed.bin").read_bytes()).hexdigest() == EXPECTED

            # --- N13: pause mid-transfer, then resume against a flipped
            # validator. Default tasks must not splice the restarted object;
            # the ignore flag may only relax a changed Last-Modified.
            n13_dir = base / "n13"
            FLIP_AT = time.monotonic() + 6
            cases = {}
            for name, ignore in [("/n13-default.bin", False), ("/n13-ignore.bin", True), ("/n13-etag.bin", True)]:
                cases[name] = rpc("submit_ex", {
                    "request_id": f"n13-{name.strip('/').replace('/', '-')}",
                    "url": url(name.lstrip("/")),
                    "output": str(n13_dir / Path(name).name),
                    "headers": [], "method": "GET", "body": None,
                    "priority": 1, "connections": 4,
                    "name_from_server": False,
                    "ignore_modification_date": ignore,
                })["task_id"]
                wait_for(cases[name], lambda row: (row["status"] == "running" and (row["downloaded"] or 0) > 0))
                cli("pause", cases[name])
                wait_for(cases[name], lambda row: row["status"] == "paused")
                cli("rate", cases[name], "64")
            # The partial data must have made checkpoint progress.
            for name in cases:
                checkpoint = n13_dir / (Path(name).name + ".part")
                assert checkpoint.exists(), names(n13_dir)
            while time.monotonic() < FLIP_AT:
                time.sleep(0.2)
            for name in cases:
                cli("resume", cases[name])
            n13_rows = {}
            for name in cases:
                n13_rows[name] = wait_for(cases[name],
                                          lambda row: row["status"] in ("complete", "failed", "restart_required", "paused"),
                                          timeout=180)
            assert n13_rows["/n13-ignore.bin"]["status"] == "complete", n13_rows
            assert n13_rows["/n13-default.bin"]["status"] == "restart_required", n13_rows
            assert n13_rows["/n13-etag.bin"]["status"] == "restart_required", n13_rows
            assert hashlib.sha256((n13_dir / "n13-ignore.bin").read_bytes()).hexdigest() == EXPECTED
            assert not (n13_dir / "n13-default.bin").exists()
            assert not (n13_dir / "n13-etag.bin").exists()

            print(json.dumps({"result": "passed", "cases": [
                "N14 Content-Disposition plain and RFC 5987 UTF-8 names",
                "reserved Windows name sanitizing",
                "redirect-derived name",
                "filename collision suffix",
                "duplicate rejection and allow, including normalization",
                "N13 resume refused by default and allowed with a matching strong ETag",
                "N13 never ignores a changed strong ETag",
            ]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            holder.shutdown()
            try:
                holder.wait(timeout=5)
            except Exception:
                pass


if __name__ == "__main__":
    main()
