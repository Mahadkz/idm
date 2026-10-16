"""Video panel hand-off: the native host offers a choice to a running GUI (which polls the daemon), and queues
it directly when no GUI is polling. Real daemon and native host on temp paths; no external network."""

import json
import os
import socket
import struct
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def exchange(host, payload):
    data = json.dumps(payload).encode()
    host.stdin.write(struct.pack("=I", len(data)) + data)
    host.stdin.flush()
    size = struct.unpack("=I", host.stdout.read(4))[0]
    return json.loads(host.stdout.read(size))


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-video-offer-") as temp:
        base = Path(temp)
        (base / "home").mkdir()
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "HOME": str(base / "home")}
        env.pop("XDG_DOWNLOAD_DIR", None)
        (base / "empty").mkdir()
        env["PATH"] = str(base / "empty")  # no yt-dlp on PATH
        daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
        host = None
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

            host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env, stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            hello = exchange(host, {"version": 1, "type": "hello"})
            assert "video.offer" in hello["capabilities"] and "video.probe" in hello["capabilities"], hello

            def offer(request_id, **extra):
                payload = {"url": "https://rr1.googlevideo.com/videoplayback?itag=137", "filenameHint": "Clip [1080p].mp4",
                           "title": "Clip", "size": 123456, "requestContext": {"headers": [{"name": "User-Agent", "value": "T/1"}],
                           "referrer": "https://www.youtube.com/"}, **extra}
                return exchange(host, {"version": 1, "requestId": request_id, "type": "video.offer", "payload": payload})

            adaptive = {"mode": "adaptive", "audioUrl": "https://rr1.googlevideo.com/videoplayback?itag=140", "container": "mp4"}

            # No GUI has polled yet: the host queues the choice itself (adaptive pair -> one media task).
            queued = offer("queue-1", **adaptive)
            assert queued["accepted"] and not queued.get("offered"), queued
            tasks = rpc({"type": "list"})["tasks"]
            assert len(tasks) == 1 and tasks[0]["kind"] == "media" and tasks[0]["output"].endswith("Clip [1080p].mp4"), tasks

            # A GUI polls: offers are held for it, with every field preserved, and drained once.
            assert rpc({"type": "take_offers"})["offers"] == []
            shown = offer("offer-1", **adaptive)
            assert shown["accepted"] and shown["offered"], shown
            single = offer("offer-2", url="https://rr1.googlevideo.com/videoplayback?itag=18")
            assert single["offered"], single
            hls = offer("offer-3", url="https://cdn.example/master.m3u8", mode="video", videoIndex=2, audioIndex=5)
            assert hls["offered"], hls
            assert len(rpc({"type": "list"})["tasks"]) == 1, "offered choices are not queued behind the user's back"
            taken = rpc({"type": "take_offers"})["offers"]
            assert [o["id"] for o in taken] == ["offer-1", "offer-2", "offer-3"], taken
            first, second, third = taken
            assert first["audio_url"].endswith("itag=140") and first["container"] == "mp4" and first["filename"] == "Clip [1080p].mp4"
            assert first["size"] == 123456 and first["title"] == "Clip"
            assert ["User-Agent", "T/1"] in first["headers"] and ["Referer", "https://www.youtube.com/"] in first["headers"], first
            assert second["audio_url"] is None and third["stream_video"] == 2 and third["stream_audio"] == 5, taken
            assert rpc({"type": "take_offers"})["offers"] == []

            # Invalid offers are refused before reaching the GUI.
            for bad in (offer("bad-1", url="file:///etc/passwd"), offer("bad-2", **{**adaptive, "audioUrl": "ftp://x/y"}),
                        offer("bad-3", filenameHint="../x.mp4"), offer("bad-4", url="https://u:p@x.test/a.mp4")):
                assert bad["accepted"] is False, bad

            # yt-dlp choices: the page address plus format ids; the offer carries the selector, never stream URLs.
            page = {"url": "https://www.youtube.com/watch?v=aaaaaaaaaaa", "mode": "ytdlp", "ytdlpFormat": "299+140", "container": "mp4"}
            assert rpc({"type": "take_offers"})["offers"] == []
            shown = offer("yt-1", **page)
            assert shown["accepted"] and shown["offered"], shown
            got = rpc({"type": "take_offers"})["offers"][0]
            assert got["ytdlp_format"] == "299+140" and got["url"].startswith("https://www.youtube.com/watch") and got["audio_url"] is None, got
            for evil in ("-o /tmp/x", "a+b+c", "best[height<=1080]", "", "--exec"):
                assert offer("yt-bad", **{**page, "ytdlpFormat": evil})["accepted"] is False, evil

            # A format probe through the host: no yt-dlp installed here, so a clear flagged error comes back.
            probed = exchange(host, {"version": 1, "type": "video.probe", "payload": {"url": "https://www.youtube.com/watch?v=aaaaaaaaaaa", "cookies": []}})
            assert probed["type"] == "video.probe.result" and probed["ok"] is False and probed["ytdlpMissing"] is True, probed
            assert "yt-dlp was not found" in probed["reason"], probed
            assert exchange(host, {"version": 1, "type": "video.probe", "payload": {"url": "file:///etc/passwd"}})["ok"] is False
            assert exchange(host, {"version": 1, "type": "video.probe", "payload": {"url": "https://u:p@x.test/"}})["ok"] is False

            # The GUI stops polling: offers fall back to direct queueing after the polling window lapses.
            time.sleep(5.5)
            late = offer("late-1", **adaptive, filenameHint="Late.mp4")
            assert late["accepted"] and not late.get("offered"), late
            print("video_offer ok")
        finally:
            if host:
                host.kill()
            daemon.terminate()


if __name__ == "__main__":
    main()
