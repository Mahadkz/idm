"""Q01/Q10 synchronization queues and the duplicate-URL policy for imports, batches and redownloads."""
import hashlib
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
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"
SIZE = 300 * 1024

lock = threading.Lock()
STATE = {"version": 1, "mode": "ok", "requests": []}


def body_for(version):
    return (bytes([version]) + bytes(range(255))) * (SIZE // 256)


def digest(version):
    return hashlib.sha256(body_for(version)).hexdigest()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *unused):
        pass

    def reply(self, head):
        path = self.path.split("?")[0]
        with lock:
            STATE["requests"].append((self.command, path))
            version, mode = STATE["version"], STATE["mode"]
        if not path.startswith("/obj"):
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if mode == "down" or (mode == "getfail" and not head):
            self.send_response(503 if mode == "down" else 500)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        etag = f'"v{version}"'
        if self.headers.get("If-None-Match") == etag:
            self.send_response(304)
            self.send_header("ETag", etag)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        data = body_for(version)
        start, end, status = 0, len(data) - 1, 200
        value = self.headers.get("Range")
        if value and value.startswith("bytes="):
            first, second = value[6:].split("-")
            start, end, status = int(first), int(second) if second else len(data) - 1, 206
        self.send_response(status)
        self.send_header("ETag", etag)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        if not head:
            self.wfile.write(data[start:end + 1])

    def do_HEAD(self):
        self.reply(True)

    def do_GET(self):
        self.reply(False)


def exchange(conn, payload):
    data = json.dumps(payload).encode()
    conn.write(struct.pack("=I", len(data)) + data)
    conn.flush()
    size = struct.unpack("=I", conn.read(4))[0]
    return json.loads(conn.read(size))


def count(method=None):
    with lock:
        return sum(1 for m, p in STATE["requests"] if p.startswith("/obj") and (method is None or m == method))


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-sync-") as temp:
        base = Path(temp)
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        url = lambda name: f"http://127.0.0.1:{server.server_port}/{name}"
        env = os.environ.copy()
        env.update({"XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")})
        (base / "home").mkdir()
        daemon = subprocess.Popen([str(DAEMON)], env=env, stderr=subprocess.PIPE, text=True)
        try:
            assert "listening locally" in daemon.stderr.readline()

            def cli(*args, success=True):
                result = subprocess.run([str(CLI), "daemon", *map(str, args)], env=env, text=True,
                                        capture_output=True, timeout=20)
                assert (result.returncode == 0) == success, (args, result.returncode, result.stderr, result.stdout)
                return json.loads(result.stdout) if result.stdout.strip() else None

            def rpc(kind, payload):
                with socket.socket(socket.AF_UNIX) as raw:
                    raw.connect(str(base / "state/daemon.sock"))
                    with raw.makefile("rwb") as conn:
                        token = (base / "state/ipc-token").read_text().strip()
                        return exchange(conn, {"version": 1, "token": token,
                                               "command": {"type": kind, "payload": payload}})

            def wait_for(predicate, what, timeout=30):
                deadline = time.monotonic() + timeout
                value = None
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(0.1)
                raise AssertionError(what)

            def task(task_id):
                return cli("info", task_id)["tasks"][0]

            def events(queue_id):
                return cli("queue", "events", queue_id)["queue_events"]

            def run_check(queue_id):
                before = sum(1 for e in events(queue_id) if e["outcome"] == "sync_checked")
                cli("queue", "check", queue_id)
                wait_for(lambda: sum(1 for e in events(queue_id) if e["outcome"] == "sync_checked") > before,
                         "sync check finished")
                evs = events(queue_id)
                last = max(e["execution"] for e in evs)
                return [e for e in evs if e["execution"] == last]

            sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()

            # --- Ordinary queues are unaffected: a plain task still leaves its queue.
            out = base / "files"
            out.mkdir()
            cli("queue", "stop")
            plain = cli("submit", url("plain.bin"), out / "plain.bin")["task_id"]
            cli("queue", "start")
            wait_for(lambda: task(plain)["status"] == "complete", "plain complete")
            assert cli("queue", "create", "ordinary", 2)["ok"]

            # --- Synchronization queue creation and rejection of misuse.
            made = cli("queue", "create", "Sync", 2, "--sync")
            qid = next(q["id"] for q in made["queues"] if q["name"] == "Sync")
            assert next(q for q in made["queues"] if q["id"] == qid)["sync"] is True
            ordinary = next(q["id"] for q in made["queues"] if q["name"] == "ordinary")
            cli("queue", "check", ordinary, success=False)
            cli("queue", "sync-interval", ordinary, 1, 0, success=False)
            cli("queue", "stop")

            member_path = out / "member.bin"
            member = cli("submit", url("obj/a"), member_path)["task_id"]
            assert cli("queue", "assign", qid, member)["ok"]
            cli("queue", "start-one", qid)
            wait_for(lambda: task(member)["status"] == "complete", "member complete")
            assert sha(member_path) == digest(1)
            summary = next(q for q in cli("list")["queues"] if q["id"] == qid)
            assert summary["complete"] == 1, summary
            # A completed member stays in the queue, so the queue cannot be deleted.
            assert not cli("queue", "remove", qid, success=False)["ok"]
            # A completed ordinary task may be moved in and becomes a member too.
            assert cli("queue", "assign", qid, plain)["ok"]
            cli("queue", "assign", 1, plain)

            # --- First check establishes the baseline: probe only, no download.
            gets = count("GET")
            run = run_check(qid)
            assert [e["outcome"] for e in run if e["action"] == f"task:{member}"] == ["sync_unchanged"], run
            assert count("GET") == gets, "baseline check must not download"
            assert sha(member_path) == digest(1)

            # --- Unchanged: conditional probe, still no body request.
            heads = count("HEAD")
            run = run_check(qid)
            assert any(e["outcome"] == "sync_unchanged" for e in run), run
            assert count("HEAD") > heads and count("GET") == gets, STATE["requests"]
            summary_event = next(e for e in run if e["outcome"] == "sync_checked")
            assert "unchanged=1 updated=0 failed=0" in summary_event["action"], summary_event

            # --- Changed remote object: re-downloaded and replaced atomically.
            with lock:
                STATE["version"] = 2
            run = run_check(qid)
            assert [e["outcome"] for e in run if e["action"] == f"task:{member}"] == ["sync_updated"], run
            assert count("GET") > gets
            assert sha(member_path) == digest(2)
            assert sorted(p.name for p in out.iterdir()) == ["member.bin", "plain.bin"], list(out.iterdir())
            assert task(member)["status"] == "complete"
            gets = count("GET")
            run = run_check(qid)
            assert any(e["outcome"] == "sync_unchanged" for e in run) and count("GET") == gets

            # --- Failed refresh keeps the old file and leaves no temp files.
            with lock:
                STATE["version"], STATE["mode"] = 3, "getfail"
            run = run_check(qid)
            failure = next(e for e in run if e["action"] == f"task:{member}")
            assert failure["outcome"] == "sync_failed", run
            assert sha(member_path) == digest(2)
            assert sorted(p.name for p in out.iterdir()) == ["member.bin", "plain.bin"], list(out.iterdir())
            with lock:
                STATE["mode"] = "down"
            run = run_check(qid)
            assert next(e for e in run if e["action"] == f"task:{member}")["outcome"] == "sync_failed", run
            assert sha(member_path) == digest(2)
            # Once the source returns, the pending version 3 is picked up.
            with lock:
                STATE["mode"] = "ok"
            run = run_check(qid)
            assert next(e for e in run if e["action"] == f"task:{member}")["outcome"] == "sync_updated", run
            assert sha(member_path) == digest(3)

            # --- Periodic check without a manual command.
            before = sum(1 for e in events(qid) if e["outcome"] == "sync_checked")
            reply = cli("queue", "sync-interval-seconds", qid, 1)
            assert next(q for q in reply["queues"] if q["id"] == qid)["sync_interval"] == 1
            wait_for(lambda: sum(1 for e in events(qid) if e["outcome"] == "sync_checked") >= before + 2,
                     "periodic checks")
            reply = cli("queue", "sync-interval", qid, "off")
            assert next(q for q in reply["queues"] if q["id"] == qid)["sync_interval"] is None
            reply = cli("queue", "sync-interval", qid, 2, 30)
            assert next(q for q in reply["queues"] if q["id"] == qid)["sync_interval"] == 9000
            cli("queue", "sync-interval", qid, "off")
            time.sleep(0.5)

            # --- Duplicate policy: import skips duplicates and reports them.
            entry = lambda name, file: {"url": url(name), "filename": file, "kind": "http", "priority": 1,
                                        "connections": 2, "rate_limit": None, "video": None, "audio": None,
                                        "category": "Other"}
            imp_dir = base / "imp"
            payload = {"request_id": "imp-1", "directory": str(imp_dir), "enqueue": False,
                       "entries": [entry("obj/a", "dup-of-member.bin"), entry("obj/new", "new.bin"),
                                   entry("obj/new#frag", "new-again.bin")]}
            receipt = rpc("import_list", payload)
            assert receipt["ok"], receipt
            assert len(receipt["task_ids"]) == 1, receipt
            skipped = receipt["skipped_duplicates"]
            assert [s["index"] for s in skipped] == [0, 2], skipped
            assert skipped[0]["duplicate_of"] == member and skipped[1]["duplicate_of_index"] == 1, skipped
            again = rpc("import_list", payload)
            assert again["task_ids"] == receipt["task_ids"] and again["skipped_duplicates"] == skipped, again
            # Batch import: second submission of the same pattern is all duplicates.
            batch_dir = base / "batch"
            first = cli("batch", url("obj/b[1-3]"), batch_dir)
            assert len(first["task_ids"]) == 3 and first["skipped_duplicates"] == [], first
            second = cli("batch", url("obj/b[2-4]"), batch_dir)
            assert len(second["task_ids"]) == 1 and len(second["skipped_duplicates"]) == 2, second

            # --- Redownload honours the duplicate policy.
            copy = rpc("submit_ex", {"request_id": "copy-1", "url": url("obj/a"),
                                     "output": str(base / "copy.bin"), "headers": [], "method": "GET",
                                     "body": None, "priority": 1, "connections": 2, "allow_duplicate": True})
            assert copy["ok"], copy
            refused = cli("redownload", member, base / "again.bin", success=False)
            assert refused["duplicate_of"] == copy["task_id"], refused
            allowed = cli("redownload", member, base / "again.bin", "--allow-duplicate")
            assert allowed["ok"] and allowed["task_id"], allowed

            print(json.dumps({"result": "passed", "cases": [
                "completed members stay in synchronization queues; ordinary queues unchanged",
                "baseline and unchanged checks probe without downloading",
                "changed object replaced atomically with the new hash",
                "failed refresh keeps the old file and leaves no temp files",
                "periodic interval checks",
                "duplicate import/batch entries skipped and reported; redownload duplicate policy",
            ]}))
        finally:
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            server.shutdown()


if __name__ == "__main__":
    main()
