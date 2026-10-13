"""POST request replay (method + body), proxy routing and their persistence."""
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
EXPECTED = hashlib.sha256(bytes(range(256)) * (4 * 1024 * 1024 // 256)).hexdigest()
BODY = "token=abc"


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-request-context-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "tests/integration/http_fixture.py"),
                                    str(base / "http-log"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        proxy_fixture = subprocess.Popen([sys.executable, str(ROOT / "tests/integration/proxy_fixture.py"),
                                          str(base / "proxy-log")],
                                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=10)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            if args[0] == "export":
                return result.stdout
            return json.loads(result.stdout) if result.stdout.strip() else None

        def rpc(kind, **payload):
            request = {"version": 1, "token": (base / "state/ipc-token").read_text(),
                       "command": {"type": kind, "payload": payload}}
            data = json.dumps(request).encode()
            with socket.socket(socket.AF_UNIX) as sock:
                sock.settimeout(10)
                sock.connect(str(base / "state/daemon.sock"))
                sock.sendall(struct.pack("=I", len(data)) + data)
                with sock.makefile("rb") as reader:
                    size = struct.unpack("=I", reader.read(4))[0]
                    return json.loads(reader.read(size))

        def submit_post(request_id, filename, method="POST", body=BODY):
            return rpc("submit_request", request_id=request_id,
                       url=f"http://127.0.0.1:{port}/{filename}",
                       output=str(base / filename),
                       headers=[["Content-Type", "application/x-www-form-urlencoded"]],
                       method=method, body=list(body.encode()) if body is not None else None,
                       priority=1, connections=4)

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def info(task_id):
            return cli("info", task_id)["tasks"][0]

        def until(task_id, predicate, timeout=25):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                row = info(task_id)
                if predicate(row):
                    return row
                time.sleep(0.1)
            raise AssertionError(row)

        def http_log(path):
            entries = [json.loads(line) for line in (base / "http-log").read_text().splitlines()]
            return [entry for entry in entries if entry["path"] == path]

        def proxy_log():
            if not (base / "proxy-log").exists():
                return []
            return [json.loads(line) for line in (base / "proxy-log").read_text().splitlines()]

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            proxy_ports = json.loads(proxy_fixture.stdout.readline())
            daemon = start()
            cli("queue", "stop")

            # A GET of /post.bin is refused by the fixture and the task fails.
            get_fail = cli("submit", f"http://127.0.0.1:{port}/post.bin", base / "get-fail.bin")["task_id"]
            cli("queue", "start")
            row = until(get_fail, lambda row: row["status"] == "failed")
            assert "405" in (row["error"] or ""), row
            assert not (base / "get-fail.bin").exists()
            cli("queue", "stop")

            # POST with a replayable form body completes with the fixture content.
            submitted = submit_post("post-primary", "post.bin")
            assert submitted["ok"], submitted
            post_id = submitted["task_id"]
            cli("queue", "start")
            until(post_id, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "post.bin").read_bytes()).hexdigest() == EXPECTED
            post_attempts = http_log("/post.bin")
            assert any(entry["method"] == "POST" and entry["body"] == BODY for entry in post_attempts)
            assert any(entry["method"] == "GET" for entry in post_attempts)
            cli("queue", "stop")

            # Validation: bodies need a non-GET method; methods must be HTTP tokens.
            assert not submit_post("bad-get", "get-body.bin", method="GET")["ok"]
            assert not submit_post("bad-method", "bad-method.bin", method="bad method")["ok"]
            assert not (base / "get-body.bin").exists() and not (base / "bad-method.bin").exists()

            # Redownload replays the same method and body; exports omit bodies.
            cloned = rpc("redownload", task_id=post_id, request_id="post-clone",
                         output=str(base / "post-copy.bin"))["task_id"]
            cli("queue", "start")
            until(cloned, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "post-copy.bin").read_bytes()).hexdigest() == EXPECTED
            cli("export", base / "export.json", "json")
            export_text = (base / "export.json").read_text()
            assert "token=" not in export_text and '"headers"' not in export_text
            assert any(entry["filename"] == "post.bin" for entry in json.loads(export_text)["entries"])
            cli("queue", "stop")

            # A queued POST survives a daemon restart with its method and body.
            held = submit_post("post-held", "held-post.bin", body=BODY)["task_id"]
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            with sqlite3.connect(base / "state/tasks.sqlite3") as database:
                method, body = database.execute("SELECT method,body FROM tasks WHERE id=?", (held,)).fetchone()
                assert (method, body) == ("POST", BODY.encode())
            cli("queue", "start")
            until(held, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "held-post.bin").read_bytes()).hexdigest() == EXPECTED
            cli("queue", "stop")

            # CLI submit-post reads the body from a file.
            body_file = base / "request.body"
            body_file.write_text(BODY)
            cli_post = cli("submit-post", f"http://127.0.0.1:{port}/post.bin",
                           base / "cli-post.bin", body_file)["task_id"]
            cli("queue", "start")
            until(cli_post, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "cli-post.bin").read_bytes()).hexdigest() == EXPECTED
            cli("queue", "stop")

            # Proxy: the default route is system; manual routing is visible in the proxy log.
            assert cli("proxy", "get")["proxy"]["mode"] == "system"
            proxy_url = f"http://127.0.0.1:{proxy_ports['http']}"
            cli("proxy", "manual", proxy_url, "10.0.0.0/8")
            assert cli("proxy", "get")["proxy"] == {"mode": "manual", "http": proxy_url,
                                                    "https": proxy_url, "bypass": ["10.0.0.0/8"]}
            manual = cli("submit", f"http://127.0.0.1:{port}/manual.bin", base / "manual.bin")["task_id"]
            cli("queue", "start")
            until(manual, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "manual.bin").read_bytes()).hexdigest() == EXPECTED
            assert any(entry["kind"] == "http" and entry["url"].endswith("/manual.bin")
                       for entry in proxy_log())
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert cli("proxy", "get")["proxy"]["mode"] == "manual"
            cli("proxy", "direct")
            direct = cli("submit", f"http://127.0.0.1:{port}/direct.bin", base / "direct.bin")["task_id"]
            until(direct, lambda row: row["status"] == "complete")
            assert hashlib.sha256((base / "direct.bin").read_bytes()).hexdigest() == EXPECTED
            assert not any(entry.get("url", "").endswith("/direct.bin") for entry in proxy_log())
            cli("proxy", "manual", "ftp://proxy.invalid", success=False)
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert cli("proxy", "get")["proxy"]["mode"] == "direct"
            cli("queue", "stop")
            print("POST replay with correct hash, GET refusal, method/body validation, "
                  "restart persistence, CLI submit-post and persisted proxy routing passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)
            proxy_fixture.terminate(); proxy_fixture.wait(timeout=5)


if __name__ == "__main__":
    main()
