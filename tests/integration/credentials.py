"""Site logins: Basic auth challenge, saved credentials, auth_required flow and TaskProvideLogin."""
import base64
import hashlib
import http.server
import json
import os
import re
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
USER, PASSWORD = "alice", "secret123"
WRONG = "wrong-password"
AUTH = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()
SIZE = 256 * 4096
BLOCK = bytes(range(256)) * 4096
LOG = {"requests": [], "lock": threading.Lock()}


class Secure(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        path = urlsplit(self.path).path
        authorization = self.headers.get("Authorization")
        with LOG["lock"]:
            LOG["requests"].append({"path": path, "authorization": authorization})
        if path == "/secure.bin" and authorization != AUTH:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="xdm-test"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        start, end, status = 0, SIZE - 1, 200
        match = re.fullmatch(r"bytes=(\d+)-(\d*)", self.headers.get("Range") or "")
        if match:
            start = int(match[1])
            end = min(int(match[2]) if match[2] else SIZE - 1, SIZE - 1)
            if start > end:
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{SIZE}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            status = 206
        self.send_response(status)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"secure-v1"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{SIZE}")
        self.end_headers()
        self.wfile.write(BLOCK[start:end + 1])


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-credentials-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Secure)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        port = server.server_port
        daemon = None

        def cli(*args, success=True, raw=False):
            result = subprocess.run(
                [str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                env=env, capture_output=True, text=True, timeout=15,
            )
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            if raw:
                return result.stdout
            return json.loads(result.stdout) if result.stdout.strip() else {}

        def rpc(kind, **payload):
            command = {"type": kind}
            if payload:
                command["payload"] = payload
            request = {"version": 1, "token": (base / "state/ipc-token").read_text(),
                       "command": command}
            data = json.dumps(request).encode()
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(15)
                stream.connect(str(base / "state/daemon.sock"))
                stream.sendall(struct.pack("=I", len(data)) + data)
                with stream.makefile("rb") as reader:
                    size = struct.unpack("=I", reader.read(4))[0]
                    return json.loads(reader.read(size))

        def info(task_id):
            return cli("info", task_id)["tasks"][0]

        def until(task_id, predicate, timeout=30):
            deadline = time.monotonic() + timeout
            row = info(task_id)
            while time.monotonic() < deadline:
                if predicate(row):
                    return row
                time.sleep(0.1)
                row = info(task_id)
            raise AssertionError(row)

        def requests():
            with LOG["lock"]:
                return list(LOG["requests"])

        def digest(path):
            return hashlib.sha256(Path(path).read_bytes()).hexdigest()

        try:
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/xdm-daemon")], env=env,
                stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            url = f"http://127.0.0.1:{port}/secure.bin"
            password_file = base / "pw.txt"
            password_file.write_text(PASSWORD + "\n")

            # No login yet: the challenge becomes auth_required with the realm,
            # not failed, and the queue retry policy never touches it.
            first = cli("submit", url, base / "one.bin")["task_id"]
            row = until(first, lambda task: task["status"] in ("auth_required", "failed"))
            assert row["status"] == "auth_required", row
            assert row["auth_realm"] == "xdm-test", row
            assert "401" in (row["error"] or ""), row
            assert not any(entry["authorization"] for entry in requests()), row
            time.sleep(0.6)
            assert info(first)["status"] == "auth_required", "no retry loop on 401"

            # A login for another host is never sent to this server.
            other = cli("login", "save", "http://other.example.com/", USER, str(password_file))
            other_id = other["logins"][0]["id"]
            held = cli("submit", url, base / "held.bin")["task_id"]
            until(held, lambda task: task["status"] == "auth_required")
            assert not any(entry["authorization"] for entry in requests())
            cli("login", "remove", other_id)

            # Saving a login leaks nothing to list/CLI output.
            saved = cli("login", "save", f"http://127.0.0.1:{port}/", USER, str(password_file))
            assert saved["logins"][0]["username"] == USER
            listing = cli("login", "list", raw=True)
            assert USER in listing and f"http://127.0.0.1:{port}/" in listing
            assert PASSWORD not in listing and WRONG not in listing
            assert PASSWORD not in cli("list", raw=True)
            cli("login", "save", "http://bad host/", USER, str(password_file), success=False)

            # Providing credentials through the IPC requeues and completes.
            provided = cli("auth", first, USER, str(password_file))
            assert provided["ok"] and PASSWORD not in json.dumps(provided), provided
            until(first, lambda task: task["status"] == "complete")
            assert digest(base / "one.bin") == EXPECTED_HASH

            # The saved origin login is applied automatically to a fresh task;
            # every request then carries exactly the right Basic header.
            before = len(requests())
            second = cli("submit", url, base / "two.bin")["task_id"]
            until(second, lambda task: task["status"] == "complete")
            assert digest(base / "two.bin") == EXPECTED_HASH
            applied = requests()[before:]
            assert applied and all(entry["authorization"] == AUTH for entry in applied), applied

            # A wrong password on a more specific path is applied, rejected and
            # recorded as auth_required again; removing it and providing the
            # right credentials with --remember restores the task.
            wrong_file = base / "wrong.txt"
            wrong_file.write_text(WRONG + "\n")
            rejected = cli("login", "save", f"http://127.0.0.1:{port}/secure.bin", USER, str(wrong_file))
            bad_id = next(row["id"] for row in rejected["logins"] if row["pattern"].endswith("/secure.bin"))
            third = cli("submit", url, base / "three.bin")["task_id"]
            row = until(third, lambda task: task["status"] == "auth_required", timeout=30)
            assert row["auth_realm"] == "xdm-test", row
            assert next(entry for entry in requests() if entry["authorization"] == "Basic " + base64.b64encode(f"{USER}:{WRONG}".encode()).decode())
            cli("login", "remove", bad_id)
            remembered = cli("auth", third, USER, str(password_file), "--remember")
            assert remembered["ok"] and PASSWORD not in json.dumps(remembered), remembered
            until(third, lambda task: task["status"] == "complete")
            assert digest(base / "three.bin") == EXPECTED_HASH

            # Disabled logins never authenticate; with the origin login off,
            # no Authorization is sent and the task waits again.
            disabled = rpc("login_save", pattern=f"http://127.0.0.1:{port}/",
                           username=USER, password=PASSWORD, enabled=False)
            assert disabled["ok"], disabled
            fourth = cli("submit", url, base / "four.bin")["task_id"]
            row = until(fourth, lambda task: task["status"] == "auth_required")
            assert row["auth_realm"] == "xdm-test", row
            assert row["retry_count"] == 0, row
            # Re-enabling the origin login (same pattern and username) lets the
            # provided credentials finish the task.
            enabled = rpc("login_save", pattern=f"http://127.0.0.1:{port}/",
                          username=USER, password=PASSWORD, enabled=True)
            assert enabled["ok"], enabled
            assert any(row["enabled"] for row in enabled["logins"]), enabled
            provided = cli("auth", fourth, USER, str(password_file))
            until(fourth, lambda task: task["status"] == "complete")
            assert digest(base / "four.bin") == EXPECTED_HASH

            # Providing a login for a task that is not waiting is refused.
            refused = cli("auth", fourth, USER, str(password_file), success=False)
            assert not refused["ok"], refused
            # Disable every saved login, then supply them again through stdin
            # with --remember, which restores (and corrects) the origin entry.
            for entry in rpc("login_list")["logins"]:
                assert rpc("login_save", pattern=entry["pattern"], username=entry["username"],
                           password=WRONG, enabled=False)["ok"], entry
            cutoff = len(requests())
            fresh = cli("submit", url, base / "five.bin")["task_id"]
            until(fresh, lambda task: task["status"] == "auth_required")
            assert not any(entry["authorization"] for entry in requests()[cutoff:]), \
                "disabled logins are never sent"
            stdin = subprocess.run(
                [str(ROOT / "target/debug/xdm-cli"), "daemon", "auth", str(fresh), USER, "-", "--remember"],
                input=f"{PASSWORD}\n", env=env, text=True, capture_output=True, timeout=15)
            assert stdin.returncode == 0, stdin.stderr
            assert PASSWORD not in stdin.stdout
            until(fresh, lambda task: task["status"] == "complete")
            assert digest(base / "five.bin") == EXPECTED_HASH

            # Passwords live only in the owner-only store next to the database.
            store = base / "state/credentials.json"
            assert store.exists() and os.stat(store).st_mode & 0o777 == 0o600
            exported = base / "list.json"
            cli("export", exported, "json", raw=True)
            exported_text = json.dumps(json.loads(exported.read_text()))
            for text in [exported_text, cli("login", "list", raw=True), cli("list", raw=True)]:
                assert PASSWORD not in text and WRONG not in text
            for database in sorted((base / "state").glob("tasks.sqlite3*")):
                assert PASSWORD.encode() not in database.read_bytes()
            with sqlite3.connect(store.parent / "tasks.sqlite3") as database:
                columns = {row[1] for row in database.execute("PRAGMA table_info(tasks)")}
                assert "auth_realm" in columns
                # Every task finished except the deliberately parked one, which
                # waits in auth_required after its foreign login was removed.
                parked = database.execute(
                    "SELECT status, auth_realm FROM tasks WHERE status='auth_required'").fetchall()
                assert parked == [("auth_required", "xdm-test")], parked
                assert database.execute(
                    "SELECT COUNT(*) FROM tasks WHERE status='complete'").fetchone()[0] == 5
            for row in cli("list")["tasks"]:
                assert row["status"] in ("complete", "auth_required"), row
            print(json.dumps({"result": "passed", "cases": [
                "401 challenge -> auth_required with realm, no retry loop",
                "foreign-host login never sent", "login save/list without passwords",
                "TaskProvideLogin requeues and completes", "saved login applied automatically",
                "wrong password rejected then removed", "remember stores the origin pattern",
                "disabled logins never authenticate", "stdin password entry",
                "credentials.json mode 0600, absent from exports and list"]}))
        finally:
            server.shutdown()
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)


EXPECTED_HASH = hashlib.sha256(BLOCK).hexdigest()

if __name__ == "__main__":
    main()
