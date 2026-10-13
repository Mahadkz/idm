"""Desktop task dialogs on a dedicated Xvfb display; never uses the user's display."""
import json
import os
import shutil
import sqlite3
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = b"payload for gui actions " * 1024
HLS_DIR = None


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if HLS_DIR and self.path.startswith("/hls/"):
            rel = self.path[len("/hls/"):].split("?")[0]
            target = HLS_DIR / rel
            if target.is_file():
                content = target.read_bytes()
                self.send_response(200)
                if target.suffix == ".m3u8":
                    self.send_header("Content-Type", "application/vnd.apple.mpegurl")
                elif target.suffix == ".ts":
                    self.send_header("Content-Type", "video/mp2t")
                elif target.suffix == ".vtt":
                    self.send_header("Content-Type", "text/vtt")
                self.send_header("Content-Length", str(len(content)))
                self.end_headers()
                self.wfile.write(content)
                return
            self.send_response(404)
            self.end_headers()
            return

        self.send_response(200)
        self.send_header("Content-Length", str(len(PAYLOAD)))
        self.end_headers()
        self.wfile.write(PAYLOAD)

    def log_message(self, *args):
        pass


def generate_hls_fixture(folder):
    folder.mkdir(parents=True, exist_ok=True)
    ffmpeg_bin = shutil.which("ffmpeg") or "/home/mahad/.local/bin/ffmpeg"
    subprocess.run([ffmpeg_bin, "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                    "-i", "testsrc2=size=320x180:rate=10", "-t", "2", "-c:v", "libx264",
                    "-f", "mpegts", str(folder / "v180.ts")], check=True)
    subprocess.run([ffmpeg_bin, "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                    "-i", "sine=frequency=440:sample_rate=48000", "-t", "2", "-c:a", "aac",
                    "-f", "mpegts", str(folder / "audio.ts")], check=True)
    subprocess.run([ffmpeg_bin, "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                    "-i", "sine=frequency=880:sample_rate=48000", "-t", "2", "-c:a", "libmp3lame",
                    "-f", "mpegts", str(folder / "mp3.ts")], check=True)
    (folder / "subs.vtt").write_text("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello from XDM\n")
    playlists = {
        "v180.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nv180.ts\n#EXT-X-ENDLIST\n",
        "audio.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.ts\n#EXT-X-ENDLIST\n",
        "subs.m3u8": "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nsubs.vtt\n#EXT-X-ENDLIST\n",
        "master.m3u8": '#EXTM3U\n'
                       '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="sound",DEFAULT=YES,LANGUAGE="en",URI="audio.m3u8"\n'
                       '#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="s",NAME="subs",DEFAULT=YES,LANGUAGE="en",URI="subs.m3u8"\n'
                       '#EXT-X-STREAM-INF:BANDWIDTH=600000,RESOLUTION=320x180,AUDIO="a",SUBTITLES="s"\n'
                       'v180.m3u8\n',
    }
    for name, text in playlists.items():
        (folder / name).write_text(text)



def main():
    with tempfile.TemporaryDirectory(prefix="xdm-gui-actions-") as directory:
        base = Path(directory)
        global HLS_DIR
        HLS_DIR = base / "hls"
        generate_hls_fixture(HLS_DIR)
        server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        port = server.server_address[1]

        virtual = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1200x900x24"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        display = virtual.stdout.readline().strip()
        assert display.isdigit(), "Xvfb did not allocate a display"
        env = {**os.environ, "DISPLAY": f":{display}", "SLINT_BACKEND": "winit-software",
               "XDM_DATA_DIR": str(base / "state"), "XDM_FAKE_OS_ACTIONS": "1"}
        env.pop("WAYLAND_DISPLAY", None)
        daemon = gui = None

        def cli(*args):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=5, check=True)
            return json.loads(result.stdout)

        def until(action):
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                value = action()
                if value:
                    return value
                time.sleep(0.1)
            raise AssertionError("GUI condition timed out")

        def window(name):
            result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], env=env,
                                    capture_output=True, text=True, timeout=5)
            return result.stdout.splitlines()[-1] if result.returncode == 0 else None

        def click(identifier, x, y):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=False, timeout=5)
            subprocess.run(["xdotool", "mousemove", "--window", identifier, str(x), str(y), "click", "1"],
                           env=env, check=True, timeout=5)
            time.sleep(0.2)

        def type_field(identifier, x, y, value):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=True, timeout=5)
            click(identifier, x, y)
            subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+a"], env=env, check=True, timeout=5)
            subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "0", str(value)], env=env, check=True, timeout=5)

        try:
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                      stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            cli("queue", "stop")
            source = cli("submit", "http://example.invalid/file.bin", base / "file.bin")["task_id"]
            gui_log = open(base / "gui.log", "w")
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=env,
                                   stderr=gui_log, text=True)
            main_window = until(lambda: window("XDM.*Downloads$"))
            time.sleep(1)
            click(main_window, 120, 400)
            click(main_window, 130, 137)
            details = until(lambda: window("XDM.*Download details$"))
            click(details, 320, 450)
            until(lambda: window("XDM.*Download details$") is None)
            click(main_window, 410, 137)
            redownload = until(lambda: window("XDM.*Redownload$"))
            click(redownload, 160, 228)
            rows = until(lambda: cli("list")["tasks"] if len(cli("list")["tasks"]) == 2 else None)
            clone = next(row for row in rows if row["id"] != source)
            assert clone["output"] == str(base / "file-again.bin") and clone["status"] == "queued"
            click(main_window, 120, 400)
            click(main_window, 700, 137)
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)
            until(lambda: len(cli("list")["tasks"]) == 1)
            assert cli("list")["tasks"][0]["id"] == source
            urls = base / "urls.txt"
            urls.write_text("http://example.invalid/b.bin\nhttp://example.invalid/c.bin\n")
            click(main_window, 340, 195)
            lists = until(lambda: window("XDM.*Import and export$"))
            type_field(lists, 200, 70, urls)
            type_field(lists, 200, 118, base / "batch")
            click(lists, 116, 172)
            click(lists, 330, 172)
            until(lambda: len(cli("list")["tasks"]) == 3)
            assert all(row["status"] == "paused" for row in cli("list")["tasks"] if row["id"] != source)
            exported = base / "gui-export.json"
            type_field(lists, 200, 432, exported)
            click(lists, 130, 486)
            until(exported.exists)
            assert len(json.loads(exported.read_text())["entries"]) == 3
            click(lists, 580, 486)
            until(lambda: window("XDM.*Import and export$") is None)
            click(main_window, 120, 400)
            selected = cli("list")["tasks"][0]["id"]
            click(main_window, 825, 28)
            queues = until(lambda: window("XDM.*Queues$"))
            type_field(queues, 160, 286, "Work")
            type_field(queues, 480, 286, "1")
            click(queues, 82, 342)
            state = until(lambda: cli("list") if len(cli("list")["queues"]) == 2 else None)
            custom = next(row for row in state["queues"] if row["name"] == "Work")
            assert custom["limit"] == 1 and custom["running"] is False
            click(queues, 390, 446)
            until(lambda: cli("info", selected)["tasks"][0]["queue_id"] == custom["id"])
            click(queues, 130, 398)
            until(lambda: next(row for row in cli("list")["queues"] if row["id"] == custom["id"])["running"])

            # Exercise independent queue start/stop and settings dialog via Queues window
            cli("queue", "stop")
            until(lambda: cli("list")["queue_running"] is False)
            click(queues, 130, 492)
            until(lambda: next(row for row in cli("list")["queues"] if row["id"] == custom["id"])["started_alone"])
            click(queues, 390, 492)
            until(lambda: not next(row for row in cli("list")["queues"] if row["id"] == custom["id"])["started_alone"])

            # Exercise Queue settings dialog (completion actions with opt-in and countdown)
            click(queues, 650, 492)
            settings = until(lambda: window("XDM.*Queue settings$"))

            # 1. Save Notify completion action via GUI
            click(settings, 217, 370)  # "Notify" button
            click(settings, 700, 425)  # "Apply completion action" button
            until(lambda: next(r for r in cli("list")["queues"] if r["id"] == custom["id"]).get("completion_action") == "notify")

            # 2. Select Shutdown and set countdown to 30 s
            click(settings, 495, 370)  # "Shutdown" button
            type_field(settings, 250, 425, "30")  # Countdown input field

            # 3. Apply without confirmation checkbox: rejected by opt-in check, remains notify
            click(settings, 700, 425)  # "Apply completion action" button
            time.sleep(0.3)
            assert next(r for r in cli("list")["queues"] if r["id"] == custom["id"]).get("completion_action") == "notify"

            # 4. Check the explicit confirmation checkbox and apply
            click(settings, 28, 473)   # "I understand this will shutdown the computer" checkbox
            click(settings, 700, 425)  # "Apply completion action" button
            until(lambda: next(r for r in cli("list")["queues"] if r["id"] == custom["id"]).get("completion_action") == "shutdown")

            # 5. Verify settings stored in sqlite database
            db = sqlite3.connect(base / "state" / "tasks.sqlite3")
            row = db.execute("SELECT completion_action, completion_confirmed, completion_countdown FROM queues WHERE id=?",
                             (custom["id"],)).fetchone()
            assert row == ("shutdown", 1, 30), f"Unexpected queue db row: {row}"

            click(settings, 600, 620)  # Close button in Queue settings
            until(lambda: window("XDM.*Queue settings$") is None)
            click(queues, 620, 398)    # Show selected queue / close queues
            until(lambda: window("XDM.*Queues$") is None)

            # Exercise Task Move Up and Down in the task list
            second = cli("submit", "http://example.invalid/second.bin", base / "second.bin")["task_id"]
            cli("queue", "assign", custom["id"], second)
            click(main_window, 835, 76)  # Refresh button in row 2
            time.sleep(1.0)
            # Select second task (row 1 in list view)
            click(main_window, 120, 460)
            # Click "Up" button (x=685, y=365)
            click(main_window, 685, 365)
            until(lambda: next(t for t in cli("list")["tasks"] if t["id"] == second)["position"] == 1)
            # Click "Down" button (x=755, y=365)
            click(main_window, 755, 365)
            until(lambda: next(t for t in cli("list")["tasks"] if t["id"] == second)["position"] == 2)
            # Click "Top" button (x=618, y=365)
            click(main_window, 618, 365)
            until(lambda: next(t for t in cli("list")["tasks"] if t["id"] == second)["position"] == 1)
            # Click "Bottom" button (x=835, y=365)
            click(main_window, 835, 365)
            until(lambda: next(t for t in cli("list")["tasks"] if t["id"] == second)["position"] == 2)

            # Exercise Batch download dialog
            click(main_window, 194, 76)  # "Batch…" button in row 2
            batch_win = until(lambda: window("XDM.*Add batch download$"))
            type_field(batch_win, 300, 95, "http://example.invalid/batch[01-03].bin")
            type_field(batch_win, 300, 265, base / "batch_out")
            click(batch_win, 160, 408)  # "Add batch" button
            until(lambda: window("XDM.*Add batch download$") is None)
            until(lambda: len([t for t in cli("list")["tasks"] if "batch" in t["url"]]) == 3)

            # Exercise Categories editor dialog
            click(main_window, 439, 76)  # "Categories…" button in row 2
            cats_win = until(lambda: window("XDM.*Categories$"))
            until(lambda: len(cli("category", "list")["categories"]) >= 6)
            type_field(cats_win, 300, 285, "Archives")
            type_field(cats_win, 300, 340, "zip, 7z, tar")
            type_field(cats_win, 300, 400, str(base / "archives"))
            type_field(cats_win, 300, 460, "*.files.invalid")
            click(cats_win, 81, 520)  # "Save / Update" button
            until(lambda: any(c["name"] == "Archives" for c in cli("category", "list")["categories"]))
            cat_rule = next(c for c in cli("category", "list")["categories"] if c["name"] == "Archives")
            assert "zip" in cat_rule["extensions"] and "7z" in cat_rule["extensions"]
            click(cats_win, 637, 520)  # "Close" button
            until(lambda: window("XDM.*Categories$") is None)

            # Exercise Connection settings page
            click(main_window, 561, 76)  # "Connection…" button in row 2
            conn_win = until(lambda: window("XDM.*Connection settings$"))
            click(conn_win, 390, 105)   # "Direct (No Proxy)" button
            type_field(conn_win, 500, 390, "4")  # Default connections field
            click(conn_win, 130, 480)  # "Save settings" button
            until(lambda: cli("proxy", "get")["proxy"] == {"mode": "direct"})
            gui_settings_file = base / "state" / "gui-settings.json"
            until(lambda: gui_settings_file.exists() and json.loads(gui_settings_file.read_text()).get("default_connections") == 4)
            click(conn_win, 570, 480)  # "Close" button
            until(lambda: window("XDM.*Connection settings$") is None)

            # Exercise Preferences dialog (toggle dark theme and verify persistence)
            click(main_window, 820, 245)  # "Preferences…" button in search row
            pref_win = until(lambda: window("XDM.*Preferences$"))
            click(pref_win, 580, 95)      # "Dark" theme button
            click(pref_win, 180, 570)     # "Save preferences" button
            until(lambda: gui_settings_file.exists() and json.loads(gui_settings_file.read_text()).get("theme") == "dark")
            click(pref_win, 540, 570)     # "Close" button
            until(lambda: window("XDM.*Preferences$") is None)

            # Exercise Add media dialog with HLS inspection and audio-only job submission
            click(main_window, 530, 195)  # "Add media…" button in toolbar
            media_win = until(lambda: window("XDM.*Add media$"))
            type_field(media_win, 400, 70, f"http://127.0.0.1:{port}/hls/master.m3u8")
            type_field(media_win, 400, 130, "Cookie: xdm=audio_test\nReferer: http://127.0.0.1/")
            click(media_win, 705, 70)   # "Inspect" button
            # Inspection runs off-UI thread; wait for response
            time.sleep(2.0)
            click(media_win, 305, 345)  # "Audio only" mode button
            click(media_win, 585, 400)  # "MP3" format button
            media_dest = base / "media-audio.mp3"
            type_field(media_win, 400, 515, str(media_dest))
            click(media_win, 200, 600)  # "Download" button
            until(lambda: window("XDM.*Add media$") is None)

            media_task = until(lambda: next((t for t in cli("list")["tasks"] if t["output"] == str(media_dest)), None))
            assert media_task["kind"] == "media"
            assert media_task["category"] == "Music"

            # Exercise Live recording media task label change ("Stop and save" vs "Pause")
            live_dest = base / "live-rec.mp4"
            live_task_id = cli("media-job", f"http://127.0.0.1:{port}/hls/master.m3u8", str(live_dest),
                               "live", "2", "0", "--max-seconds", "30")["task_id"]
            click(main_window, 835, 76)  # Refresh button
            live_task = until(lambda: next((t for t in cli("list")["tasks"] if t["id"] == live_task_id), None))
            assert live_task["kind"] == "media"
            # In main window, select live task in tasks listview
            click(main_window, 120, 410)
            time.sleep(0.5)
            # Click row 1 to select standard task
            click(main_window, 120, 450)
            time.sleep(0.5)

            # Exercise Countdown banner and Cancel button in main window
            banner_task = cli("submit", f"http://127.0.0.1:{port}/complete.bin", base / "banner-test.bin")["task_id"]
            banner_queue = cli("queue", "create", "BannerQueue", 1)["queue_id"]
            cli("queue", "completion", banner_queue, "shutdown", "--confirm-destructive", "--countdown", 30)
            cli("queue", "assign", banner_queue, banner_task)
            cli("queue", "start-one", banner_queue)

            # Daemon arms completion pending
            pending = until(lambda: [r for r in cli("completion", "status")["completion_pending"] if r["queue_id"] == banner_queue])
            assert pending[0]["action"] == "shutdown"
            assert 0 < pending[0]["seconds_remaining"] <= 30

            # Wait for GUI periodic poll (every 5s when idle) to catch pending action, then click Cancel
            cancelled = False
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                time.sleep(0.8)
                click(main_window, 838, 38)
                if not [r for r in cli("completion", "status")["completion_pending"] if r["queue_id"] == banner_queue]:
                    cancelled = True
                    break
            assert cancelled, "Completion was not cancelled by banner button click"
            events = cli("queue", "events", banner_queue)["queue_events"]
            assert any(e.get("action_status") == "cancelled" for e in events), f"Expected cancelled event, got {events}"

            assert gui.poll() is None
            print("isolated GUI task actions, batch dialog, categories editor, connection settings, preferences dialog, add media dialog, and completion actions passed")
        finally:
            log_path = base / "gui.log"
            if log_path.exists():
                print("GUI LOG:\n" + log_path.read_text())
            for process in [gui, daemon, virtual]:
                if process and process.poll() is None:
                    process.terminate(); process.wait(timeout=5)


if __name__ == "__main__":
    main()
