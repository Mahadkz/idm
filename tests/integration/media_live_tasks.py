"""End-to-end daemon live-recording jobs on the locally generated sliding
window feed: CLI inspect live flags, progress while recording, pause as
stop-and-save, self-completion at max_seconds, the VOD fallback, and Basic
auth from a saved site login."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from media_audio import decodes, duration, probe

from media_live import LivePlaylist

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
USER = "alice"
SECRET = "s3cret-protected"


def basic_token():
    import base64
    return "Basic " + base64.b64encode(f"{USER}:{SECRET}".encode()).decode()


def generate_segments(folder):
    """The media_live fixture (one continuous encode, split into 1 s slices)
    at a realistic bitrate, so ffmpeg's buffered muxer flushes recorded bytes
    to disk while the capture is still running."""
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error",
         "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
         "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
         "-t", "40",
         "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-g", "30",
         "-b:v", "3M", "-maxrate", "3M", "-bufsize", "1M",
         "-c:a", "aac", "-b:a", "64k",
         "-f", "segment", "-segment_time", "1.0", "-reset_timestamps", "0",
         str(folder / "seg%02d.ts")], check=True, timeout=120)


def main():
    if not CLI.exists():
        raise SystemExit("build first: cargo build --workspace")
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary) and (Path.home() / ".local/bin/ffmpeg").is_file():
            os.environ["PATH"] = f"{Path.home() / '.local/bin'}:{os.environ['PATH']}"
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")
    with tempfile.TemporaryDirectory(prefix="xdm-media-live-tasks-") as temp:
        base = Path(temp)
        folder = base / "media"
        folder.mkdir(parents=True)
        generate_segments(folder)
        # One rolling playlist per origin: each instance starts its clock at
        # its first fetch so both captures see a growing feed.
        live = LivePlaylist()
        auth_live = LivePlaylist()
        token = basic_token()

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                try:
                    path = self.path.split("?")[0]
                    if path.startswith("/auth") and self.headers.get("Authorization") != token:
                        self.send_response(401)
                        self.send_header(
                            "WWW-Authenticate", f'Basic realm="{USER}"'
                        )
                        self.end_headers()
                        return
                    if path == "/live.m3u8":
                        body, ctype = live.render(), "application/vnd.apple.mpegurl"
                    elif path == "/auth/live.m3u8":
                        body, ctype = auth_live.render(), "application/vnd.apple.mpegurl"
                    elif path == "/vod.m3u8":
                        body = (
                            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:1.0,\nseg00.ts\n"
                            "#EXTINF:1.0,\nseg01.ts\n#EXT-X-ENDLIST\n"
                        ).encode()
                        ctype = "application/vnd.apple.mpegurl"
                    else:
                        # Segments answer under both / and /auth/ prefixes.
                        file = folder / path.split("/")[-1]
                        if not file.is_file():
                            self.send_error(404)
                            return
                        body, ctype = file.read_bytes(), "video/mp2t"
                    self.send_response(200)
                    self.send_header("Content-Type", ctype)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except BrokenPipeError:
                    pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        (base / "home").mkdir()
        daemon = None
        try:
            stream_url = f"http://127.0.0.1:{server.server_port}/live.m3u8"
            auth_url = f"http://127.0.0.1:{server.server_port}/auth/live.m3u8"
            vod_url = f"http://127.0.0.1:{server.server_port}/vod.m3u8"
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/xdm-daemon")], env=env,
                stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()

            def cli(*arguments):
                result = subprocess.run(
                    [str(CLI), "daemon", *map(str, arguments)],
                    env=env, capture_output=True, text=True, timeout=30)
                assert result.returncode == 0, result.stderr + result.stdout
                return json.loads(result.stdout)

            def track_flags(url):
                result = subprocess.run(
                    [str(CLI), "media", "inspect", url],
                    env=env, capture_output=True, text=True, timeout=60)
                assert result.returncode == 0, result.stderr
                return json.loads(result.stdout), result.stderr

            # CLI inspect keeps the bare track array on stdout; the live
            # verdict rides on stderr.
            tracks, stderr = track_flags(stream_url)
            video = next(t["index"] for t in tracks if t["kind"] == "video")
            audio = next(t["index"] for t in tracks if t["kind"] == "audio")
            assert "live: true" in stderr, stderr
            vod_tracks, stderr = track_flags(vod_url)
            assert "live: false" in stderr, stderr
            assert isinstance(tracks, list) and vod_tracks, (tracks, vod_tracks)

            def row_of(task_id):
                return next(t for t in cli("list")["tasks"] if t["id"] == task_id)

            def wait_status(task_id, wanted, deadline=40.0, extra_ok=()):
                limit = time.monotonic() + deadline
                while time.monotonic() < limit:
                    row = row_of(task_id)
                    if row["status"] == wanted:
                        return row
                    allowed = ("queued", "running", "pausing") + extra_ok
                    assert row["status"] in allowed, row
                    time.sleep(0.2)
                raise AssertionError(f"task {task_id} never became {wanted}")

            def submit_live(url, output, extra=()):
                reply = cli("media-job", url, str(output),
                            "live", str(video), str(audio), *extra)
                assert reply["ok"], reply
                return reply["task_id"]

            # Stop and save through the queue: the running job is paused, the
            # recorder finishes gracefully, and the task COMPLETES with the
            # playable file instead of pausing.
            take = base / "stop-save.mkv"
            take_id = submit_live(stream_url, take)
            wait_status(take_id, "running")
            # Progress reports recorded bytes and capture seconds.
            limit = time.monotonic() + 10
            while time.monotonic() < limit:
                row = row_of(take_id)
                if (row["downloaded"] or 0) > 0 and (row["media_seconds"] or 0) >= 1:
                    break
                time.sleep(0.2)
            else:
                raise AssertionError(f"no live progress reported: {row}")
            assert row["total"] is None, "a live capture has no total"
            time.sleep(4.0)
            assert not take.exists(), "destination appeared before completion"
            cli("pause", take_id)
            wait_status(take_id, "complete")
            streams = probe(take)
            assert any(s["codec_type"] == "video" for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            assert duration(take) >= 2.0, duration(take)
            decodes(take)
            assert not (base / "stop-save.mkv.xdm-part").exists(), "temp file survived"

            # A catch cap ends the recording by itself; the fragmented MP4
            # path stays playable.
            capped = base / "capped.mp4"
            capped_id = submit_live(stream_url, capped, extra=("--max-seconds", "3"))
            wait_status(capped_id, "complete")
            streams = probe(capped)
            assert any(s["codec_type"] == "video" for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            assert duration(capped) >= 2.0, duration(capped)
            decodes(capped)
            assert not (base / "capped.mp4.xdm-part").exists(), "temp file survived"

            # A VOD manifest submitted as Live falls back to the normal media
            # download behavior: it records until the stream ends.
            fallback = base / "vod-fallback.mkv"
            fallback_id = submit_live(vod_url, fallback)
            wait_status(fallback_id, "complete")
            decodes(fallback)
            assert duration(fallback) >= 1.0, duration(fallback)

            # A Basic-auth protected manifest: without a saved login the job
            # fails; after `login save` the daemon replays the credentials of
            # the manifest origin for ffmpeg (no explicit header was passed).
            denied = base / "auth-denied.mp4"
            denied_id = submit_live(auth_url, denied, extra=("--max-seconds", "3"))
            wait_status(denied_id, "failed", extra_ok=("failed",))
            assert not denied.exists()
            login = subprocess.run(
                [str(CLI), "daemon", "login", "save",
                 f"http://127.0.0.1:{server.server_port}/", USER, "-"],
                env=env, input=f"{SECRET}\n", capture_output=True, text=True, timeout=10)
            assert login.returncode == 0, login.stderr
            # Saved logins never echo the password back through the CLI.
            assert SECRET not in login.stdout and SECRET not in login.stderr, login
            authed = base / "auth-live.mp4"
            auth_id = submit_live(auth_url, authed, extra=("--max-seconds", "3"))
            wait_status(auth_id, "complete")
            streams = probe(authed)
            assert any(s["codec_type"] == "video" for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            assert duration(authed) >= 2.0, duration(authed)
            decodes(authed)
            assert not (base / "auth-live.mp4.xdm-part").exists()
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            server.shutdown()
            server.server_close()
    print("daemon live jobs: inspect flags, bytes+seconds progress, pause as stop-and-save, "
          "max-seconds self-completion, VOD fallback, and Basic-auth login replay passed")


if __name__ == "__main__":
    main()

