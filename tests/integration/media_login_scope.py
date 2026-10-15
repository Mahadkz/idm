"""Review item 1: a saved login reaches only its own origin in a media manifest walk and never ffmpeg argv."""
import base64
import http.server
import json
import os
import stat
import sys
import tempfile
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon

USER, PASSWORD = "mediauser", "media-secret-77"
AUTH = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()
LOG, LOCK = [], threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        host = (self.headers.get("Host") or "").split(":")[0]
        auth = self.headers.get("Authorization")
        with LOCK:
            LOG.append({"host": host, "path": self.path, "auth": auth})
        # Host A (127.0.0.1) is protected; host B (localhost) is not.
        if host == "127.0.0.1" and auth != AUTH:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="media"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        port = self.server.server_port
        if self.path == "/master.m3u8":
            body = (f'#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="b",URI="http://localhost:{port}/other.m3u8"\n'
                    f'#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO="a"\nvariant.m3u8\n').encode()
        else:
            body = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nhttp://localhost:%d/seg.ts\n#EXT-X-ENDLIST\n" % port
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_port
    tools = Path(tempfile.mkdtemp(prefix="xdm-fakeff-"))
    argv_log = tools / "argv.log"
    for name in ("ffmpeg", "ffprobe"):
        script = tools / name
        script.write_text(f'#!/bin/sh\necho "{name} $@" >> "{argv_log}"\ncat /proc/$$/cmdline | tr "\\0" " " >> "{argv_log}"\nexit 1\n')
        script.chmod(script.stat().st_mode | stat.S_IEXEC)
    daemon = Daemon("xdm-medialogin-", env_extra={"PATH": f"{tools}:{os.environ['PATH']}"})
    try:
        daemon.cli("login", "save", f"http://127.0.0.1:{port}/", USER, daemon.secret("pw.txt", PASSWORD))
        task = daemon.cli("media-job", f"http://127.0.0.1:{port}/master.m3u8", daemon.base / "out.mp3", "audio", "0", "mp3:96")["task_id"]
        daemon.until(task, lambda t: t["status"] in ("failed", "auth_required", "complete"))
        with LOCK:
            seen = list(LOG)
        home = [e for e in seen if e["host"] == "127.0.0.1"]
        other = [e for e in seen if e["host"] == "localhost"]
        assert any(e["auth"] == AUTH for e in home), seen          # own origin: authenticated manifest walk
        assert any(e["path"] == "/variant.m3u8" and e["auth"] == AUTH for e in home), seen
        assert other, "the cross-origin rendition playlist was never fetched"
        assert all(e["auth"] is None for e in other), other         # host B never sees credentials
        argv = argv_log.read_text() if argv_log.exists() else ""
        assert argv, "ffprobe stub was not reached"
        for secret in (PASSWORD, AUTH, base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode(), "Authorization"):
            assert secret not in argv, "credentials leaked into ffmpeg/ffprobe argv"
        print(json.dumps({"result": "passed", "cases": [
            "own-origin manifests authenticated", "cross-origin playlist unauthenticated",
            "no credentials in ffprobe/ffmpeg argv"]}))
    finally:
        server.shutdown()
        daemon.close()


main()
