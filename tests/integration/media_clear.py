"""End-to-end clear HLS/DASH stream-copy check using generated local media."""
import json
import shutil
import os
import struct
import subprocess
import tempfile
import threading
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise AssertionError(f"{args}: {result.stderr}")
    return result.stdout


def exchange(writer, reader, payload):
    data = json.dumps(payload).encode()
    writer.write(struct.pack("=I", len(data)) + data)
    writer.flush()
    size = struct.unpack("=I", reader.read(4))[0]
    return json.loads(reader.read(size))


def generate(folder):
    for height in (180, 360):
        run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
            "-i", f"testsrc2=size={height * 16 // 9}x{height}:rate=10",
            "-t", "2", "-c:v", "mpeg2video", "-f", "mpegts",
            str(folder / f"v{height}.ts"))
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
        "-i", "sine=frequency=440:sample_rate=48000", "-t", "2", "-c:a", "aac",
        "-f", "mpegts", str(folder / "audio.ts"))
    for height in (180, 360):
        (folder / f"v{height}.m3u8").write_text(
            f"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nv{height}.ts\n#EXT-X-ENDLIST\n")
    (folder / "audio.m3u8").write_text(
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.ts\n#EXT-X-ENDLIST\n")
    (folder / "master.m3u8").write_text(
        '#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="sound",'
        'DEFAULT=YES,URI="audio.m3u8"\n'
        '#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=320x180,AUDIO="a"\n'
        'v180.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,'
        'RESOLUTION=640x360,AUDIO="a"\nv360.m3u8\n')
    run("ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(folder / "v180.ts"),
        "-i", str(folder / "v360.ts"), "-i", str(folder / "audio.ts"),
        "-map", "0:v", "-map", "1:v", "-map", "2:a", "-c:v", "libx264",
        "-c:a", "aac", "-f", "dash", "-adaptation_sets", "id=0,streams=v id=1,streams=a",
        str(folder / "multi.mpd"))


def main():
    if not CLI.exists():
        raise SystemExit("build first: cargo build -p xdm-cli")
    for binary in ("ffmpeg", "ffprobe"):
        if not shutil.which(binary):
            raise SystemExit(f"{binary} required on PATH")
    with tempfile.TemporaryDirectory(prefix="xdm-media-test-") as directory:
        folder = Path(directory)
        generate(folder)

        class Quiet(SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=directory, **kwargs)

            def log_message(self, *_args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Quiet)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            for format_name, manifest in (("hls", "master.m3u8"), ("dash", "multi.mpd")):
                url = f"{base}/{manifest}"
                tracks = json.loads(run(str(CLI), "media", "inspect", url))
                audio = next(track["index"] for track in tracks if track["kind"] == "audio")
                for height in (180, 360):
                    video = next(track["index"] for track in tracks if track["height"] == height)
                    output = folder / f"{format_name}-{height}.mp4"
                    run(str(CLI), "media", "download", url, str(output), str(video), str(audio))
                    streams = json.loads(run("ffprobe", "-v", "error", "-show_streams", "-of", "json", str(output)))["streams"]
                    assert any(stream["codec_type"] == "video" and stream["height"] == height for stream in streams)
                    assert any(stream["codec_type"] == "audio" for stream in streams)
                    run("ffmpeg", "-v", "error", "-i", str(output), "-f", "null", "-")
            env = os.environ.copy()
            env.update({"XDM_DATA_DIR": str(folder / "state"), "HOME": str(folder / "home")})
            (folder / "home").mkdir()
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            host = None
            try:
                assert "listening locally" in daemon.stderr.readline()
                host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                url = f"{base}/master.m3u8"
                inspection = exchange(host.stdin, host.stdout, {"version": 1, "type": "media.inspect", "payload": {"url": url}})
                assert inspection["ok"], inspection
                tracks = inspection["tracks"]
                video = next(track["index"] for track in tracks if track["height"] == 360)
                audio = next(track["index"] for track in tracks if track["kind"] == "audio")
                submission = {"version": 1, "requestId": "local-clear-media-1", "type": "media.submit",
                              "payload": {"url": url, "videoIndex": video, "audioIndex": audio}}
                first = exchange(host.stdin, host.stdout, submission)
                duplicate = exchange(host.stdin, host.stdout, submission)
                assert first["accepted"] and first["taskId"] == duplicate["taskId"]
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    listed = json.loads(subprocess.check_output(
                        [str(CLI), "daemon", "list"], env=env, text=True, timeout=5))["tasks"]
                    if listed[0]["status"] == "complete":
                        break
                    if listed[0]["status"] == "failed":
                        raise AssertionError(listed[0])
                    time.sleep(0.1)
                assert listed[0]["status"] == "complete" and listed[0]["kind"] == "media", listed
                queued_output = Path(listed[0]["output"])
                assert queued_output.is_file() and queued_output.suffix == ".mp4"
                queued_streams = json.loads(run("ffprobe", "-v", "error", "-show_streams", "-of", "json", str(queued_output)))["streams"]
                assert any(track["codec_type"] == "video" and track["height"] == 360 for track in queued_streams)
                assert any(track["codec_type"] == "audio" for track in queued_streams)
                run("ffmpeg", "-v", "error", "-i", str(queued_output), "-f", "null", "-")
            finally:
                if host:
                    host.stdin.close()
                    assert host.wait(timeout=5) == 0
                daemon.terminate()
                daemon.wait(timeout=5)
            (folder / "audio.m3u8").write_text(
                '#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI="key.bin"\n#EXTINF:2,\naudio.ts\n')
            rejected = subprocess.run((str(CLI), "media", "inspect", f"{base}/master.m3u8"),
                                      capture_output=True, text=True, timeout=30)
            assert rejected.returncode != 0 and "encrypted HLS" in rejected.stderr
        finally:
            server.shutdown()
            server.server_close()
    print("clear HLS/DASH high+low with audio, native host queue, decode, and encrypted-audio refusal passed")


if __name__ == "__main__":
    main()
