"""N23: manual proxy credentials live in the credential store; a 407 parks the task with the proxy realm."""
import base64
import hashlib
import http.client
import http.server
import json
import sqlite3
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon

USER, PASSWORD = "proxyuser", "proxy-secret-9"
EXPECTED_AUTH = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()
SIZE = 512 * 1024
BLOCK = bytes(range(256)) * (SIZE // 256)
LOG, LOCK = [], threading.Lock()


class Origin(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(SIZE))
        self.end_headers()
        self.wfile.write(BLOCK)


class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def denied(self):
        self.send_response(407)
        self.send_header("Proxy-Authenticate", 'Basic realm="xdm-proxy"')
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        auth = self.headers.get("Proxy-Authorization")
        with LOCK:
            LOG.append({"kind": "http", "auth": auth})
        if auth != EXPECTED_AUTH:
            return self.denied()
        target = urlsplit(self.path)
        upstream = http.client.HTTPConnection(target.hostname, target.port, timeout=30)
        upstream.request("GET", target.path or "/", headers={"Host": target.netloc})
        response = upstream.getresponse()
        body = response.read()
        self.send_response(response.status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_CONNECT(self):
        auth = self.headers.get("Proxy-Authorization")
        with LOCK:
            LOG.append({"kind": "connect", "auth": auth})
        if auth != EXPECTED_AUTH:
            return self.denied()
        # Credentials were right; refuse the tunnel itself (no TLS origin in this fixture).
        self.send_response(502)
        self.send_header("Content-Length", "0")
        self.end_headers()


def serve(handler):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main():
    origin, proxy = serve(Origin), serve(Proxy)
    url = f"http://127.0.0.1:{origin.server_port}/file.bin"
    proxy_url = f"http://127.0.0.1:{proxy.server_port}"
    daemon = Daemon("xdm-proxyauth-")
    try:
        daemon.cli("proxy", "manual", proxy_url)
        # Plain HTTP target: the 407 response carries the proxy realm.
        first = daemon.cli("submit", url, daemon.base / "a.bin")["task_id"]
        row = daemon.until(first, lambda t: t["status"] in ("auth_required", "failed"))
        assert row["status"] == "auth_required" and row["auth_proxy"] is True, row
        assert row["auth_realm"] == "xdm-proxy" and "407" in row["error"], row
        assert row["error_kind"] == "auth", row
        # The origin login path refuses to treat it as a site login.
        refused = daemon.cli("auth", first, "x", daemon.secret("x.txt", "y"), success=False)
        assert "proxy-login" in refused["error"], refused
        with LOCK:
            seen = len(LOG)
        import time
        time.sleep(0.8)
        with LOCK:
            assert len(LOG) == seen, "no retry loop on 407"

        # A wrong proxy password is tried and parked again.
        daemon.cli("proxy-login", USER, daemon.secret("bad.txt", "nope"))
        bad = "Basic " + base64.b64encode(f"{USER}:nope".encode()).decode()
        deadline = time.monotonic() + 30
        while not any(entry["auth"] == bad for entry in LOG) and time.monotonic() < deadline:
            time.sleep(0.1)
        row = daemon.until(first, lambda t: t["status"] == "auth_required")
        assert row["status"] == "auth_required" and row["auth_proxy"], row

        # The right password requeues every proxy-blocked task and completes it.
        daemon.cli("proxy-login", USER, daemon.secret("pw.txt", PASSWORD))
        daemon.until(first, lambda t: t["status"] == "complete")
        assert hashlib.sha256((daemon.base / "a.bin").read_bytes()).hexdigest() == hashlib.sha256(BLOCK).hexdigest()
        with LOCK:
            assert any(entry["auth"] == EXPECTED_AUTH for entry in LOG), LOG

        # HTTPS targets: a 407 to CONNECT (no headers visible to the client) also lands in auth_required.
        daemon.cli("proxy-login-clear")
        second = daemon.cli("submit", "https://127.0.0.1:9/x.bin", daemon.base / "b.bin")["task_id"]
        row = daemon.until(second, lambda t: t["status"] in ("auth_required", "failed"))
        assert row["status"] == "auth_required" and row["auth_proxy"], row
        daemon.cli("proxy-login", USER, daemon.secret("pw.txt", PASSWORD))
        row = daemon.until(second, lambda t: t["status"] in ("failed",))
        with LOCK:
            assert any(e["kind"] == "connect" and e["auth"] == EXPECTED_AUTH for e in LOG), LOG
        assert row["error_kind"] in ("network", "other", "connect_refused"), row

        # Credentials inside the proxy URL are moved to the store, not kept in settings.
        daemon.cli("proxy-login-clear")
        reply = daemon.rpc("set_proxy", proxy={"mode": "manual", "http": f"http://{USER}:{PASSWORD}@127.0.0.1:{proxy.server_port}", "https": None, "bypass": []})
        assert PASSWORD not in json.dumps(reply) and reply["proxy"]["http"] == proxy_url, reply
        third = daemon.cli("submit", url, daemon.base / "c.bin", "--allow-duplicate")["task_id"]
        daemon.until(third, lambda t: t["status"] == "complete")

        # Secrets are absent from settings, exports, listings and argv-visible output.
        daemon.cli("export", daemon.base / "export.json", "json", raw=True)
        texts = [(daemon.base / "export.json").read_text(), daemon.cli("proxy", "get", raw=True),
                 daemon.cli("list", raw=True)]
        for text in texts:
            assert PASSWORD not in text, text[:200]
        with sqlite3.connect(daemon.base / "state/tasks.sqlite3") as database:
            dump = json.dumps(database.execute("SELECT key,value FROM settings").fetchall())
            assert PASSWORD not in dump and "proxyuser" not in dump, dump
        for path in (daemon.base / "state").glob("tasks.sqlite3*"):
            assert PASSWORD.encode() not in path.read_bytes()
        store = daemon.base / "state/credentials.json"
        assert store.exists() and store.stat().st_mode & 0o777 == 0o600
        print(json.dumps({"result": "passed", "cases": [
            "407 -> auth_required with proxy realm", "proxy-login requeues and completes",
            "wrong proxy password parks again", "CONNECT 407 -> auth_required",
            "userinfo moved out of proxy URL", "no secrets in settings/exports"]}))
    finally:
        origin.shutdown()
        proxy.shutdown()
        daemon.close()


main()
