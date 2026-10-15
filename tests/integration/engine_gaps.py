"""N41 server timestamps, N38 separate temp directory, unknown-length completion, MIME name reconciliation and ListPage sort keys."""
import json
import os
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"
KNOWN = "Sun, 06 Nov 1994 08:49:37 GMT"
KNOWN_EPOCH = 784111777
BLOCK = bytes(range(256)) * 256
# path -> (size, content type, last-modified, per-chunk delay, send content-length)
FILES = {
    "/ts.bin": (2 << 20, None, KNOWN, 0.0, True),
    "/badts.bin": (1 << 20, None, "not a date", 0.0, True),
    "/slow.bin": (4 << 20, None, KNOWN, 0.04, True),
    "/slow2.bin": (4 << 20, None, KNOWN, 0.04, True),
    "/unknown.bin": (1 << 20, None, KNOWN, 0.0, False),
    "/getfile.php": (100 << 10, "application/zip", None, 0.0, True),
    "/noext": (50 << 10, "application/pdf", None, 0.0, True),
    "/page.zip": (2 << 10, "text/html; charset=utf-8", None, 0.0, True),
    "/note.txt": (3 << 10, "text/plain", None, 0.0, True),
    "/s-small.bin": (100 << 10, None, None, 0.0, True),
    "/s-big.bin": (900 << 10, None, None, 0.0, True),
    "/s-mid.bin": (400 << 10, None, None, 0.0, True),
    "/s-a.bin": (60 << 10, None, None, 0.0, True),
    "/s-b.bin": (60 << 10, None, None, 0.0, True),
}


def data(start, end):
    out = []
    position = start
    while position <= end:
        offset = position % len(BLOCK)
        count = min(len(BLOCK) - offset, end - position + 1)
        out.append(BLOCK[offset:offset + count])
        position += count
    return b"".join(out)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *unused):
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path not in FILES:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            return self.end_headers()
        size, ctype, modified, delay, with_length = FILES[path]
        start, end, status = 0, size - 1, 200
        value = self.headers.get("Range")
        if value and with_length:
            first, second = value[6:].split("-")
            start, end, status = int(first), int(second) if second else size - 1, 206
        self.send_response(status)
        if ctype:
            self.send_header("Content-Type", ctype)
        if modified:
            self.send_header("Last-Modified", modified)
        self.send_header("ETag", '"v1"')
        if with_length:
            self.send_header("Content-Length", str(end - start + 1))
        else:
            self.send_header("Connection", "close")
            self.close_connection = True
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        position = start
        try:
            while position <= end:
                count = min(64 * 1024, end - position + 1)
                self.wfile.write(data(position, position + count - 1))
                self.wfile.flush()
                position += count
                if delay:
                    time.sleep(delay)
        except (ConnectionError, OSError):
            pass


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-engine-gaps-") as temp, \
            tempfile.TemporaryDirectory(prefix="xdm-shm-", dir="/dev/shm") as shm:
        base = Path(temp)
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
                                        capture_output=True, timeout=20)
                assert (result.returncode == 0) == success, (args, result.returncode, result.stderr, result.stdout)
                return json.loads(result.stdout) if result.stdout.strip() else None

            def rpc(kind, payload=None):
                command = {"type": kind}
                if payload is not None:
                    command["payload"] = payload
                with socket.socket(socket.AF_UNIX) as raw:
                    raw.connect(str(base / "state/daemon.sock"))
                    with raw.makefile("rwb") as conn:
                        token = (base / "state/ipc-token").read_text().strip()
                        body = json.dumps({"version": 1, "token": token, "command": command}).encode()
                        conn.write(struct.pack("=I", len(body)) + body)
                        conn.flush()
                        size = struct.unpack("=I", conn.read(4))[0]
                        return json.loads(conn.read(size))

            def submit(request_id, name, output, **extra):
                payload = {"request_id": request_id, "url": url(name), "output": str(output), "headers": [],
                           "method": "GET", "body": None, "priority": 1, "connections": 4,
                           "name_from_server": False, "ignore_modification_date": False,
                           "allow_duplicate": True}
                payload.update(extra)
                reply = rpc("submit_ex", payload)
                assert reply["ok"], reply
                return reply["task_id"]

            def row(task_id):
                return cli("info", task_id)["tasks"][0]

            def wait_for(task_id, predicate, timeout=60):
                deadline = time.monotonic() + timeout
                last = None
                while time.monotonic() < deadline:
                    last = row(task_id)
                    if predicate(last):
                        return last
                    time.sleep(0.1)
                raise AssertionError(last)

            def done(task_id):
                return wait_for(task_id, lambda r: r["status"] in ("complete", "failed", "paused", "restart_required"))

            def page(sort, **filt):
                reply = rpc("list_page", {"offset": 0, "limit": 100, "sort": sort, "filter": filt or None})
                assert reply["ok"], reply
                return [t["id"] for t in reply["tasks"]]

            cli("limit", 8)

            # --- N41: server Last-Modified becomes the file's modification time.
            out = base / "ts"
            a = submit("ts-1", "ts.bin", out / "a.bin")
            b = submit("ts-2", "ts.bin", out / "b.bin", preserve_server_time=False)
            c = submit("ts-3", "badts.bin", out / "c.bin")
            for task in (a, b, c):
                assert done(task)["status"] == "complete"
            assert int((out / "a.bin").stat().st_mtime) == KNOWN_EPOCH
            assert abs((out / "b.bin").stat().st_mtime - time.time()) < 300
            assert abs((out / "c.bin").stat().st_mtime - time.time()) < 300, "invalid date leaves the time unchanged"
            # A resumed download also gets the server time.
            d = submit("ts-4", "slow.bin", out / "d.bin")
            wait_for(d, lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 0)
            cli("pause", d)
            assert done(d)["status"] == "paused"
            cli("resume", d)
            assert done(d)["status"] == "complete"
            assert int((out / "d.bin").stat().st_mtime) == KNOWN_EPOCH

            # --- N38: partials live in the temp directory, same volume first.
            temp_same = base / "scratch"
            reply = rpc("set_temp_dir", {"path": "relative/dir"})
            assert not reply["ok"], reply
            assert rpc("set_temp_dir", {"path": str(temp_same)})["temp_dir"] == str(temp_same)
            assert cli("temp-dir", "get")["temp_dir"] == str(temp_same)
            dest = base / "final"
            t = submit("tmp-1", "slow.bin", dest / "t.bin")
            wait_for(t, lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 0)
            cli("pause", t)
            assert done(t)["status"] == "paused"
            assert not (dest / "t.bin.part").exists() and not (dest / "t.bin.part.meta").exists()
            assert any(p.name.endswith(".part") for p in temp_same.iterdir()), list(temp_same.iterdir())
            assert any(p.name.endswith(".part.meta") for p in temp_same.iterdir())
            paused = row(t)
            assert paused["downloaded"] and paused["total"], paused  # progress read from the temp checkpoint
            cli("resume", t)
            assert done(t)["status"] == "complete"
            assert (dest / "t.bin").stat().st_size == 4 << 20
            assert list(temp_same.iterdir()) == [], list(temp_same.iterdir())
            assert sorted(p.name for p in dest.iterdir()) == ["t.bin"]

            # delete_partial removes the temp-dir pair of a paused task.
            r = submit("tmp-2", "slow.bin", dest / "r.bin")
            wait_for(r, lambda x: x["status"] == "running" and (x["downloaded"] or 0) > 0)
            cli("pause", r)
            assert done(r)["status"] == "paused"
            assert len(list(temp_same.iterdir())) == 2
            assert rpc("task_remove_ex", {"task_id": r, "delete_partial": True})["ok"]
            assert list(temp_same.iterdir()) == []

            # PartialScan sees orphaned temp partials; delete stays XDM-checkpoint-only.
            o = submit("tmp-3", "slow.bin", dest / "orphan.bin")
            wait_for(o, lambda x: x["status"] == "running" and (x["downloaded"] or 0) > 0)
            cli("pause", o)
            assert done(o)["status"] == "paused"
            owned_scan = rpc("partial_scan", {"directory": str(dest)})
            assert owned_scan["partials"] == [], owned_scan
            assert rpc("remove", {"task_id": o})["ok"]
            (temp_same / "foreign.part").write_bytes(b"browser")
            (temp_same / "notes.part.meta").write_text("{}")
            scan = rpc("partial_scan", {"directory": str(dest)})
            assert scan["ok"] and len(scan["partials"]) == 1, scan
            entry = scan["partials"][0]
            assert entry["part"].startswith(str(temp_same)) and entry["meta"].startswith(str(temp_same))
            deleted = rpc("partial_delete", {"directory": str(dest), "paths": [entry["part"], entry["meta"]]})
            assert deleted["ok"] and deleted["total"] == 2, deleted
            assert sorted(p.name for p in temp_same.iterdir()) == ["foreign.part", "notes.part.meta"]

            # Cross-volume: temp on /dev/shm (a different device), destination on disk.
            assert os.stat(shm).st_dev != os.stat(base).st_dev
            assert rpc("set_temp_dir", {"path": shm})["ok"]
            x = submit("tmp-4", "slow.bin", dest / "x.bin")
            wait_for(x, lambda v: v["status"] == "running" and (v["downloaded"] or 0) > 0)
            cli("pause", x)
            assert done(x)["status"] == "paused"
            assert any(p.name.endswith(".part") for p in Path(shm).iterdir())
            cli("resume", x)
            assert done(x)["status"] == "complete"
            assert (dest / "x.bin").stat().st_size == 4 << 20
            assert int((dest / "x.bin").stat().st_mtime) == KNOWN_EPOCH
            assert not any(p.name.endswith((".part", ".part.meta", ".xdm-copy")) for p in Path(shm).iterdir())
            assert sorted(p.name for p in dest.iterdir()) == ["t.bin", "x.bin"]
            rpc("set_temp_dir", {"path": None})
            assert cli("temp-dir", "get")["temp_dir"] is None

            # --- Unknown-length completion.
            u = submit("unk-1", "unknown.bin", base / "unk/u.bin")
            finished = done(u)
            assert finished["status"] == "complete", finished
            assert finished["size_unverified"] is True and "might be incomplete" in finished["notice"], finished
            known = row(a)
            assert known["size_unverified"] is False and known["notice"] is None, known
            assert (base / "unk/u.bin").stat().st_size == 1 << 20

            # --- Server-name reconciliation (MIME vs extension).
            names = base / "names"
            ids = {n: submit(f"nm-{n}", n, names / f"placeholder-{i}.dat", name_from_server=True)
                   for i, n in enumerate(["getfile.php", "noext", "note.txt"])}
            html = submit("nm-html", "page.zip", names / "html-placeholder.dat", name_from_server=True)
            outcome = done(html)
            assert outcome["status"] == "failed" and "HTML page instead of a file" in outcome["error"], outcome
            assert outcome["retry_count"] == 0
            for n, task in ids.items():
                assert done(task)["status"] == "complete", (n, row(task))
            listing = sorted(p.name for p in names.iterdir())
            assert "getfile.zip" in listing and "noext.pdf" in listing and "note.txt" in listing, listing
            assert not any(p.endswith(".php") for p in listing), listing

            # --- ListPage sort keys.
            sizes = base / "sizes"
            sid = {n: submit(f"sz-{n}", f"s-{n}.bin", sizes / f"{n}.bin") for n in ["small", "big", "mid"]}
            for task in sid.values():
                assert done(task)["status"] == "complete"
            ordered = [t for t in page("+size", ) if t in sid.values()]
            assert ordered == [sid["small"], sid["mid"], sid["big"]], ordered
            assert [t for t in page("-size") if t in sid.values()] == ordered[::-1]
            # Progress: a paused half-done task sorts between untouched and complete ones.
            half = submit("sz-half", "slow2.bin", sizes / "half.bin")
            wait_for(half, lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 1 << 20)
            cli("pause", half)
            assert done(half)["status"] == "paused"
            order = page("+progress")
            assert order.index(half) < order.index(sid["big"]), order
            assert page("-progress")[0] != half
            # Rate: a throttled live task sorts below an unthrottled one.
            fast = submit("rt-fast", "slow.bin", sizes / "fast.bin")
            slow = submit("rt-slow", "slow2.bin", sizes / "slowrate.bin")
            cli("rate", slow, 32)
            cli("rate", fast, 512)
            wait_for(fast, lambda r: (r["bytes_per_second"] or 0) > 0)
            wait_for(slow, lambda r: r["status"] == "running" and (r["downloaded"] or 0) > 0)
            time.sleep(1.5)
            live = [t for t in page("-rate") if t in (fast, slow)]
            assert live == [fast, slow], (live, row(fast), row(slow))
            assert [t for t in page("+rate") if t in (fast, slow)] == [slow, fast]
            cli("pause", fast)
            cli("pause", slow)
            # Last try: never-started tasks sort before tasks that ran.
            cli("queue", "stop")
            never = rpc("submit_ex", {"request_id": "lt-never", "url": url("s-a.bin"), "output": str(sizes / "never.bin"),
                                      "priority": 1, "connections": 4})["task_id"]
            assert row(never)["status"] == "queued"
            tries = page("+last_try")
            assert tries.index(never) < tries.index(sid["big"]), tries
            assert page("-last_try").index(never) > page("-last_try").index(sid["big"])
            # Queue name: Alpha sorts before Zed regardless of the ids.
            zed = rpc("queue_create", {"name": "Zed", "limit": 1})["queue_id"]
            alpha = rpc("queue_create", {"name": "Alpha", "limit": 1})["queue_id"]
            assert rpc("assign_queue", {"queue_id": zed, "task_ids": [half]})["ok"]
            assert rpc("assign_queue", {"queue_id": alpha, "task_ids": [never]})["ok"]
            by_queue = page("+queue")
            assert by_queue.index(never) < by_queue.index(half), by_queue
            assert page("-queue").index(half) < page("-queue").index(never)
            assert not rpc("list_page", {"offset": 0, "limit": 5, "sort": "bogus"})["ok"]

            print(json.dumps({"result": "passed", "cases": [
                "N41 Last-Modified applied, opt-out, invalid date unchanged, resumed download",
                "N38 same-volume temp dir: partials, progress, resume, publish",
                "N38 delete_partial and PartialScan/Delete in the temp dir",
                "N38 cross-volume (/dev/shm) copy publish",
                "unknown-length EOF flagged size_unverified with notice",
                "MIME extension reconciliation and HTML-for-binary failure",
                "ListPage sort by size, progress, live rate, last try and queue name",
            ]}))
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            holder.shutdown()


if __name__ == "__main__":
    main()
