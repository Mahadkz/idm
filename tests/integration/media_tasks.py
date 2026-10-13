"""End-to-end daemon media jobs: audio-only, video+subtitle MKV, header-gated
manifests, and persisted job modes that survive a daemon restart."""

import json
import os
import shutil
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from media_audio import COOKIE, decodes, duration, generate, probe, run, subtitle_text

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"


def exchange(writer, reader, payload):
    data = json.dumps(payload).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def main():
    if not CLI.exists():
        raise SystemExit("build first: cargo build --workspace")
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary) and (Path.home() / ".local/bin/ffmpeg").is_file():
            os.environ["PATH"] = f"{Path.home() / '.local/bin'}:{os.environ['PATH']}"
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")
    with tempfile.TemporaryDirectory(prefix="xdm-media-tasks-") as temp:
        base = Path(temp)
        folder = base / "media"
        generate(folder)
        generate(folder / "gated")

        class Guard(SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=folder, **kwargs)

            def log_message(self, *_args):
                pass

            def do_GET(self):
                if self.path.startswith("/gated/") and self.headers.get("Cookie") != COOKIE:
                    self.send_error(403, "missing cookie")
                    return
                super().do_GET()

        server = ThreadingHTTPServer(("127.0.0.1", 0), Guard)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        (base / "home").mkdir()
        daemon = None
        try:
            base_url = f"http://127.0.0.1:{server.server_port}"
            url = f"{base_url}/master.m3u8"
            gated = f"{base_url}/gated/master.m3u8"

            def cli(*arguments):
                result = subprocess.run([str(CLI), "daemon", *map(str, arguments)],
                                        env=env, capture_output=True, text=True, timeout=10)
                assert result.returncode == 0, result.stderr + result.stdout
                try:
                    return json.loads(result.stdout)
                except json.JSONDecodeError:
                    return result.stdout  # `export` prints a plain-text line.

            def start():
                process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                           stderr=subprocess.PIPE, text=True)
                assert "listening locally" in process.stderr.readline()
                return process

            def wait_status(task_id, wanted, deadline=30.0):
                limit = time.monotonic() + deadline
                while time.monotonic() < limit:
                    row = next(t for t in cli("list")["tasks"] if t["id"] == task_id)
                    if row["status"] == wanted:
                        return row
                    assert row["status"] not in ("failed",) or wanted == "failed", row
                    time.sleep(0.1)
                raise AssertionError(f"task {task_id} never became {wanted}")

            daemon = start()
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

            tracks = json.loads(run(str(CLI), "media", "inspect", url))
            video = next(t["index"] for t in tracks if t["kind"] == "video")
            audio = next(t["index"] for t in tracks if t["kind"] == "audio")
            subs = [t["index"] for t in tracks if t["kind"] == "subtitle"]
            assert video is not None and audio is not None and subs, tracks

            # CLI media submission helpers share one entry: SubmitMediaJob.
            def submit_media_job(arguments, output):
                reply = cli("media-job", url, str(output), *arguments)
                assert reply["ok"]
                return reply["task_id"]

            # Audio-only MP3 transcode through the queue.
            song = base / "song.mp3"
            song_id = submit_media_job(["audio", str(audio), "mp3:96"], song)
            same = rpc({"type": "submit_media_job", "payload": {
                "request_id": "media-tasks-dup", "url": url, "output": str(base / "dup.mp3"),
                "headers": [], "mode": {"kind": "audio", "track": audio,
                "format": {"kind": "mp3", "bitrate_kbps": 96}}, "priority": 1}})
            assert same["ok"] and same["task_id"] >= 1
            dup = rpc({"type": "submit_media_job", "payload": {
                "request_id": "media-tasks-dup", "url": url, "output": str(base / "dup.mp3"),
                "headers": [], "mode": {"kind": "audio", "track": audio,
                "format": {"kind": "mp3", "bitrate_kbps": 96}}, "priority": 1}})
            assert dup["ok"] and dup["task_id"] == same["task_id"], (dup, same)
            wait_status(song_id, "complete")
            assert probe(song)[0]["codec_name"] == "mp3"
            assert 1.6 <= duration(song) <= 3.0
            decodes(song)

            # Video + subtitle MKV job through the queue.
            movie = base / "movie.mkv"
            movie_id = submit_media_job(
                ["video", str(video), str(audio), "--subtitles", str(subs[0])], movie)
            wait_status(movie_id, "complete")
            streams = probe(movie)
            assert any(s["codec_type"] == "video" and s["height"] == 180 for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            assert any(s["codec_type"] == "subtitle" for s in streams), streams
            assert "Hello from XDM" in subtitle_text(movie)
            decodes(movie)

            # Header-protected manifest: without the cookie the task fails
            # permanently; with it the same job completes.
            private = base / "private.m4a"
            denied = cli("media-job", gated, str(private), "audio", str(audio), "m4a:64",
                         "--header", "X-Unrelated: nothing")
            denied_id = denied["task_id"]
            wait_status(denied_id, "failed")
            allowed = base / "allowed.m4a"
            allowed_id = cli("media-job", gated, str(allowed), "audio", str(audio), "m4a:64",
                             "--header", f"Cookie: {COOKIE}")["task_id"]
            wait_status(allowed_id, "complete")
            assert probe(allowed)[0]["codec_name"] == "aac"
            decodes(allowed)

            # Portable export: headers never appear, but plain video MP4 jobs
            # keep their portable media shape (grown-up job kinds that a list
            # reader cannot restore ride out as plain HTTP entries).
            plain = base / "plain.mp4"
            plain_id = cli("media-job", url, str(plain), "video", str(video),
                           str(audio))["task_id"]
            wait_status(plain_id, "complete")
            # Portable export: no request headers ever appear.
            archive = base / "tasks.json"
            cli("export", archive, "json")
            text = archive.read_text()
            assert '"headers"' not in text and COOKIE not in text, "headers leaked into export"
            exported = json.loads(text)
            media_entries = [e for e in exported["entries"] if e["kind"] == "media"]
            assert media_entries and all(e["video"] is not None for e in media_entries), media_entries

            # Restart preserves the queued job's mode; it then completes.
            queued = base / "queued.opus"
            cli("queue", "stop")
            opus = cli("media-job", gated, str(queued), "audio", str(audio), "opus:48",
                       "--header", f"Cookie: {COOKIE}")["task_id"]
            daemon.terminate()
            daemon.wait(timeout=5)
            with sqlite3.connect(base / "state/tasks.sqlite3") as database:
                row = database.execute(
                    "SELECT media_mode_json FROM tasks WHERE id=?", (opus,)).fetchone()[0]
                assert json.loads(row) == {"kind": "audio", "track": audio,
                                           "format": {"kind": "opus", "bitrate_kbps": 48}}, row
            daemon = start()
            cli("queue", "start")
            assert wait_status(opus, "complete")["kind"] == "media"
            assert probe(queued)[0]["codec_name"] == "opus"
            decodes(queued)
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            server.shutdown()
            server.server_close()
    print("daemon media jobs: audio mp3, video+subtitle mkv, header-gated retry, "
          "portable export without headers, and restart-preserved pending mode passed")


if __name__ == "__main__":
    main()
