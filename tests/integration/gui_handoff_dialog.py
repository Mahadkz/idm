"""Browser handoff with no GUI running: the daemon starts the GUI, which shows Download File Info.
Start Download runs the task, Download Later queues without starting, Cancel creates nothing, and a second
xdm-gui launch opens no second window. Private Xvfb and data dir; every process started is killed at the end."""
import http.server
import json
import os
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHOTS = Path(os.environ.get("XDM_SHOT_DIR", "/tmp/xdm-handoff-shots"))
SIZE = 200_000


class Slow(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(SIZE))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        try:
            for _ in range(SIZE // 1000):
                self.wfile.write(b"x" * 1000)
                time.sleep(0.2)
        except OSError:
            pass


def procs_with_data_dir(state):
    found = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                if f"XDM_DATA_DIR={state}".encode() in (entry / "environ").read_bytes().split(b"\0"):
                    found.append(int(entry.name))
            except OSError:
                pass
    return found


def main():
    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
    server = Server(("127.0.0.1", 0), Slow)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_port}"
    virtual = host = daemon = None
    with tempfile.TemporaryDirectory(prefix="xdm-handoff-") as temp:
        base = Path(temp)
        state = base / "state"
        try:
            virtual = subprocess.Popen(["Xvfb", "-displayfd", "1", "-nocursor", "-screen", "0", "1200x900x24"],
                                       stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            display = virtual.stdout.readline().strip()
            assert display.isdigit(), "Xvfb did not allocate a display"
            (base / "home").mkdir()
            env = {**os.environ, "DISPLAY": f":{display}", "SLINT_BACKEND": "winit-software", "HOME": str(base / "home"),
                   "XDM_DATA_DIR": str(state), "XDM_FAKE_OS_ACTIONS": "1", "XDM_GUI_AUTOLAUNCH": "1"}
            env.pop("XDM_NO_GUI_LAUNCH", None)
            for key in ("WAYLAND_DISPLAY", "XDG_DOWNLOAD_DIR"):
                env.pop(key, None)

            def window(name):
                r = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], env=env, capture_output=True, text=True, timeout=30)
                return r.stdout.splitlines()[-1] if r.returncode == 0 and r.stdout.strip() else None

            def until(action, timeout=40):
                end = time.monotonic() + timeout
                while time.monotonic() < end:
                    value = action()
                    if value:
                        return value
                    time.sleep(0.2)
                raise AssertionError("timed out")

            def tasks():
                out = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", "list"], env=env, capture_output=True, text=True, timeout=30).stdout
                return json.loads(out)["tasks"]

            def native(message):
                data = json.dumps(message).encode()
                host.stdin.write(struct.pack("=I", len(data)) + data)
                host.stdin.flush()
                return json.loads(host.stdout.read(struct.unpack("=I", host.stdout.read(4))[0]))

            def offer(n, name):
                reply = native({"version": 1, "requestId": f"hd-{n}", "type": "download.submit",
                                "payload": {"url": f"{origin}/{name}", "filenameHint": name}})
                assert reply["accepted"] and reply.get("offered"), reply

            def click(dialog, x):
                subprocess.run(["xdotool", "windowfocus", dialog], env=env, check=False, timeout=30)
                subprocess.run(["xdotool", "mousemove", "--window", dialog, str(x), "190", "click", "1"], env=env, check=True, timeout=30)

            def gone():
                return window("Download File Info") is None

            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=env, stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            SHOTS.mkdir(parents=True, exist_ok=True)

            offer(1, "start.bin")  # no GUI running: the daemon launches it
            dialog = until(lambda: window("Download File Info"))
            time.sleep(1)
            shot = SHOTS / "handoff-auto-started-dialog.png"
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size", "1200x900", "-i", env["DISPLAY"],
                            "-frames:v", "1", str(shot)], env=env, check=True, timeout=30)
            assert tasks() == [], "nothing is created before the user confirms"
            click(dialog, 278)  # Start Download
            running = until(lambda: tasks())
            assert running[0]["output"].endswith("start.bin") and running[0]["status"] != "paused", running
            until(gone)

            offer(2, "later.bin")
            dialog = until(lambda: window("Download File Info"))
            click(dialog, 151)  # Download Later
            until(lambda: len(tasks()) == 2)
            until(gone)
            later = [t for t in tasks() if t["output"].endswith("later.bin")][0]
            time.sleep(2)
            later = [t for t in tasks() if t["output"].endswith("later.bin")][0]
            assert later["status"] in ("paused", "queued", "stopped") and not later.get("downloaded"), later

            offer(3, "cancel.bin")
            dialog = until(lambda: window("Download File Info"))
            click(dialog, 406)  # Cancel
            until(gone)
            time.sleep(1)
            assert len(tasks()) == 2 and not any(t["output"].endswith("cancel.bin") for t in tasks()), tasks()

            before = [p for p in procs_with_data_dir(state) if (Path("/proc") / str(p) / "exe").resolve().name == "xdm-gui"]
            assert len(before) == 1, before
            second = subprocess.run([str(ROOT / "target/debug/xdm-gui")], env=env, timeout=30)
            assert second.returncode == 0
            after = [p for p in procs_with_data_dir(state) if (Path("/proc") / str(p) / "exe").resolve().name == "xdm-gui"]
            assert after == before, (before, after)
            # Stale socket from a crash: kill -9 the instance, a new launch must start cleanly.
            os.kill(before[0], signal.SIGKILL)
            time.sleep(1)
            fresh = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=env, stderr=subprocess.DEVNULL)
            try:
                until(lambda: window("XDM.*Downloads$") or fresh.poll() is not None)
                assert fresh.poll() is None, "stale lock blocked startup"
                shot2 = SHOTS / "handoff-main-window.png"
                subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size", "1200x900", "-i", env["DISPLAY"],
                                "-frames:v", "1", str(shot2)], env=env, check=True, timeout=30)
            finally:
                fresh.kill()
            # Dialog-only GUI: SIGTERM exits it, and with nothing to do it leaves by itself after ~30 s.
            for process in (host, daemon):
                process.kill()
                process.wait()
            for pid in procs_with_data_dir(state):
                os.kill(pid, signal.SIGKILL)
            time.sleep(1)
            termed = subprocess.Popen([str(ROOT / "target/debug/xdm-gui"), "--dialog-only"], env=env, stderr=subprocess.DEVNULL)
            until(lambda: window("XDM.*Downloads$"))
            termed.terminate()
            assert termed.wait(timeout=10) is not None, "SIGTERM ignored"
            idle = subprocess.Popen([str(ROOT / "target/debug/xdm-gui"), "--dialog-only"], env=env, stderr=subprocess.DEVNULL)
            started = time.monotonic()
            assert idle.wait(timeout=60) == 0
            assert 25 < time.monotonic() - started < 50, "idle dialog-only GUI should exit after about 30 s"
            # No display: the daemon must not launch a GUI and the host queues directly.
            bare = {k: v for k, v in env.items() if k not in ("DISPLAY", "WAYLAND_DISPLAY")}
            bare["XDM_DATA_DIR"] = str(base / "bare")
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=bare, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            host = subprocess.Popen([str(ROOT / "target/debug/xdm-native-host")], env=bare, stdin=subprocess.PIPE,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            reply = native({"version": 1, "requestId": "hd-bare", "type": "download.submit",
                            "payload": {"url": f"{origin}/bare.bin", "filenameHint": "bare.bin"}})
            assert reply["accepted"] and not reply.get("offered"), reply
            assert not [p for p in procs_with_data_dir(base / "bare") if (Path("/proc") / str(p) / "exe").resolve().name == "xdm-gui"]
            print("gui_handoff_dialog ok; screenshots", shot)
        finally:
            for pid in procs_with_data_dir(state) + procs_with_data_dir(base / "bare"):
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
            for process in (host, daemon, virtual):
                if process and process.poll() is None:
                    process.kill()
            server.shutdown()


if __name__ == "__main__":
    main()
