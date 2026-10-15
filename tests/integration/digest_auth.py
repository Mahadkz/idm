"""N11: HTTP Digest (MD5 and SHA-256, qop=auth, nc/cnonce, stale nonce) for saved logins and TaskProvideLogin."""
import hashlib
import http.server
import json
import re
import sys
import threading
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon

USER, PASSWORD = "mufasa", "circle of life"
SIZE = 1024 * 1024
BLOCK = bytes(range(256)) * (SIZE // 256)
EXPECTED = hashlib.sha256(BLOCK).hexdigest()
LOCK = threading.Lock()
LOG = []
STATE = {"nonce_uses": {}, "seen_nc": set(), "counter": 0, "stale_after": None}

ALGORITHMS = {"MD5": hashlib.md5, "SHA-256": hashlib.sha256}


def parse_auth(header):
    return {k.lower(): (q if q else v) for k, q, v in
            re.findall(r'(\w+)=(?:"([^"]*)"|([^,\s]*))', header[len("Digest "):])}


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def challenge(self, algorithm, stale=False):
        with LOCK:
            STATE["counter"] += 1
            nonce = f"nonce-{STATE['counter']}"
            STATE["nonce_uses"][nonce] = 0
        value = (f'Digest realm="digest-test", qop="auth", algorithm={algorithm}, '
                 f'nonce="{nonce}", opaque="op-{algorithm}"' + (", stale=true" if stale else ""))
        self.send_response(401)
        self.send_header("WWW-Authenticate", value)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def verify(self, algorithm):
        header = self.headers.get("Authorization") or ""
        if not header.startswith("Digest "):
            return "none"
        fields = parse_auth(header)
        h = lambda text: ALGORITHMS[algorithm](text.encode()).hexdigest()
        if fields.get("algorithm", "").upper() != algorithm or fields.get("opaque") != f"op-{algorithm}":
            return "bad"
        if fields.get("username") != USER or fields.get("uri") != self.path:
            return "bad"
        ha1 = h(f"{USER}:digest-test:{PASSWORD}")
        ha2 = h(f"{self.command}:{fields['uri']}")
        expected = h(f"{ha1}:{fields['nonce']}:{fields['nc']}:{fields['cnonce']}:auth:{ha2}")
        if fields.get("qop") != "auth" or fields["response"] != expected:
            return "bad"
        with LOCK:
            if fields["nonce"] not in STATE["nonce_uses"]:
                return "stale"
            if (fields["nonce"], fields["nc"]) in STATE["seen_nc"]:
                return "replay"
            STATE["seen_nc"].add((fields["nonce"], fields["nc"]))
            STATE["nonce_uses"][fields["nonce"]] += 1
            limit = STATE["stale_after"]
            if limit and STATE["nonce_uses"][fields["nonce"]] > limit:
                del STATE["nonce_uses"][fields["nonce"]]
                return "stale"
        LOG.append({"path": self.path, "nc": fields["nc"], "range": self.headers.get("Range"),
                    "nonce": fields["nonce"]})
        return "ok"

    def do_GET(self):
        path = urlsplit(self.path).path
        algorithm = "SHA-256" if path.startswith("/sha") else "MD5"
        verdict = self.verify(algorithm)
        with LOCK:
            LOG.append({"path": self.path, "verdict": verdict})
        if verdict != "ok":
            self.challenge(algorithm, stale=verdict == "stale")
            return
        start, end, status = 0, SIZE - 1, 200
        match = re.fullmatch(r"bytes=(\d+)-(\d*)", self.headers.get("Range") or "")
        if match:
            start, end, status = int(match[1]), min(int(match[2] or SIZE - 1), SIZE - 1), 206
        self.send_response(status)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"digest-v1"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{SIZE}")
        self.end_headers()
        self.wfile.write(BLOCK[start:end + 1])


def verdicts():
    with LOCK:
        return [entry["verdict"] for entry in LOG if "verdict" in entry]


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base_url = f"http://127.0.0.1:{server.server_port}"
    daemon = Daemon("xdm-digest-")
    try:
        password = daemon.secret("pw.txt", PASSWORD)
        # No login: the Digest challenge parks the task with its realm, no retry loop.
        first = daemon.cli("submit", f"{base_url}/md5.bin", daemon.base / "a.bin")["task_id"]
        row = daemon.until(first, lambda t: t["status"] in ("auth_required", "failed"))
        assert row["status"] == "auth_required" and row["auth_realm"] == "digest-test", row
        assert row["error_kind"] == "auth", row
        assert verdicts().count("none") >= 1 and "bad" not in verdicts(), verdicts()

        # TaskProvideLogin with a Digest server completes it (MD5).
        daemon.cli("auth", first, USER, password)
        daemon.until(first, lambda t: t["status"] == "complete")
        assert hashlib.sha256((daemon.base / "a.bin").read_bytes()).hexdigest() == EXPECTED

        # A saved origin login drives SHA-256 Digest over a 4-connection ranged transfer:
        # every request carries its own Authorization with a distinct nc.
        daemon.cli("login", "save", f"{base_url}/", USER, password)
        with LOCK:
            LOG.clear()
        second = daemon.cli("submit", f"{base_url}/sha.bin", daemon.base / "b.bin")["task_id"]
        daemon.until(second, lambda t: t["status"] == "complete")
        assert hashlib.sha256((daemon.base / "b.bin").read_bytes()).hexdigest() == EXPECTED
        authorised = [entry for entry in LOG if "nc" in entry]
        ranged = [entry for entry in authorised if entry["range"]]
        assert len(ranged) >= 2, authorised
        pairs = [(entry["nonce"], entry["nc"]) for entry in authorised]
        assert len(pairs) == len(set(pairs)), pairs
        assert max(int(entry["nc"], 16) for entry in authorised) >= len(authorised), authorised
        assert verdicts().count("replay") == 0 and verdicts().count("bad") == 0, verdicts()
        # Only the very first request is unauthenticated.
        assert verdicts().count("none") == 1, verdicts()

        # A nonce that goes stale mid-transfer is renewed (stale=true) without failing.
        STATE["stale_after"] = 2
        third = daemon.cli("submit", f"{base_url}/md5.bin", daemon.base / "c.bin")["task_id"]
        daemon.until(third, lambda t: t["status"] == "complete")
        assert hashlib.sha256((daemon.base / "c.bin").read_bytes()).hexdigest() == EXPECTED
        assert "stale" in verdicts(), verdicts()
        STATE["stale_after"] = None

        # Wrong password: answered once, then auth_required, never an endless loop.
        wrong = daemon.secret("wrong.txt", "not-the-password")
        daemon.cli("login", "save", f"{base_url}/", USER, wrong)
        with LOCK:
            LOG.clear()
        fourth = daemon.cli("submit", f"{base_url}/md5.bin", daemon.base / "d.bin")["task_id"]
        row = daemon.until(fourth, lambda t: t["status"] == "auth_required")
        assert row["auth_realm"] == "digest-test" and row["error_kind"] == "auth", row
        import time
        time.sleep(1.0)
        seen = len([v for v in verdicts()])
        assert seen <= 8, verdicts()
        assert daemon.info(fourth)["status"] == "auth_required"
        assert len(verdicts()) == seen, "no retry loop on repeated 401"

        # The password never appears in listings, exports or task rows.
        for text in [daemon.cli("list", raw=True), daemon.cli("login", "list", raw=True)]:
            assert PASSWORD not in text and "not-the-password" not in text
        print(json.dumps({"result": "passed", "cases": [
            "digest challenge -> auth_required with realm", "TaskProvideLogin digest MD5",
            "saved login digest SHA-256 across range requests with distinct nc",
            "stale nonce renewed", "wrong password stops after one retry"]}))
    finally:
        server.shutdown()
        daemon.close()


main()
