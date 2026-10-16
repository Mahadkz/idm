"""A video-panel choice reaches the running GUI as a Download File Info dialog (same flow as Add URL),
and Start Download queues the adaptive pair, which merges. Dedicated Xvfb display; never the user's."""
import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import media_adaptive as fixture

ROOT = Path(__file__).resolve().parents[2]
SHOTS = Path(os.environ.get("XDM_SHOT_DIR", "/tmp/xdm-video-shots"))


def main():
    fixture.ffmpeg_tools()
    with tempfile.TemporaryDirectory(prefix="xdm-gui-video-") as temp:
        base = Path(temp)
        fixture.make_streams(base / "served")
        fixture.Ranged.folder = base / "served"
        fixture.Ranged.log = []
        server = fixture.QuietServer(("127.0.0.1", 0), fixture.Ranged)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        virtual = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1200x900x24"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        display = virtual.stdout.readline().strip()
        assert display.isdigit(), "Xvfb did not allocate a display"
        (base / "home").mkdir()
        env = {**os.environ, "DISPLAY": f":{display}", "SLINT_BACKEND": "winit-software", "HOME": str(base / "home"),
               "XDM_DATA_DIR": str(base / "state"), "XDM_FAKE_OS_ACTIONS": "1"}
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("XDG_DOWNLOAD_DIR", None)
        daemon = gui = host = None

        def until(action, timeout=60):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if gui and gui.poll() is not None:
                    raise AssertionError(f"xdm-gui exited with {gui.returncode}")
                value = action()
                if value:
                    return value
                time.sleep(0.2)
            raise AssertionError("timed out")

        def window(name):
            result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], env=env, capture_output=True, text=True, timeout=30)
            return result.stdout.splitlines()[-1] if result.returncode == 0 and result.stdout.strip() else None

        def native(message):
            data = json.dumps(message).encode()
            host.stdin.write(struct.pack("=I", len(data)) + data)
            host.stdin.flush()
            return json.loads(host.stdout.read(struct.unpack("=I", host.stdout.read(4))[0]))

        try:
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=env, stderr=open(base / "gui.log", "w"), text=True)
            until(lambda: window("XDM.*Downloads$"))
            time.sleep(3)  # the GUI's first poll marks it present
            host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env, stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            origin = f"http://127.0.0.1:{server.server_port}"
            video_size = (base / "served/video.mp4").stat().st_size
            reply = native({"version": 1, "requestId": "gui-offer-1", "type": "video.offer", "payload": {
                "url": f"{origin}/video.mp4?expire=1", "audioUrl": f"{origin}/audio.m4a?expire=1", "mode": "adaptive",
                "container": "mp4", "filenameHint": "Clip title [240p].mp4", "title": "Clip title", "size": video_size,
                "requestContext": {"headers": [{"name": "X-Xdm-Secret", "value": fixture.SECRET}]}}})
            assert reply["accepted"] and reply.get("offered"), reply
            dialog = until(lambda: window("Download File Info"))
            time.sleep(1)
            SHOTS.mkdir(parents=True, exist_ok=True)
            shot = SHOTS / "xdm-file-info-dialog.png"
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1200x900",
                            "-i", env["DISPLAY"], "-frames:v", "1", str(shot)], env=env, check=True, timeout=30)
            assert not any(t for t in json.loads(subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "list"], env=env, capture_output=True, text=True).stdout)["tasks"]), \
                "nothing is queued before the user confirms"
            # Start Download (button at 223,179 size 111x23 in the dialog).
            subprocess.run(["xdotool", "windowfocus", dialog], env=env, check=False, timeout=30)
            subprocess.run(["xdotool", "mousemove", "--window", dialog, "278", "190", "click", "1"], env=env, check=True, timeout=30)

            def finished():
                tasks = json.loads(subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "list"], env=env, capture_output=True, text=True).stdout)["tasks"]
                if tasks and tasks[0]["status"] == "failed":
                    raise AssertionError(tasks[0])
                return tasks and tasks[0]["status"] == "complete" and tasks[0]
            task = until(finished, 90)
            assert task["kind"] == "media" and task["output"].endswith("Clip title [240p].mp4"), task
            streams = json.loads(subprocess.run(["ffprobe", "-v", "error", "-show_streams", "-of", "json", task["output"]],
                                                check=True, capture_output=True, text=True).stdout)["streams"]
            assert sorted(s["codec_type"] for s in streams) == ["audio", "video"], streams
            print("gui_video_offer ok; screenshot", shot)
        finally:
            for process in (host, gui, daemon, virtual):
                if process and process.poll() is None:
                    process.terminate()
            server.shutdown()


if __name__ == "__main__":
    main()
