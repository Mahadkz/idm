"""Optional yt-dlp integration through a fake yt-dlp (a Python stub that emits fixture JSON): MediaProbe
parsing and caching, the missing-program error, argv safety, the cookie file (0600, private directory,
deleted afterwards, never on argv or in logs), two-stream downloads through the segmented engine with the
per-format headers and an ffmpeg merge, a 403 that re-probes once, and the yt-dlp downloader fallback
with pause. Local fixture servers only; no external network and no real yt-dlp."""

import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from media_adaptive import QuietServer, ffmpeg_tools, make_streams  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/xdm-daemon"
SECRET = "xdm-ytdlp-test"
COOKIE_VALUE = "cookie-secret-zxcv-9876"

STUB = r'''#!__PYTHON__
import json, os, sys, time, shutil, stat, hashlib
state = os.environ["FAKE_YTDLP_STATE"]
args = sys.argv[1:]
log = {"argv": args, "pid": os.getpid()}
if "--cookies" in args:
    jar = args[args.index("--cookies") + 1]
    log["cookie_mode"] = stat.S_IMODE(os.stat(jar).st_mode)
    log["cookie_dir_mode"] = stat.S_IMODE(os.stat(os.path.dirname(jar)).st_mode)
    log["cookie_jar"] = jar
    log["cookie_text"] = open(jar).read()
with open(os.path.join(state, "calls.jsonl"), "a") as handle:
    handle.write(json.dumps(log) + "\n")
url = args[-1]
origin = os.environ["FAKE_ORIGIN"]
if "-J" in args:
    counter = os.path.join(state, "probes-" + hashlib.sha1(url.encode()).hexdigest()[:10])
    n = (int(open(counter).read()) if os.path.exists(counter) else 0) + 1
    open(counter, "w").write(str(n))
    token = "old" if ("expiring" in url and n == 1) else "new"
    headers = {"User-Agent": "fake-agent", "X-Xdm-Secret": os.environ["FAKE_SECRET"]}
    def fmt(id, ext, kind, **extra):
        base = {"format_id": id, "ext": ext, "protocol": "https", "url": f"{origin}/{id}.{ext}?token={token}", "http_headers": headers,
                "vcodec": "none", "acodec": "none"}
        base.update(extra)
        return base
    formats = [
        {"format_id": "sb0", "ext": "mhtml", "protocol": "mhtml", "url": "https://x/sb", "vcodec": "none", "acodec": "none", "format_note": "storyboard"},
        fmt("video", "mp4", "v", vcodec="avc1.4d401f", height=180, width=320, fps=25, tbr=900, filesize=None, filesize_approx=1234567.0, format_note="180p"),
        fmt("audio", "m4a", "a", acodec="mp4a.40.2", abr=128, filesize=22222),
        fmt("muxed", "mp4", "m", vcodec="avc1.4d401f", acodec="mp4a.40.2", height=180, width=320, fps=25, tbr=1000),
        fmt("hlsvideo", "mp4", "v", vcodec="avc1.4d401f", height=180, protocol="m3u8_native", tbr=800),
        fmt("slow", "mp4", "v", vcodec="avc1.4d401f", height=90, protocol="m3u8_native", tbr=100),
    ]
    json.dump({"id": "x", "title": "Fixture: Title", "duration": 12, "thumbnail": "https://img/t.jpg", "extractor_key": "Fake", "formats": formats}, sys.stdout)
    sys.exit(0)
# downloader mode
out = args[args.index("-o") + 1]
sel = args[args.index("-f") + 1]
fmt_id = sel.split("+")[0]
total = 1000000
for done in (100000, 400000):
    print(f"XDMP {done} NA {total}", flush=True)
if fmt_id == "slow":
    time.sleep(60)
shutil.copy(os.path.join(os.environ["FAKE_SERVED"], "video.mp4"), out)
print(f"XDMP {total} {total} NA", flush=True)
'''


class Served(BaseHTTPRequestHandler):
    folder = None
    log = None

    def log_message(self, *_args):
        pass

    def do_GET(self):
        path = self.path.split("?")[0].lstrip("/")
        token = self.path.split("token=")[-1] if "token=" in self.path else ""
        name = {"video.mp4": "video.mp4", "audio.m4a": "audio.m4a", "muxed.mp4": "video.mp4"}.get(path)
        self.log.append((path, token, self.headers.get("X-Xdm-Secret"), self.headers.get("User-Agent")))
        if name is None or self.headers.get("X-Xdm-Secret") != SECRET or token != "new":
            self.send_error(403 if name else 404)
            return
        data = (self.folder / name).read_bytes()
        start, end, status = 0, len(data) - 1, 200
        header = self.headers.get("Range")
        if header and header.startswith("bytes="):
            lo, _, hi = header[6:].partition("-")
            start, end, status = int(lo), min(int(hi) if hi else end, len(data) - 1), 206
        body = data[start:end + 1]
        self.send_response(status)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(len(body)))
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        self.wfile.write(body)


class Daemon:
    def __init__(self, base, name, env_extra):
        self.base = base / name
        (self.base / "home").mkdir(parents=True)
        env = {**os.environ, "XDM_DATA_DIR": str(self.base / "state"), "HOME": str(self.base / "home"), **env_extra}
        self.proc = subprocess.Popen([str(BINARY)], env=env, stderr=subprocess.PIPE, text=True)
        assert "listening locally" in self.proc.stderr.readline()
        self.token = (self.base / "state/ipc-token").read_text().strip()

    def rpc(self, command):
        data = json.dumps({"version": 1, "token": self.token, "command": command}).encode()
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(90)
            stream.connect(str(self.base / "state/daemon.sock"))
            stream.sendall(struct.pack("=I", len(data)) + data)
            with stream.makefile("rb") as reader:
                size = struct.unpack("=I", reader.read(4))[0]
                return json.loads(reader.read(size))

    def task(self, task_id):
        return next(t for t in self.rpc({"type": "list"})["tasks"] if t["id"] == task_id)

    def wait(self, task_id, wanted, seconds=60):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            row = self.task(task_id)
            if row["status"] == wanted:
                return row
            assert row["status"] != "failed" or wanted == "failed", row
            time.sleep(0.1)
        raise AssertionError(f"task never became {wanted}: {self.task(task_id)}")

    def stop(self):
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def probe(daemon, url, cookies=None, refresh=False):
    return daemon.rpc({"type": "media_probe", "payload": {"page_url": url, "cookies": cookies or [], "refresh": refresh}})


def calls(state):
    path = state / "calls.jsonl"
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def streams(path):
    return sorted(s["codec_type"] for s in json.loads(subprocess.run(
        ["ffprobe", "-v", "error", "-show_streams", "-of", "json", str(path)], check=True, capture_output=True, text=True).stdout)["streams"])


def main():
    if not BINARY.exists():
        raise SystemExit("build first: cargo build --workspace")
    ffmpeg_tools()
    with tempfile.TemporaryDirectory(prefix="xdm-ytdlp-") as temp:
        base = Path(temp)
        make_streams(base / "served")
        Served.folder, Served.log = base / "served", []
        server = QuietServer(("127.0.0.1", 0), Served)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        origin = f"http://127.0.0.1:{server.server_port}"
        state = base / "fake-state"
        state.mkdir()
        stub = base / "tools" / "fake-yt-dlp"
        stub.parent.mkdir()
        stub.write_text(STUB.replace("__PYTHON__", sys.executable))
        stub.chmod(0o755)
        fake_env = {"FAKE_YTDLP_STATE": str(state), "FAKE_ORIGIN": origin, "FAKE_SECRET": SECRET, "FAKE_SERVED": str(base / "served")}

        # 1. No yt-dlp anywhere: a clear error with an install hint, flagged for the UI.
        empty = base / "empty-path"
        empty.mkdir()
        missing = Daemon(base, "missing", {"PATH": str(empty), "XDM_RANGE_CAP_HOSTS": "127.0.0.1"})
        try:
            reply = probe(missing, "https://video.test/watch?v=1")
            assert not reply["ok"] and reply.get("ytdlp_missing") is True, reply
            assert "yt-dlp was not found" in reply["error"] and "pipx install yt-dlp" in reply["error"], reply
            assert missing.rpc({"type": "get_ytdlp_path"})["ytdlp_found"] is False
            bad = missing.rpc({"type": "set_ytdlp_path", "payload": {"path": str(base / "nope")}})
            assert not bad["ok"], bad
        finally:
            missing.stop()

        daemon = Daemon(base, "main", {**fake_env, "XDM_RANGE_CAP_HOSTS": "127.0.0.1", "XDM_RANGE_CAP_BYTES": "400000"})
        try:
            # 2. Configure the stub; the setting reads back.
            reply = daemon.rpc({"type": "set_ytdlp_path", "payload": {"path": str(stub)}})
            assert reply["ok"] and reply["ytdlp_found"] and reply["ytdlp_path"] == str(stub), reply
            assert daemon.rpc({"type": "get_ytdlp_path"})["ytdlp_path"] == str(stub)

            # 3. argv safety: nothing that could be read as an option reaches yt-dlp.
            for bad_url in ["--exec=touch /tmp/pwned", "-o /tmp/x", "file:///etc/passwd", "https://a.test/x y", "javascript:1", ""]:
                reply = probe(daemon, bad_url)
                assert not reply["ok"], (bad_url, reply)
            assert calls(state) == [], "rejected URLs never start yt-dlp"
            refused = daemon.rpc({"type": "submit_media_job", "payload": {
                "request_id": "evil-selector", "url": "https://video.test/watch?v=2", "output": str(base / "evil.mp4"), "headers": [],
                "priority": 1, "mode": {"kind": "yt_dlp", "format": "-o /tmp/x"}}})
            assert not refused["ok"], refused
            refused = daemon.rpc({"type": "submit_media_job", "payload": {
                "request_id": "evil-selector2", "url": "https://video.test/watch?v=2", "output": str(base / "evil.mp4"), "headers": [],
                "priority": 1, "mode": {"kind": "yt_dlp", "format": "best[height<=1080]"}}})
            assert not refused["ok"], refused

            # 4. Probe parsing and the exact argument vector.
            page = "https://video.test/watch?v=parsing"
            reply = probe(daemon, page)
            assert reply["ok"], reply
            result = reply["probe"]
            assert result["title"] == "Fixture: Title" and result["duration"] == 12 and result["extractor"] == "Fake", result
            by_id = {f["id"]: f for f in result["formats"]}
            assert set(by_id) == {"video", "audio", "muxed", "hlsvideo", "slow"}, "storyboards are dropped"
            assert [by_id[k]["kind"] for k in ("video", "audio", "muxed")] == ["video", "audio", "muxed"]
            assert by_id["video"]["filesize"] == 1234567 and by_id["video"]["filesize_approx"] is True
            assert by_id["audio"]["filesize"] == 22222 and by_id["audio"]["filesize_approx"] is False
            assert by_id["video"]["height"] == 180 and by_id["video"]["fps"] == 25 and by_id["video"]["vcodec"].startswith("avc1")
            assert by_id["audio"]["acodec"].startswith("mp4a") and by_id["audio"]["abr"] == 128
            assert by_id["video"]["headers"] and ["X-Xdm-Secret", SECRET] in by_id["video"]["headers"]
            assert by_id["hlsvideo"]["protocol"] == "m3u8_native"
            argv = calls(state)[0]["argv"]
            assert argv[:1] == ["-J"] and "--no-playlist" in argv and "--no-warnings" in argv, argv
            assert argv[-2:] == ["--", page], "the URL is last, after --"
            assert "--cookies" not in argv
            # Cached briefly per URL: a second probe does not run yt-dlp, a refresh does.
            before = len(calls(state))
            assert probe(daemon, page)["ok"] and len(calls(state)) == before
            assert probe(daemon, page, refresh=True)["ok"] and len(calls(state)) == before + 1

            # 5. Cookies: private 0600 file in a private directory, gone afterwards, never on argv or in logs.
            cookie = {"name": "SID", "value": COOKIE_VALUE, "domain": ".video.test", "path": "/", "secure": True, "expires": 1900000000}
            reply = probe(daemon, "https://video.test/watch?v=cookies", cookies=[cookie])
            assert reply["ok"], reply
            seen = calls(state)[-1]
            assert seen["cookie_mode"] == 0o600 and seen["cookie_dir_mode"] == 0o700, seen
            assert COOKIE_VALUE in seen["cookie_text"] and seen["cookie_text"].startswith("# Netscape HTTP Cookie File")
            assert not Path(seen["cookie_jar"]).exists() and not Path(seen["cookie_jar"]).parent.exists(), "cookie jar deleted"
            assert COOKIE_VALUE not in json.dumps(seen["argv"]), "cookies never on argv"
            tampered = {**cookie, "name": "a\tb", "value": "x\ny"}
            assert probe(daemon, "https://video.test/watch?v=cookies2", cookies=[tampered])["ok"]
            assert "x\ny" not in calls(state)[-1]["cookie_text"], "tab/newline cookies are skipped"
            for log in (daemon.base / "state").rglob("*"):
                if log.is_file() and log.suffix in {".log", ".txt", ".json"}:
                    assert COOKIE_VALUE not in log.read_text(errors="ignore"), log

            def submit(request_id, page_url, selector, output):
                return daemon.rpc({"type": "submit_media_job", "payload": {
                    "request_id": request_id, "url": page_url, "output": str(output), "headers": [], "priority": 1,
                    "mode": {"kind": "yt_dlp", "format": selector}}})

            # 6. Two plain https formats: the engine downloads both with the per-format headers, ffmpeg merges.
            merged = base / "Video" / "merged.mp4"
            start = len(Served.log)
            reply = submit("yt-merge", "https://video.test/watch?v=merge", "video+audio", merged)
            assert reply["ok"], reply
            row = daemon.wait(reply["task_id"], "complete")
            assert row["kind"] == "media" and row["category"] == "Video", row
            assert streams(merged) == ["audio", "video"]
            hits = Served.log[start:]
            assert {h[0] for h in hits} == {"video.mp4", "audio.m4a"}, hits
            assert all(h[2] == SECRET and h[3] == "fake-agent" for h in hits), "per-format http_headers were sent"
            assert not any("-J" not in c["argv"] for c in calls(state)), "no yt-dlp download was needed"
            assert not list(merged.parent.glob("*.xdm-*"))

            # 7. A single muxed https format goes through the engine as one file.
            single = base / "Video" / "single.mp4"
            reply = submit("yt-single", "https://video.test/watch?v=single", "muxed", single)
            daemon.wait(reply["task_id"], "complete")
            assert single.read_bytes() == (base / "served/video.mp4").read_bytes()

            # 8. An expired address (403) re-probes once and succeeds with the fresh one.
            expiring = base / "Video" / "expiring.mp4"
            start = len(Served.log)
            reply = submit("yt-expiring", "https://video.test/watch?v=expiring", "video+audio", expiring)
            daemon.wait(reply["task_id"], "complete")
            probes_here = [c for c in calls(state) if c["argv"][-1].endswith("v=expiring")]
            assert len(probes_here) == 2, "exactly one re-probe"
            tokens = [h[1] for h in Served.log[start:]]
            assert "old" in tokens and tokens.count("new") >= 2, tokens
            assert streams(expiring) == ["audio", "video"]

            # 9. Formats the engine cannot fetch (HLS) are downloaded by yt-dlp itself, progress parsed.
            fallback = base / "Video" / "fallback.mp4"
            reply = submit("yt-fallback", "https://video.test/watch?v=fallback", "hlsvideo+audio", fallback)
            daemon.wait(reply["task_id"], "complete")
            dl = [c for c in calls(state) if "-f" in c["argv"] and c["argv"][-1].endswith("v=fallback")][-1]["argv"]
            assert dl[dl.index("-f") + 1] == "hlsvideo+audio" and "--newline" in dl and "--no-playlist" in dl, dl
            assert dl[-2:] == ["--", "https://video.test/watch?v=fallback"]
            assert dl[dl.index("-o") + 1].endswith("fallback.xdm-ytdlp.mp4")
            assert fallback.read_bytes() == (base / "served/video.mp4").read_bytes()
            assert not list(fallback.parent.glob("*.xdm-ytdlp*")), "temporary name is gone after publishing"

            # 10. Pause kills yt-dlp; resume runs it again (yt-dlp continues its own partial files).
            slow = base / "Video" / "slow.mp4"
            reply = submit("yt-slow", "https://video.test/watch?v=slow", "slow", slow)
            tid = reply["task_id"]
            end = time.monotonic() + 20
            while time.monotonic() < end and not [c for c in calls(state) if c["argv"][-1].endswith("v=slow") and "-f" in c["argv"]]:
                time.sleep(0.1)
            pid = [c for c in calls(state) if c["argv"][-1].endswith("v=slow") and "-f" in c["argv"]][-1]["pid"]
            time.sleep(1)
            row = daemon.task(tid)
            assert row["status"] in {"downloading", "active", "running"} or row["downloaded"] > 0, row
            assert daemon.rpc({"type": "pause", "payload": {"task_id": tid}})["ok"]
            end = time.monotonic() + 10
            while time.monotonic() < end:
                try:
                    os.kill(pid, 0)
                    # A zombie also answers; check the state file.
                    if Path(f"/proc/{pid}/stat").read_text().split()[2] == "Z":
                        break
                except (ProcessLookupError, FileNotFoundError):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("yt-dlp was not killed by the pause")
            assert daemon.task(tid)["status"] == "paused", daemon.task(tid)
            assert not slow.exists()
            print("media_ytdlp ok")
        finally:
            daemon.stop()
            server.shutdown()


if __name__ == "__main__":
    main()
