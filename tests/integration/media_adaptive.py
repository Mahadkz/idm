"""Daemon adaptive media job: a video-only and an audio-only plain HTTP stream (as served for
adaptive web video) are fetched with the segmented engine, with the submitted headers, and
stream-copied into one container. Local fixture servers only; no external network."""

import json
import os
import re
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

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/xdm-daemon"
SECRET = "xdm-adaptive-test"


def ffmpeg_tools():
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary) and (Path.home() / ".local/bin" / binary).is_file():
            os.environ["PATH"] = f"{Path.home() / '.local/bin'}:{os.environ['PATH']}"
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")


def make_streams(folder):
    folder.mkdir(parents=True)
    subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=25:duration=12",
                    "-vf", "noise=alls=60:allf=t+u", "-c:v", "libx264", "-crf", "8", "-preset", "ultrafast",
                    "-pix_fmt", "yuv420p", "-an", str(folder / "video.mp4")], check=True)
    subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=4",
                    "-c:a", "aac", "-b:a", "128k", "-vn", str(folder / "audio.m4a")], check=True)


class QuietServer(ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass  # clients abort their probe bodies; not a test failure


class Ranged(BaseHTTPRequestHandler):
    folder = None
    log = None
    strict = 0

    def log_message(self, *_args):
        pass

    def do_GET(self):
        name = self.path.split("?")[0].lstrip("/")
        path = self.folder / name
        if self.headers.get("X-Xdm-Secret") != SECRET or not path.is_file():
            self.send_error(403 if path.is_file() else 404)
            return
        data = path.read_bytes()
        header = self.headers.get("Range")
        self.log.append((name, header))
        if self.strict:
            # Like googlevideo: ranged requests only, and no range above the chunk limit.
            bounded = re.fullmatch(r"bytes=(\d+)-(\d+)", header or "")
            if not bounded or int(bounded.group(2)) - int(bounded.group(1)) + 1 > self.strict:
                self.send_error(403)
                return
        start, end, status = 0, len(data) - 1, 200
        match = re.fullmatch(r"bytes=(\d+)-(\d*)", header or "")
        if match:
            start = int(match.group(1))
            end = min(int(match.group(2)) if match.group(2) else end, len(data) - 1)
            status = 206
        body = data[start:end + 1]
        self.send_response(status)
        self.send_header("Content-Type", "video/mp4" if name.endswith("mp4") else "audio/mp4")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(len(body)))
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        self.wfile.write(body)


def main():
    if not BINARY.exists():
        raise SystemExit("build first: cargo build --workspace")
    ffmpeg_tools()
    with tempfile.TemporaryDirectory(prefix="xdm-media-adaptive-") as temp:
        base = Path(temp)
        make_streams(base / "served")
        Ranged.folder = base / "served"
        Ranged.log = []
        Ranged.strict = 450000
        server = QuietServer(("127.0.0.1", 0), Ranged)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        (base / "home").mkdir()
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home"),
               "XDM_RANGE_CAP_HOSTS": "127.0.0.1", "XDM_RANGE_CAP_BYTES": "400000"}
        daemon = subprocess.Popen([str(BINARY)], env=env, stderr=subprocess.PIPE, text=True)
        try:
            assert "listening locally" in daemon.stderr.readline()
            token = (base / "state/ipc-token").read_text().strip()

            def rpc(command):
                data = json.dumps({"version": 1, "token": token, "command": command}).encode()
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.settimeout(10)
                    stream.connect(str(base / "state/daemon.sock"))
                    stream.sendall(struct.pack("=I", len(data)) + data)
                    with stream.makefile("rb") as reader:
                        size = struct.unpack("=I", reader.read(4))[0]
                        return json.loads(reader.read(size))

            def task(task_id):
                return next(t for t in rpc({"type": "list"})["tasks"] if t["id"] == task_id)

            def wait(task_id, wanted):
                end = time.monotonic() + 40
                while time.monotonic() < end:
                    row = task(task_id)
                    if row["status"] == wanted:
                        return row
                    assert row["status"] != "failed" or wanted == "failed", row
                    time.sleep(0.1)
                raise AssertionError(f"task never became {wanted}: {task(task_id)}")

            origin = f"http://127.0.0.1:{server.server_port}"

            def submit(request_id, output, headers, video="video.mp4", audio="audio.m4a"):
                return rpc({"type": "submit_media_job", "payload": {
                    "request_id": request_id, "url": f"{origin}/{video}?expire=1", "output": str(output),
                    "headers": headers, "priority": 1,
                    "mode": {"kind": "adaptive", "audio_url": f"{origin}/{audio}?expire=1"}}})

            # Invalid submissions are refused: bad container, non-web audio URL.
            assert not submit("bad-ext", base / "x.avi", [])["ok"]
            bad = rpc({"type": "submit_media_job", "payload": {
                "request_id": "bad-audio", "url": f"{origin}/video.mp4", "output": str(base / "y.mp4"),
                "headers": [], "priority": 1,
                "mode": {"kind": "adaptive", "audio_url": "file:///etc/passwd"}}})
            assert not bad["ok"], bad

            # Without the secret header both streams are refused, so the job fails.
            denied = submit("denied", base / "denied.mp4", [])
            assert denied["ok"]
            wait(denied["task_id"], "failed")
            assert not (base / "denied.mp4").exists()

            output = base / "Video" / "merged.mp4"
            reply = submit("adaptive-1", output, [["X-Xdm-Secret", SECRET]])
            assert reply["ok"], reply
            row = wait(reply["task_id"], "complete")
            assert row["kind"] == "media" and row["category"] == "Video", row
            probed = json.loads(subprocess.run(
                ["ffprobe", "-v", "error", "-show_streams", "-of", "json", str(output)],
                check=True, capture_output=True, text=True).stdout)["streams"]
            kinds = sorted(s["codec_type"] for s in probed)
            assert kinds == ["audio", "video"], probed
            assert next(s for s in probed if s["codec_type"] == "video")["codec_name"] == "h264"
            assert next(s for s in probed if s["codec_type"] == "audio")["codec_name"] == "aac"
            subprocess.run(["ffmpeg", "-v", "error", "-i", str(output), "-f", "null", "-"], check=True)
            assert not list(output.parent.glob("*.xdm-*")) and not list(output.parent.glob("*.part*")), \
                list(output.parent.iterdir())
            # The segmented engine used byte ranges on both streams.
            if os.environ.get("XDM_TEST_VERBOSE"):
                print(Ranged.log)
            for name in ("video.mp4", "audio.m4a"):
                parts = [h for n, h in Ranged.log if n == name and h]
                assert len(parts) >= 3, (name, Ranged.log)
                # The strict server refused anything open-ended or over its chunk limit, and the job still finished.
                assert all(re.fullmatch(r"bytes=\d+-\d+", h) for h in parts), parts
            video_size = (base / "served/video.mp4").stat().st_size
            assert video_size > 4 * 400000, video_size
            # Same request ID with the same fields is idempotent.
            again = submit("adaptive-1", output, [["X-Xdm-Secret", SECRET]])
            assert again["ok"] and again["task_id"] == reply["task_id"], again
            print("media_adaptive ok")
        finally:
            daemon.terminate()
            server.shutdown()


if __name__ == "__main__":
    main()
