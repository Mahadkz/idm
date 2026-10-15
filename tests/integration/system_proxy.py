"""N26: `system` proxy mode imports GNOME (gsettings) and KDE (kioslaverc) settings and reports PAC as unsupported."""
import hashlib
import http.client
import http.server
import json
import os
import stat
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon

SIZE = 256 * 1024
BLOCK = bytes(range(256)) * (SIZE // 256)
HASH = hashlib.sha256(BLOCK).hexdigest()
HITS = {"origin": 0, "proxy": 0}
PROXY_AUTH = []


class Origin(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        HITS["origin"] += 1
        self.send_response(200)
        self.send_header("Content-Length", str(SIZE))
        self.end_headers()
        self.wfile.write(BLOCK)


class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        HITS["proxy"] += 1
        PROXY_AUTH.append(self.headers.get("Proxy-Authorization"))
        target = urlsplit(self.path)
        upstream = http.client.HTTPConnection(target.hostname, target.port, timeout=30)
        upstream.request("GET", target.path or "/")
        body = upstream.getresponse().read()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def serve(handler):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def gnome(mode, port=0, ignore="@as []", pac=""):
    return f"""org.gnome.system.proxy autoconfig-url '{pac}'
org.gnome.system.proxy ignore-hosts {ignore}
org.gnome.system.proxy mode '{mode}'
org.gnome.system.proxy.http host '127.0.0.1'
org.gnome.system.proxy.http port {port}
org.gnome.system.proxy.https host ''
org.gnome.system.proxy.https port 0
org.gnome.system.proxy.socks host ''
org.gnome.system.proxy.socks port 0
"""


def main():
    origin, proxy = serve(Origin), serve(Proxy)
    url = f"http://127.0.0.1:{origin.server_port}/file.bin"
    # Fake `gsettings` reading its answer from a file the test rewrites.
    import tempfile
    tools = Path(tempfile.mkdtemp(prefix="xdm-fakegs-"))
    answer = tools / "answer.txt"
    script = tools / "gsettings"
    script.write_text('#!/bin/sh\n[ "$1" = list-recursively ] && cat "$XDM_FAKE_GSETTINGS"\n')
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    home = tools / "home"
    (home / ".config").mkdir(parents=True)
    daemon = Daemon("xdm-sysproxy-", env_extra={
        "PATH": f"{tools}:{os.environ['PATH']}", "XDM_FAKE_GSETTINGS": str(answer),
        "HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config"),
        "XDM_SYSTEM_PROXY_TTL_MS": "0"})
    try:
        daemon.cli("proxy", "system")
        n = [0]

        def fetch():
            n[0] += 1
            task = daemon.cli("submit", url, daemon.base / f"f{n[0]}.bin", "--allow-duplicate")["task_id"]
            return daemon.until(task, lambda t: t["status"] in ("complete", "failed", "auth_required"))

        # GNOME manual proxy: traffic goes through the fixture proxy.
        answer.write_text(gnome("manual", proxy.server_port))
        row = fetch()
        assert row["status"] == "complete" and HITS["proxy"] >= 1, (row, HITS)
        assert hashlib.sha256((daemon.base / "f1.bin").read_bytes()).hexdigest() == HASH
        note = daemon.cli("proxy", "get")["proxy_note"]
        assert "gnome" in note and f"127.0.0.1:{proxy.server_port}" in note, note

        # ignore-hosts covering the target bypasses the proxy.
        hits = HITS["proxy"]
        answer.write_text(gnome("manual", proxy.server_port, ignore="['localhost', '127.0.0.0/8']"))
        row = fetch()
        assert row["status"] == "complete" and HITS["proxy"] == hits, (row, HITS)

        # mode 'none' connects directly.
        answer.write_text(gnome("none", proxy.server_port))
        row = fetch()
        assert row["status"] == "complete" and HITS["proxy"] == hits, (row, HITS)

        # Review 2: stored manual-proxy credentials are never sent to an OS-discovered proxy.
        daemon.cli("proxy-login", "someone", daemon.secret("pp.txt", "proxy-only-secret"))
        answer.write_text(gnome("manual", proxy.server_port))
        row = fetch()
        assert row["status"] == "complete" and PROXY_AUTH and all(a is None for a in PROXY_AUTH), PROXY_AUTH
        daemon.cli("proxy-login-clear")

        # PAC: reported clearly, never a silent direct connection.
        origin_hits = HITS["origin"]
        answer.write_text(gnome("auto", pac="http://wpad.example/proxy.pac"))
        row = fetch()
        assert row["status"] == "failed", row
        assert "PAC not supported" in row["error"] and "http://wpad.example/proxy.pac" in row["error"], row
        assert row["error_kind"] == "proxy_config", row
        assert HITS["origin"] == origin_hits, "a PAC setting must not fall back to direct"
        note = daemon.cli("proxy", "get")["proxy_note"]
        assert "PAC not supported" in note, note

        # Review 2: GNOME saying mode=none must not fall through to a stale kioslaverc.
        (home / ".config/kioslaverc").write_text(
            f"[Proxy Settings]\nProxyType=1\nhttpProxy=http://127.0.0.1:{proxy.server_port}\nNoProxyFor=\n")
        answer.write_text(gnome("none", proxy.server_port))
        before = HITS["proxy"]
        row = fetch()
        assert row["status"] == "complete" and HITS["proxy"] == before, (row, HITS)

        # No GNOME settings at all: KDE's kioslaverc is read.
        answer.write_text("")
        hits = HITS["proxy"]
        row = fetch()
        assert row["status"] == "complete" and HITS["proxy"] == hits + 1, (row, HITS)
        assert "kde" in daemon.cli("proxy", "get")["proxy_note"]

        # Review 12: the resolution is cached, so a burst of transfers spawns gsettings once.
        counter = tools / "count.txt"
        counting = tools / "bin2"
        counting.mkdir()
        fake = counting / "gsettings"
        fake.write_text(f'#!/bin/sh\necho x >> "{counter}"\ncat "{answer}"\n')
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        answer.write_text(gnome("manual", proxy.server_port))
        cached = Daemon("xdm-sysproxy2-", env_extra={"PATH": f"{counting}:{os.environ['PATH']}",
                        "HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config")})
        try:
            cached.cli("proxy", "system")
            for index in range(3):
                task = cached.cli("submit", url, cached.base / f"c{index}.bin", "--allow-duplicate")["task_id"]
                assert cached.until(task, lambda t: t["status"] in ("complete", "failed"))["status"] == "complete"
            assert len(counter.read_text().split()) == 1, counter.read_text()
        finally:
            cached.close()
        print(json.dumps({"result": "passed", "cases": [
            "gnome manual via gsettings", "ignore-hosts bypass", "mode none direct",
            "PAC reported as unsupported without going direct", "kde kioslaverc"]}))
    finally:
        origin.shutdown()
        proxy.shutdown()
        daemon.close()


main()
