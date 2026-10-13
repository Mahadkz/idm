"""End-to-end audio-only and subtitle workflow on a locally generated clear HLS feed."""
import json
import os
import shutil
import subprocess
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOL = ROOT / "target/debug/examples/media-audio"
COOKIE = "xdm=secret123"


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise AssertionError(f"{args}: {result.stderr}")
    return result.stdout


def fail(*args, marker):
    result = subprocess.run(args, capture_output=True, text=True, timeout=90)
    if result.returncode == 0:
        raise AssertionError(f"{args} unexpectedly succeeded")
    if marker not in result.stderr:
        raise AssertionError(f"{args}: expected '{marker}' in {result.stderr}")
    return result.stderr


def probe(output):
    streams = json.loads(run("ffprobe", "-v", "error", "-show_streams", "-of", "json", str(output)))["streams"]
    return streams


def duration(output):
    report = json.loads(
        run("ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "json", str(output))
    )
    return float(report["format"]["duration"])


def decodes(output):
    run("ffmpeg", "-v", "error", "-i", str(output), "-f", "null", "-")


def subtitle_text(container):
    return run(
        "ffmpeg", "-v", "error", "-i", str(container), "-map", "0:s", "-c:s", "srt", "-f", "srt", "-"
    )


def generate(folder):
    folder.mkdir(parents=True, exist_ok=True)
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
        "-i", "testsrc2=size=320x180:rate=10", "-t", "2", "-c:v", "libx264",
        "-f", "mpegts", str(folder / "v180.ts"))
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
        "-i", "sine=frequency=440:sample_rate=48000", "-t", "2", "-c:a", "aac",
        "-f", "mpegts", str(folder / "audio.ts"))
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
        "-i", "sine=frequency=880:sample_rate=48000", "-t", "2", "-c:a", "libmp3lame",
        "-f", "mpegts", str(folder / "mp3.ts"))
    (folder / "subs.vtt").write_text("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello from XDM\n\n00:00:01.000 --> 00:00:02.000\nsecond cue\n")
    playlists = {
        "v180.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nv180.ts\n#EXT-X-ENDLIST\n",
        "audio.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.ts\n#EXT-X-ENDLIST\n",
        "mp3audio.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nmp3.ts\n#EXT-X-ENDLIST\n",
        "subs.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsubs.vtt\n#EXT-X-ENDLIST\n",
        "master.m3u8": '#EXTM3U\n'
                       '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="sound",DEFAULT=YES,LANGUAGE="en",URI="audio.m3u8"\n'
                       '#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="s",NAME="subs",DEFAULT=YES,LANGUAGE="en",URI="subs.m3u8"\n'
                       '#EXT-X-STREAM-INF:BANDWIDTH=600000,RESOLUTION=320x180,AUDIO="a",SUBTITLES="s"\n'
                       'v180.m3u8\n',
        "mp3master.m3u8": '#EXTM3U\n'
                          '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="sound",DEFAULT=YES,LANGUAGE="en",URI="mp3audio.m3u8"\n'
                          '#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="s",NAME="subs",DEFAULT=YES,LANGUAGE="en",URI="subs.m3u8"\n'
                          '#EXT-X-STREAM-INF:BANDWIDTH=600000,RESOLUTION=320x180,AUDIO="a",SUBTITLES="s"\n'
                          'v180.m3u8\n',
    }
    for name, text in playlists.items():
        (folder / name).write_text(text)


def main():
    if not TOOL.exists():
        raise SystemExit("build first: cargo build -p xdm-media --examples")
    if not shutil.which("ffmpeg") and (Path.home() / ".local/bin/ffmpeg").is_file():
        os.environ["PATH"] = f"{Path.home() / '.local/bin'}:{os.environ['PATH']}"
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")
    with tempfile.TemporaryDirectory(prefix="xdm-audio-test-") as directory:
        folder = Path(directory)
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
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            url = f"{base}/master.m3u8"
            mp3_url = f"{base}/mp3master.m3u8"
            cookie = ["--header", f"Cookie: {COOKIE}"]

            tracks = json.loads(run(str(TOOL), "inspect", url))
            audio = next(t["index"] for t in tracks if t["kind"] == "audio")
            video = next(t["index"] for t in tracks if t["kind"] == "video")
            subs = [t["index"] for t in tracks if t["kind"] == "subtitle"]
            audio_metadata = next(t for t in tracks if t["kind"] == "audio")
            assert audio_metadata["codec"] == "aac", tracks
            assert audio_metadata.get("language") == "en", tracks
            assert subs, tracks
            assert next(t for t in tracks if t["kind"] == "subtitle").get("default") is True, tracks

            # Subtitles ride along into MP4 (mov_text) and MKV (copy).
            mp4 = folder / "movie.mp4"
            run(str(TOOL), "download", url, str(mp4), str(video), str(audio), "--subtitles", str(subs[0]))
            streams = probe(mp4)
            assert any(
                stream["codec_type"] == "subtitle" and stream["codec_name"] == "mov_text"
                for stream in streams
            ), streams
            assert "Hello from XDM" in subtitle_text(mp4)
            decodes(mp4)

            mkv = folder / "movie.mkv"
            run(str(TOOL), "download", url, str(mkv), str(video), str(audio), "--subtitles", str(subs[0]))
            streams = probe(mkv)
            assert any(
                stream["codec_type"] == "subtitle" and stream["codec_name"] == "webvtt"
                for stream in streams
            ), streams
            decodes(mkv)

            # Sidecar saves: .srt and .vtt next to the matched output basename.
            srt = folder / "movie.srt"
            run(str(TOOL), "subtitles", url, str(srt), str(subs[0]))
            srt_text = srt.read_text()
            assert "Hello from XDM" in srt_text and "-->" in srt_text, srt_text
            vtt = folder / "movie.vtt"
            run(str(TOOL), "subtitles", url, str(vtt), str(subs[0]))
            vtt_text = vtt.read_text()
            assert vtt_text.startswith("WEBVTT") and "Hello from XDM" in vtt_text, vtt_text
            fail(str(TOOL), "subtitles", url, str(folder / "notes.txt"), str(subs[0]),
                 marker=".srt or .vtt")

            # Audio-only MP3 transcode keeps the source duration.
            song = folder / "song.mp3"
            run(str(TOOL), "audio", url, str(song), str(audio), "mp3:96")
            streams = probe(song)
            assert streams[0]["codec_type"] == "audio" and streams[0]["codec_name"] == "mp3", streams
            assert 1.6 <= duration(song) <= 3.0, duration(song)
            decodes(song)

            # M4a transcode and original stream-copy of the AAC track.
            encoded = folder / "song.m4a"
            run(str(TOOL), "audio", url, str(encoded), str(audio), "m4a:64")
            streams = probe(encoded)
            assert streams[0]["codec_name"] == "aac", streams
            decodes(encoded)

            copied = folder / "copy.m4a"
            run(str(TOOL), "audio", url, str(copied), str(audio), "original")
            streams = probe(copied)
            assert streams[0]["codec_name"] == "aac", streams
            assert 1.6 <= duration(copied) <= 3.0, duration(copied)
            decodes(copied)

            # Opus transcode into .opus.
            opus = folder / "song.opus"
            run(str(TOOL), "audio", url, str(opus), str(audio), "opus:48")
            streams = probe(opus)
            assert streams[0]["codec_name"] == "opus", streams
            decodes(opus)

            # MP3 source track stream-copies straight into .mp3.
            mp3_stream = json.loads(run(str(TOOL), "inspect", mp3_url))
            mp3_track = next(t["index"] for t in mp3_stream if t["kind"] == "audio")
            original_mp3 = folder / "original.mp3"
            run(str(TOOL), "audio", mp3_url, str(original_mp3), str(mp3_track), "original")
            streams = probe(original_mp3)
            assert streams[0]["codec_name"] == "mp3", streams
            decodes(original_mp3)

            # Extension/format mismatches never produce output files.
            fail(str(TOOL), "audio", mp3_url, str(folder / "wrong.m4a"), str(mp3_track), "original",
                 marker="does not fit")
            assert not (folder / "wrong.m4a").exists()
            fail(str(TOOL), "audio", url, str(folder / "bad.opus"), str(audio), "mp3:96",
                 marker=".mp3 extension")
            assert not (folder / "bad.opus").exists()
            fail(str(TOOL), "audio", url, str(folder / "noext"), str(audio), "mp3:96",
                 marker="file extension")

            # Existing outputs are never overwritten.
            keep = folder / "keep.mp3"
            keep.write_bytes(b"keep")
            fail(str(TOOL), "audio", url, str(keep), str(audio), "mp3:96", marker="already exists")
            assert keep.read_bytes() == b"keep"

            # Header-protected manifest: refused without the cookie, fine with it.
            gated = f"{base}/gated/master.m3u8"
            fail(str(TOOL), "inspect", gated, marker="403")
            fail(str(TOOL), "download", gated, str(folder / "priv.mp4"), str(video), str(audio),
                 marker="403")
            gated_tracks = json.loads(run(str(TOOL), "inspect", gated, *cookie))
            gated_audio = next(t["index"] for t in gated_tracks if t["kind"] == "audio")
            gated_video = next(t["index"] for t in gated_tracks if t["kind"] == "video")
            priv = folder / "priv.mp3"
            run(str(TOOL), "audio", gated, str(priv), str(gated_audio), "mp3:96", *cookie)
            assert probe(priv)[0]["codec_name"] == "mp3"
            decodes(priv)
            priv_mp4 = folder / "priv.mp4"
            run(str(TOOL), "download", gated, str(priv_mp4), str(gated_video), str(gated_audio), *cookie)
            decodes(priv_mp4)
        finally:
            server.shutdown()
            server.server_close()
    print("audio-only formats, subtitle mux and sidecars, header-gated manifests, and non-overwrite checked")


if __name__ == "__main__":
    main()
