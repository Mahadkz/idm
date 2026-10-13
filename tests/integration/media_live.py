"""End-to-end live HLS/DASH checks: inspect live flags, stop-and-save
recording, self-completion on ENDLIST, and max-duration caps on a locally
generated sliding-window feed."""

import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
import time
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOL = ROOT / "target/debug/examples/media-live"
SEGMENT_POOL = 40
WINDOW = 6
SEGMENT_SECS = 1.0


def run(*args, timeout=120):
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise AssertionError(f"{args}: {result.stderr}")
    return result.stdout


def fail(*args, marker, timeout=120):
    result = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if result.returncode == 0:
        raise AssertionError(f"{args} unexpectedly succeeded")
    if marker not in result.stderr:
        raise AssertionError(f"{args}: expected '{marker}' in {result.stderr}")
    return result.stderr


def probe_streams(output):
    return json.loads(
        run("ffprobe", "-v", "error", "-show_streams", "-of", "json", str(output))
    )["streams"]


def duration(output):
    report = json.loads(
        run("ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "json", str(output))
    )
    return float(report["format"]["duration"])


def decodes(output):
    run("ffmpeg", "-nostdin", "-v", "error", "-i", str(output), "-f", "null", "-")


def generate_segments(folder):
    """One continuous encode, split into 1 s slices with advancing timestamps."""
    run("ffmpeg", "-hide_banner", "-loglevel", "error",
        "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=10",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
        "-t", str(SEGMENT_POOL),
        "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-g", "10",
        "-c:a", "aac", "-b:a", "32k",
        "-f", "segment", "-segment_time", str(SEGMENT_SECS), "-reset_timestamps", "0",
        str(folder / "seg%02d.ts"))


def generate_dash(folder):
    """A dynamic DASH manifest over real segments, kept available by the server."""
    dash = folder / "dash"
    dash.mkdir()
    run("ffmpeg", "-hide_banner", "-loglevel", "error",
        "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=10",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
        "-t", "2", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
        "-c:a", "aac",
        "-f", "dash", "-adaptation_sets", "id=0,streams=v id=1,streams=a",
        str(dash / "live.mpd"))
    text = (dash / "live.mpd").read_text()
    text = re.sub(r"\t*mediaPresentationDuration=\"[^\"]*\"\n", "", text)
    text = text.replace("maxSegmentDuration", 'minimumUpdatePeriod="PT1.0S" maxSegmentDuration')
    lowered = text.lower()
    if 'type="static"' in lowered:
        text = text.replace('type="static"', 'type="dynamic"').replace(
            "type='static'", "type='dynamic'")
    return text


class LivePlaylist:
    """A media playlist whose window grows one segment per second; the clock
    starts at its first fetch so every recording sees a growing feed."""

    def __init__(self, endlist_after=None):
        self.started = None
        self.endlist_after = endlist_after

    def render(self):
        if self.started is None:
            self.started = time.monotonic()
        finished = int(time.monotonic() - self.started)
        # The pool files are complete on disk, so the first segment is served
        # right away and the window grows once per second.
        count = max(finished, 1)
        if self.endlist_after is not None:
            count = min(count, self.endlist_after)
        count = min(count, SEGMENT_POOL)
        first = max(0, count - WINDOW)
        lines = ["#EXTM3U", "#EXT-X-VERSION:3", "#EXT-X-TARGETDURATION:2",
                 f"#EXT-X-MEDIA-SEQUENCE:{first}"]
        for number in range(first, count):
            lines.append("#EXTINF:1.0,")
            lines.append(f"seg{number:02d}.ts")
        if self.endlist_after is not None and finished >= self.endlist_after:
            lines.append("#EXT-X-ENDLIST")
        return ("\n".join(lines) + "\n").encode()


def dynamic_mpd(template):
    """Serves the MPD with a rolling availabilityStartTime so segment one is
    always the currently available fragment."""
    now = datetime.now(timezone.utc)
    stamp = (now - timedelta(seconds=1.6)).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
    return template.replace(
        "<MPD ", f'<MPD availabilityStartTime="{stamp}" publishTime="{stamp}" ', 1
    ).encode()


def main():
    if not TOOL.exists():
        raise SystemExit("build first: cargo build -p xdm-media --examples")
    if not shutil.which("ffmpeg") and (Path.home() / ".local/bin/ffmpeg").is_file():
        os.environ["PATH"] = f"{Path.home() / '.local/bin'}:{os.environ['PATH']}"
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")
    with tempfile.TemporaryDirectory(prefix="xdm-live-test-") as directory:
        folder = Path(directory)
        generate_segments(folder)
        mpd_template = generate_dash(folder)
        live = LivePlaylist()
        ended = LivePlaylist(endlist_after=3)

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                try:
                    path = self.path.split("?")[0]
                    if path == "/live.m3u8":
                        body, ctype = live.render(), "application/vnd.apple.mpegurl"
                    elif path == "/end.m3u8":
                        body, ctype = ended.render(), "application/vnd.apple.mpegurl"
                    elif path == "/vod.m3u8":
                        body = ("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:1.0,\nseg00.ts\n"
                                "#EXTINF:1.0,\nseg01.ts\n#EXT-X-ENDLIST\n").encode()
                        ctype = "application/vnd.apple.mpegurl"
                    elif path == "/dash/dyn.mpd":
                        body, ctype = dynamic_mpd(mpd_template), "application/dash+xml"
                    else:
                        file = folder / path.lstrip("/")
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
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"

            # inspect: rolling playlists are live, ended and static ones are not.
            report = json.loads(run(str(TOOL), "inspect", f"{base}/live.m3u8"))
            assert report["live"] is True, report
            tracks = report["tracks"]
            video = next(t["index"] for t in tracks if t["kind"] == "video")
            audio = next(t["index"] for t in tracks if t["kind"] == "audio")
            vod = json.loads(run(str(TOOL), "inspect", f"{base}/vod.m3u8"))
            assert vod["live"] is False, vod
            dash = json.loads(run(str(TOOL), "inspect", f"{base}/dash/dyn.mpd"))
            assert dash["live"] is True, dash
            assert any(t["kind"] == "video" for t in dash["tracks"]), dash
            assert any(t["kind"] == "audio" for t in dash["tracks"]), dash

            # Stop and save: an external stop ~4 s in must keep the recording.
            take = folder / "take.mkv"
            process = subprocess.Popen(
                [str(TOOL), "record", f"{base}/live.m3u8", str(take), str(video), str(audio)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(4.0)
            process.stdin.close()
            summary = json.loads(process.stdout.read())
            process.wait(timeout=60)
            assert process.returncode == 0, process.stderr.read()
            assert summary["duration_secs"] >= 2.0, summary
            assert summary["bytes"] > 0 and summary["bytes"] == take.stat().st_size, summary
            streams = probe_streams(take)
            assert any(s["codec_type"] == "video" for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            assert duration(take) >= 2.0
            decodes(take)
            assert not (folder / "take.mkv.xdm-part").exists(), "temp file survived"

            # An existing destination is never overwritten.
            busy = folder / "busy.mkv"
            busy.write_bytes(b"keep")
            fail(str(TOOL), "record", f"{base}/live.m3u8", str(busy), str(video), str(audio),
                 marker="already exists")
            assert busy.read_bytes() == b"keep"

            # The feed ending by itself finishes the recording normally.
            ending = folder / "ending.mkv"
            process = subprocess.Popen(
                [str(TOOL), "record", f"{base}/end.m3u8", str(ending), str(video), str(audio)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            summary = json.loads(process.stdout.read())
            process.wait(timeout=60)
            assert process.returncode == 0, process.stderr.read()
            assert 1.5 <= summary["duration_secs"] <= 6.0, summary
            decodes(ending)
            assert not (folder / "ending.mkv.xdm-part").exists()

            # max-duration caps a recording without any stop signal; the
            # fragmented-MP4 path stays playable.
            capped = folder / "capped.mp4"
            summary = json.loads(run(str(TOOL), "record", f"{base}/live.m3u8", str(capped),
                                     str(video), str(audio), "--max-secs", "3"))
            assert summary["duration_secs"] >= 2.0, summary
            streams = probe_streams(capped)
            assert any(s["codec_type"] == "video" for s in streams), streams
            assert any(s["codec_type"] == "audio" for s in streams), streams
            decodes(capped)
            assert not (folder / "capped.mp4.xdm-part").exists()
        finally:
            server.shutdown()
            server.server_close()
    print("live inspect flags, stop-and-save, ENDLIST self-completion, max-duration, "
          "non-overwrite, and temp-file cleanup checked")


if __name__ == "__main__":
    main()
