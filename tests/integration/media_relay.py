"""Private loopback media relay: ffmpeg/ffprobe argv never carries cookies, tokens or
passwords; captured headers and saved logins reach only their own origin; the relay
needs its random path token and closes with the job; Basic and Digest logins work."""
import base64
import hashlib
import http.server
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
BIN = Path.home() / ".local/bin"
os.environ["PATH"] = f"{BIN}:{os.environ['PATH']}"
COOKIE, BEARER = "sid=cookie-secret-4711", "Bearer token-secret-9001"
USER, PASSWORD = "reluser", "relay-pass-8842"
LOG, LOCK = [], threading.Lock()
SEGMENTS = {}


def make_segments(folder):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                    "testsrc2=size=320x180:rate=25", "-f", "lavfi", "-i", "sine=frequency=440",
                    "-t", "4", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                    "-g", "25", "-c:a", "aac", "-f", "segment", "-segment_time", "2",
                    str(folder / "seg%d.ts")], check=True, timeout=120)
    for f in folder.glob("seg*.ts"):
        SEGMENTS[f.name] = f.read_bytes()


def playlist(prefix):
    return ("#EXTM3U\n#EXT-X-TARGETDURATION:3\n" + "".join(
        f"#EXTINF:2.0,\n{prefix}{n}\n" for n in sorted(SEGMENTS)) + "#EXT-X-ENDLIST\n").encode()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    kind = "plain"   # set per server: cookie | segments | digest

    def log_message(self, *a):
        pass

    def reply(self, code, body=b"", ctype="application/octet-stream", extra=()):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        with LOCK:
            LOG.append({"server": self.server.name, "path": self.path,
                        "cookie": self.headers.get("Cookie"), "auth": self.headers.get("Authorization")})
        name = self.path.split("?")[0].rsplit("/", 1)[-1]
        if self.server.name == "A":
            if self.headers.get("Cookie") != COOKIE or self.headers.get("Authorization") != BEARER:
                return self.reply(403)
            if name == "master.m3u8":
                return self.reply(200, playlist(f"http://localhost:{self.server.other}/"),
                                  "application/vnd.apple.mpegurl")
        elif self.server.name == "B":
            if name in SEGMENTS:
                return self.reply(200, SEGMENTS[name], "video/mp2t")
            return self.reply(404)
        elif self.server.name == "D":
            return self.digest(name)
        elif self.server.name == "S":   # Basic, segments via relative paths
            want = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()
            if self.headers.get("Authorization") != want:
                return self.reply(401, extra=[("WWW-Authenticate", 'Basic realm="r"')])
            if name == "vod.m3u8":
                return self.reply(200, playlist(""), "application/vnd.apple.mpegurl")
            if name in SEGMENTS:
                return self.reply(200, SEGMENTS[name], "video/mp2t")
        self.reply(404)

    def digest(self, name):
        realm, nonce = "rel", "abcdef0123456789"
        header = self.headers.get("Authorization") or ""
        if header.startswith("Digest "):
            f = dict(re.findall(r'(\w+)="?([^",]*)"?', header[7:]))
            ha1 = hashlib.md5(f"{USER}:{realm}:{PASSWORD}".encode()).hexdigest()
            ha2 = hashlib.md5(f"GET:{f.get('uri')}".encode()).hexdigest()
            want = hashlib.md5(f"{ha1}:{nonce}:{f.get('nc')}:{f.get('cnonce')}:auth:{ha2}".encode()).hexdigest()
            if f.get("response") == want and f.get("uri") == self.path and f.get("username") == USER:
                if name == "vod.m3u8":
                    return self.reply(200, playlist(""), "application/vnd.apple.mpegurl")
                if name in SEGMENTS:
                    return self.reply(200, SEGMENTS[name], "video/mp2t")
                return self.reply(404)
        self.reply(401, extra=[("WWW-Authenticate",
                                f'Digest realm="{realm}", nonce="{nonce}", qop="auth", algorithm=MD5')])


def serve(name, other=None):
    s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    s.daemon_threads = True
    s.name, s.other = name, other
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s


def sample_argv(samples, stop):
    while not stop.is_set():
        for pid in os.listdir("/proc"):
            if pid.isdigit():
                try:
                    raw = Path(f"/proc/{pid}/cmdline").read_bytes()
                except OSError:
                    continue
                argv = raw.replace(b"\0", b" ").decode(errors="replace")
                if re.match(r"(\S*/)?(ffmpeg|ffprobe) ", argv):
                    samples.append(argv)
        time.sleep(0.01)


def status_of(url):
    try:
        return urllib.request.urlopen(url, timeout=5).status
    except urllib.error.HTTPError as e:
        return e.code


def main():
    for tool in ("ffmpeg", "ffprobe"):
        assert subprocess.run(["which", tool], capture_output=True).returncode == 0, tool
    work = Path(tempfile.mkdtemp(prefix="xdm-relay-"))
    make_segments(work)
    b = serve("B")
    a = serve("A", other=b.server_port)
    samples, stop = [], threading.Event()
    sampler = threading.Thread(target=sample_argv, args=(samples, stop), daemon=True)
    sampler.start()
    probes = {}

    def watch_relay():
        # Hit the live relay with a missing and a wrong token while the job runs.
        deadline = time.time() + 60
        while time.time() < deadline and "port" not in probes:
            for argv in list(samples):
                m = re.search(r"http://127\.0\.0\.1:(\d+)/([0-9a-f]+)/", argv)
                if m:
                    port = int(m.group(1))
                    probes["port"], probes["token"] = port, m.group(2)
                    probes["no_token"] = status_of(f"http://127.0.0.1:{port}/0/master.m3u8")
                    probes["root"] = status_of(f"http://127.0.0.1:{port}/")
                    probes["wrong"] = status_of(f"http://127.0.0.1:{port}/{'0' * len(m.group(2))}/0/master.m3u8")
                    return
            time.sleep(0.005)
    watcher = threading.Thread(target=watch_relay, daemon=True)
    watcher.start()
    try:
        out = work / "out.mp4"
        url = f"http://127.0.0.1:{a.server_port}/master.m3u8"
        result = subprocess.run(
            [str(CLI), "media", "download", url, str(out), "0", "--header", f"Cookie: {COOKIE}",
             "--header", f"Authorization: {BEARER}", "--header", "Referer: http://127.0.0.1/page",
             "--header", "User-Agent: xdm-relay-test"],
            capture_output=True, text=True, timeout=120)
        assert result.returncode == 0, result.stderr + result.stdout
        assert out.is_file() and out.stat().st_size > 1000, "download produced no output"
        watcher.join(5)
        # (1) argv never carries secrets
        assert samples, "never sampled an ffmpeg/ffprobe process"
        for argv in samples:
            for secret in (COOKIE, "cookie-secret", "token-secret", "Cookie", "Authorization", "-headers"):
                assert secret not in argv, f"{secret!r} in argv: {argv}"
        # (2) origin scoping
        with LOCK:
            seen = list(LOG)
        on_a = [e for e in seen if e["server"] == "A"]
        on_b = [e for e in seen if e["server"] == "B"]
        assert on_a and all(e["cookie"] == COOKIE and e["auth"] == BEARER for e in on_a), on_a
        assert any(e["path"].endswith(".ts") for e in on_b), "segments never fetched from origin B"
        assert all(e["cookie"] is None and e["auth"] is None for e in on_b), on_b
        # (3) token required
        assert probes.get("no_token") == 404 and probes.get("root") == 404 and probes.get("wrong") == 404, probes
        # (4) closed after the job
        s = socket.socket()
        s.settimeout(3)
        assert s.connect_ex(("127.0.0.1", probes["port"])) != 0, "relay still listening after the job"
        s.close()
    finally:
        stop.set()

    # (5) saved logins: Basic and Digest, via daemon jobs
    d_srv, s_srv = serve("D"), serve("S")
    daemon = Daemon("xdm-relay-", env_extra={"PATH": os.environ["PATH"]})
    cases = []
    try:
        for label, srv in (("basic", s_srv), ("digest", d_srv)):
            base_url = f"http://127.0.0.1:{srv.server_port}/"
            daemon.cli("login", "save", base_url, USER, daemon.secret(f"pw-{label}.txt", PASSWORD))
            target = daemon.base / f"{label}.mp4"
            task = daemon.cli("media-job", base_url + "vod.m3u8", target, "video", "0")["task_id"]
            row = daemon.until(task, lambda t: t["status"] in ("failed", "auth_required", "complete"), timeout=90)
            assert row["status"] == "complete", (label, row)
            assert target.is_file() and target.stat().st_size > 1000
            cases.append(label)
    finally:
        daemon.close()
    for argv in samples:
        assert PASSWORD not in argv and "Authorization" not in argv, argv
    print(json.dumps({"result": "passed", "cases": [
        "argv free of cookie/token", "cross-origin segments get no credentials",
        "relay 404 without token", "relay closed after job", *[f"{c} login through relay" for c in cases]],
        "argv_samples": len(samples)}))


main()
