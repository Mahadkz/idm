"""Desktop task dialogs on a dedicated Xvfb display; never uses the user's display."""
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = b"payload for gui actions " * 1024
CHECKPOINT = ('{"version":1,"url":"http://x/g","effective_url":"http://x/g","context_sha256":"",'
              '"length":10,"segments":[]}')
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

        body = b"B" * (2 * 1024 * 1024) if self.path.startswith("/big") else PAYLOAD
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

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
                                    env=env, capture_output=True, text=True, timeout=60, check=True)
            return json.loads(result.stdout)

        def until(action, timeout=60):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if gui and gui.poll() is not None:
                    raise AssertionError(f"xdm-gui exited prematurely with code {gui.returncode}")
                if daemon and daemon.poll() is not None:
                    raise AssertionError(f"xdm-daemon exited prematurely with code {daemon.returncode}")
                value = action()
                if value:
                    return value
                time.sleep(0.1)
            raise AssertionError(f"GUI condition timed out after {timeout}s")

        def window(name):
            result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], env=env,
                                    capture_output=True, text=True, timeout=30)
            return result.stdout.splitlines()[-1] if result.returncode == 0 else None

        def click(identifier, x, y):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=False, timeout=30)
            subprocess.run(["xdotool", "mousemove", "--window", identifier, str(x), str(y), "click", "1"],
                           env=env, check=True, timeout=30)
            time.sleep(0.2)

        def rclick(identifier, x, y):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=False, timeout=30)
            subprocess.run(["xdotool", "mousemove", "--window", identifier, str(x), str(y), "click", "3"],
                           env=env, check=True, timeout=30)
            time.sleep(0.3)

        def click_mod(identifier, x, y, modifier):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=False, timeout=30)
            subprocess.run(["xdotool", "mousemove", "--window", identifier, str(x), str(y),
                            "keydown", modifier, "click", "1", "keyup", modifier], env=env, check=True, timeout=30)
            time.sleep(0.2)

        def key(*names):
            subprocess.run(["xdotool", "key", "--clearmodifiers", *names], env=env, check=True, timeout=30)
            time.sleep(0.25)

        def grab(name, box):
            """Screenshot of the isolated display cropped to box=(x0,y0,x1,y1); returns PNG bytes."""
            from PIL import Image
            path = base / f"{name}.png"
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size", "1200x900",
                            "-i", env["DISPLAY"], "-frames:v", "1", str(path)], env=env, check=True, timeout=30)
            return Image.open(path).convert("RGB").crop(box).tobytes()

        def statuses():
            return {t["id"]: t["status"] for t in cli("list")["tasks"]}

        def row_y(task_id):
            # Rows are listed by ascending id, 40px each, first row centred at y=457.
            ids = sorted(t["id"] for t in cli("list")["tasks"])
            return 457 + 40 * ids.index(task_id)

        def type_field(identifier, x, y, value):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=True, timeout=30)
            click(identifier, x, y)
            subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+a"], env=env, check=True, timeout=30)
            subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "0", str(value)], env=env, check=True, timeout=30)

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
            click(main_window, 120, 457)
            click(main_window, 72, 138)
            details = until(lambda: window("XDM.*Download details$"))
            click(details, 320, 450)
            until(lambda: window("XDM.*Download details$") is None)
            click(main_window, 203, 138)
            redownload = until(lambda: window("XDM.*Redownload$"))
            click(redownload, 160, 228)
            rows = until(lambda: cli("list")["tasks"] if len(cli("list")["tasks"]) == 2 else None)
            clone = next(row for row in rows if row["id"] != source)
            assert clone["output"] == str(base / "file-again.bin") and clone["status"] == "queued"
            click(main_window, 120, row_y(clone["id"]))
            click(main_window, 375, 138)
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)
            until(lambda: len(cli("list")["tasks"]) == 1)
            assert cli("list")["tasks"][0]["id"] == source
            urls = base / "urls.txt"
            urls.write_text("http://example.invalid/b.bin\nhttp://example.invalid/c.bin\n")
            click(main_window, 342, 196)
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
            selected = cli("list")["tasks"][0]["id"]
            click(main_window, 120, row_y(selected))
            click(main_window, 740, 30)
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

            def move_task(btn_x, btn_y, target_pos):
                def get_pos():
                    tasks = cli("list")["tasks"]
                    t = next((t for t in tasks if t["id"] == second), None)
                    return t["position"] if t else None
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    if gui and gui.poll() is not None:
                        raise AssertionError(f"xdm-gui exited prematurely with code {gui.returncode}")
                    if daemon and daemon.poll() is not None:
                        raise AssertionError(f"xdm-daemon exited prematurely with code {daemon.returncode}")
                    if get_pos() == target_pos:
                        return
                    cur_pos = get_pos()
                    row_y_now = 457 if cur_pos == 1 else 497  # queue-filtered list, ordered by position
                    click(main_window, 835, 76)  # Refresh button (toolbar row 2 since the Add media button)
                    click(main_window, 120, row_y_now)  # Select second task's current row
                    click(main_window, btn_x, btn_y)  # Click move button
                    step_end = time.monotonic() + 2.0
                    while time.monotonic() < step_end:
                        if get_pos() == target_pos:
                            return
                        time.sleep(0.1)
                raise AssertionError(f"Task move to position {target_pos} timed out after 60s")

            move_task(668, 373, 1)  # Up
            move_task(746, 373, 2)  # Down
            move_task(596, 373, 1)  # Top
            move_task(840, 373, 2)  # Bottom

            # Exercise Batch download dialog
            click(main_window, 194, 76)  # "Batch…" button in row 2
            batch_win = until(lambda: window("XDM.*Add batch download$"))
            type_field(batch_win, 300, 95, "http://example.invalid/batch[01-03].bin")
            type_field(batch_win, 300, 265, base / "batch_out")
            click(batch_win, 160, 408)  # "Add batch" button
            until(lambda: window("XDM.*Add batch download$") is None)
            until(lambda: len([t for t in cli("list")["tasks"] if "batch" in t["url"]]) == 3)

            # Exercise Categories editor dialog
            click(main_window, 398, 76)  # "Categories…" button in row 2
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
            click(main_window, 532, 76)  # "Connection…" button in row 2
            conn_win = until(lambda: window("XDM.*Connection settings$"))
            click(conn_win, 390, 105)   # "Direct (No Proxy)" button
            type_field(conn_win, 500, 368, "4")  # Default connections field
            type_field(conn_win, 500, 426, "45")  # HTTP stall timeout field
            click(conn_win, 130, 568)  # "Save settings" button
            until(lambda: cli("proxy", "get")["proxy"] == {"mode": "direct"})
            until(lambda: cli("stall-timeout", "get").get("stall_timeout_seconds") == 45)
            gui_settings_file = base / "state" / "gui-settings.json"
            until(lambda: gui_settings_file.exists() and json.loads(gui_settings_file.read_text()).get("default_connections") == 4)
            click(conn_win, 580, 568)  # "Close" button
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
            click(main_window, 508, 196)  # "Add media…" button in toolbar
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
            click(main_window, 120, 497)
            time.sleep(0.5)
            # Click row 1 to select standard task
            click(main_window, 120, 457)
            time.sleep(0.5)

            # Exercise Countdown banner and Cancel button in main window
            banner_task = cli("submit", f"http://127.0.0.1:{port}/complete.bin", base / "banner-test.bin")["task_id"]
            banner_queue = cli("queue", "create", "BannerQueue", 1)["queue_id"]
            cli("queue", "completion", banner_queue, "shutdown", "--confirm-destructive", "--countdown", 120)
            cli("queue", "assign", banner_queue, banner_task)
            cli("queue", "start-one", banner_queue)

            # Daemon arms completion pending
            pending = until(lambda: [r for r in cli("completion", "status")["completion_pending"] if r["queue_id"] == banner_queue])
            assert pending[0]["action"] == "shutdown"
            assert 0 < pending[0]["seconds_remaining"] <= 120

            # Wait for GUI periodic poll (every 5s when idle) to catch pending action, then click Cancel
            cancelled = False
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline:
                if gui and gui.poll() is not None:
                    raise AssertionError(f"xdm-gui exited prematurely with code {gui.returncode}")
                if daemon and daemon.poll() is not None:
                    raise AssertionError(f"xdm-daemon exited prematurely with code {daemon.returncode}")
                time.sleep(0.8)
                click(main_window, 838, 38)
                if not [r for r in cli("completion", "status")["completion_pending"] if r["queue_id"] == banner_queue]:
                    cancelled = True
                    break
            assert cancelled, "Completion was not cancelled by banner button click"
            events = cli("queue", "events", banner_queue)["queue_events"]
            assert any(e.get("action_status") == "cancelled" for e in events), f"Expected cancelled event, got {events}"


            # ---- Main-window parity phase: sorting, selection, keyboard, columns, language ----
            for task in cli("list")["tasks"]:
                cli("remove", task["id"])
            until(lambda: not cli("list")["tasks"])
            stray = window("XDM.*Queues$")
            if stray:
                click(stray, 390, 622)  # Close any Queues dialog left open by the banner polling above
                until(lambda: window("XDM.*Queues$") is None)
            click(main_window, 435, 30)  # "All queues" clears the queue filter
            zeta = cli("submit", "http://example.invalid/zeta.bin", base / "zeta.bin")["task_id"]
            alpha = cli("submit", "http://example.invalid/alpha.bin", base / "alpha.bin")["task_id"]
            mid = cli("submit", "http://example.invalid/mid.bin", base / "mid.bin")["task_id"]
            click(main_window, 833, 76)  # Refresh
            time.sleep(0.5)

            # Sorting: header click sorts ascending by file name, second click flips to descending.
            click(main_window, 70, 413)  # "File name" header
            time.sleep(0.5)
            click(main_window, 120, 457)  # first visible row
            click(main_window, 642, 80)   # Pause
            until(lambda: statuses()[alpha] == "paused")
            assert statuses()[zeta] == "queued" and statuses()[mid] == "queued"
            click(main_window, 70, 413)  # descending
            time.sleep(0.5)
            click(main_window, 120, 457)
            click(main_window, 642, 80)
            until(lambda: statuses()[zeta] == "paused")
            assert statuses()[mid] == "queued"

            # Multi-select with Ctrl+A, then pause everything and resume via Ctrl/Shift click.
            click(main_window, 120, 497)
            key("ctrl+a")
            click(main_window, 642, 80)
            until(lambda: set(statuses().values()) == {"paused"})
            click(main_window, 120, 457)
            click_mod(main_window, 120, 537, "shift")  # shift-range: rows 1..3
            click(main_window, 735, 80)  # Resume
            until(lambda: "paused" not in statuses().values())
            click(main_window, 120, 457)
            click_mod(main_window, 120, 537, "ctrl")  # ctrl-toggle: rows 1 and 3 only
            click(main_window, 642, 80)
            until(lambda: sum(1 for v in statuses().values() if v == "paused") == 2)
            ordered = sorted(cli("list")["tasks"], key=lambda t: t["output"], reverse=True)  # GUI order: desc by name
            assert [t["status"] for t in ordered] == ["paused", "queued", "paused"], ordered

            # Keyboard: Enter opens properties, arrows move the selection, Delete asks for confirmation.
            click(main_window, 120, 457)
            key("Return")
            details = until(lambda: window("XDM.*Download details$"))
            click(details, 320, 450)
            until(lambda: window("XDM.*Download details$") is None)
            click(main_window, 120, 457)
            key("Down")
            key("Delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 330, 188)  # Cancel keeps the task
            until(lambda: window("XDM.*Remove task$") is None)
            assert len(cli("list")["tasks"]) == 3
            click(main_window, 120, 497)  # refocus the list (selection is still the arrow-selected row)
            key("Delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)  # confirm
            until(lambda: len(cli("list")["tasks"]) == 2)
            assert mid not in {t["id"] for t in cli("list")["tasks"]}, "arrow-selected row (mid) should be removed"
            click(main_window, 120, 457)
            key("ctrl+a")
            key("Delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)
            until(lambda: not cli("list")["tasks"])

            # Open / Open folder go through the (faked) platform opener without a shell.
            done = base / "opened.bin"
            opened = cli("submit", "http://example.invalid/opened.bin", done)["task_id"]
            done.write_bytes(b"x")
            db = sqlite3.connect(base / "state" / "tasks.sqlite3")
            db.execute("UPDATE tasks SET status='complete' WHERE id=?", (opened,))
            db.commit()
            db.close()
            click(main_window, 833, 76)
            time.sleep(0.5)
            click(main_window, 120, 457)
            click(main_window, 516, 138)  # Open
            click(main_window, 634, 138)  # Open folder
            open_log = base / "state" / "gui-open.log"
            until(lambda: open_log.exists() and len(open_log.read_text().splitlines()) >= 2)
            lines = open_log.read_text().splitlines()
            assert lines[0].endswith(str(done)) and lines[1].endswith(str(base)), lines

            # Move / Rename of the completed download.
            moved = base / "moved" / "renamed.bin"
            click(main_window, 793, 138)  # Move / Rename...
            mover = until(lambda: window("XDM.*Move / Rename completed download$"))
            type_field(mover, 300, 95, moved)
            click(mover, 150, 190)
            time.sleep(1.0)  # the daemon refuses a missing destination directory and the dialog stays open
            assert window("XDM.*Move / Rename completed download$") and done.exists() and not moved.exists()
            moved.parent.mkdir()
            click(mover, 150, 190)
            until(moved.exists)
            assert not done.exists()
            # The daemon moved the file and its task record follows it.
            until(lambda: cli("info", opened)["tasks"][0]["output"] == str(moved))
            assert next(t for t in cli("list")["tasks"] if t["id"] == opened)["output"] == str(moved)
            # An existing destination is refused by the daemon: nothing moves and the dialog stays open.
            taken = base / "moved" / "taken.bin"
            taken.write_bytes(b"keep me")
            click(main_window, 120, 457)
            click(main_window, 793, 138)
            mover = until(lambda: window("XDM.*Move / Rename completed download$"))
            type_field(mover, 300, 95, taken)
            click(mover, 150, 190)
            time.sleep(1.0)
            assert window("XDM.*Move / Rename completed download$"), "daemon error must keep the dialog open"
            assert moved.exists() and taken.read_bytes() == b"keep me"
            assert cli("info", opened)["tasks"][0]["output"] == str(moved)
            click(mover, 450, 190)  # Cancel
            until(lambda: window("XDM.*Move / Rename completed download$") is None)

            # Column visibility persists to gui-settings.json.
            before = grab("cols-before", (0, 395, 900, 560))
            click(main_window, 704, 254)  # Columns...
            columns = until(lambda: window("XDM.*Customize columns$"))
            click(columns, 25, 148)  # uncheck Size
            click(columns, 25, 304)  # uncheck Category
            click(columns, 104, 405)  # Save
            until(lambda: json.loads(gui_settings_file.read_text())["columns"]["size"] is False)
            saved = json.loads(gui_settings_file.read_text())["columns"]
            assert saved["category"] is False and saved["filename"] is True, saved
            time.sleep(0.5)
            assert grab("cols-after", (0, 395, 900, 560)) != before, "hiding columns must change the table"

            # Language selection in Preferences persists and re-renders the main window.
            english = grab("lang-en", (0, 0, 900, 200))
            click(main_window, 820, 254)
            pref_win = until(lambda: window("XDM.*Preferences$"))
            subprocess.run(["xdotool", "mousemove", "--window", pref_win, "300", "300", "click", "--repeat", "20",
                            "--delay", "50", "5"], env=env, check=True, timeout=30)
            time.sleep(0.5)
            click(pref_win, 580, 420)  # Urdu
            click(pref_win, 210, 560)  # Save preferences
            until(lambda: json.loads(gui_settings_file.read_text()).get("language") == "ur")
            click(pref_win, 550, 560)  # Close
            until(lambda: window("XDM.*Preferences$") is None)
            time.sleep(0.5)
            urdu = grab("lang-ur", (0, 0, 900, 200))
            assert urdu != english, "Urdu catalog must change the toolbar labels"
            header = grab("header-before-restart", (222, 395, 900, 430))

            # Restart: columns and language are restored from settings.
            gui.terminate()
            gui.wait(timeout=15)
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=env, stderr=gui_log, text=True)
            main_window = until(lambda: window("XDM.*Downloads$"))
            time.sleep(1.5)
            assert grab("restart-ur", (0, 0, 900, 200)) == urdu, "language must persist across restart"
            assert grab("restart-header", (222, 395, 900, 430)) == header, "hidden columns must persist across restart"

            # ---- Daemon-wired dialogs: remove with partial, cleanup, bandwidth, sync queue, restart ----
            for t in cli("list")["tasks"]:
                cli("remove", t["id"])
            until(lambda: not cli("list")["tasks"])
            click(main_window, 833, 76)  # Refresh
            time.sleep(0.5)

            # Remove dialog: "Also delete partial file" is offered for unfinished tasks and maps to TaskRemoveEx.
            keep = base / "keepdir"
            keep.mkdir()
            kept_task = cli("submit", "http://example.invalid/kept.bin", keep / "kept.bin")["task_id"]
            gone_task = cli("submit", "http://example.invalid/gone.bin", keep / "gone.bin")["task_id"]
            for name in ("kept.bin", "gone.bin"):
                (keep / f"{name}.part").write_bytes(b"p" * 50)
                (keep / f"{name}.part.meta").write_text(CHECKPOINT)
            click(main_window, 833, 76)
            time.sleep(0.8)
            click(main_window, 120, row_y(kept_task))
            click(main_window, 375, 138)
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 206)  # unticked: only the list entry goes
            until(lambda: len(cli("list")["tasks"]) == 1)
            assert (keep / "kept.bin.part").exists() and (keep / "kept.bin.part.meta").exists()
            click(main_window, 833, 76)
            time.sleep(0.8)
            click(main_window, 120, 457)
            click(main_window, 375, 138)
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 170, 141)  # "Also delete partial file"
            click(remove, 120, 206)
            until(lambda: not cli("list")["tasks"])
            assert not (keep / "gone.bin.part").exists() and not (keep / "gone.bin.part.meta").exists(), \
                "the ticked box must delete the task's own partial files"
            assert (keep / "kept.bin.part").exists()

            # Clean up partial files: scan, multi-select, confirm; only the selected XDM pair is deleted.
            work = base / "partials"
            work.mkdir()
            (work / "ghost.bin.part").write_bytes(b"g" * 100)
            (work / "ghost.bin.part.meta").write_text(CHECKPOINT)
            (work / "g2.bin.part.meta").write_text(CHECKPOINT)
            (work / "foo.zip.part").write_bytes(b"browser partial")
            (work / "other.bin.part").write_bytes(b"x")
            (work / "other.bin.part.meta").write_text('{"hello": 1}')
            click(main_window, 805, 196)  # More controls
            click(main_window, 546, 258)  # Clean up partial files...
            cleanup = until(lambda: window("XDM.*Clean up partial files$"))
            type_field(cleanup, 370, 67, work)
            click(cleanup, 709, 67)  # Scan
            time.sleep(1.5)
            click(cleanup, 200, 144)  # second row: ghost.bin (g2.bin sorts first)
            click(cleanup, 605, 458)  # Delete selected...
            time.sleep(0.4)
            assert (work / "ghost.bin.part").exists(), "deletion needs the confirmation click"
            click(cleanup, 579, 458)  # Confirm delete
            until(lambda: not (work / "ghost.bin.part").exists())
            assert not (work / "ghost.bin.part.meta").exists()
            leftovers = sorted(f.name for f in work.iterdir())
            assert leftovers == ["foo.zip.part", "g2.bin.part.meta", "other.bin.part", "other.bin.part.meta"], leftovers
            click(cleanup, 380, 530)  # Close
            until(lambda: window("XDM.*Clean up partial files$") is None)
            click(main_window, 805, 196)  # Hide controls

            # Bandwidth settings (reached from Connection settings): global limit, quota and host limit
            # are applied through the daemon and read back with BandwidthStatus.
            click(main_window, 532, 76)
            conn_win = until(lambda: window("XDM.*Connection settings$"))
            click(conn_win, 350, 484)  # Bandwidth limits and quota...
            bandwidth = until(lambda: window("XDM.*Bandwidth$"))
            time.sleep(1.0)
            type_field(bandwidth, 280, 113, "512")
            click(bandwidth, 570, 113)  # Apply global limit
            until(lambda: cli("bandwidth")["bandwidth"]["global_kib"] == 512)
            type_field(bandwidth, 158, 475, "100")
            type_field(bandwidth, 443, 475, "2")
            click(bandwidth, 603, 475)  # Apply quota
            until(lambda: cli("bandwidth")["bandwidth"]["quota_mib"] == 100)
            assert cli("bandwidth")["bandwidth"]["quota_window_seconds"] == 7200
            type_field(bandwidth, 148, 245, "*.example.test")
            type_field(bandwidth, 407, 245, "64")
            click(bandwidth, 581, 245)  # Set limit
            until(lambda: ["*.example.test", 64] in cli("bandwidth")["bandwidth"]["host_limits"])
            click(bandwidth, 564, 717)  # Close
            until(lambda: window("XDM.*Bandwidth$") is None)
            click(conn_win, 580, 568)
            until(lambda: window("XDM.*Connection settings$") is None)
            cli("limit-global", "unlimited")
            cli("quota", "off")

            # Synchronization queue: create, set interval, check now.
            click(main_window, 740, 30)
            queues = until(lambda: window("XDM.*Queues$"))
            type_field(queues, 160, 286, "Mirror")
            type_field(queues, 480, 286, "1")
            click(queues, 90, 550)  # Create sync queue
            mirror = until(lambda: next((q for q in cli("list")["queues"] if q["name"] == "Mirror"), None))
            assert mirror["sync"] is True and mirror["sync_interval"] is None, mirror
            type_field(queues, 360, 550, "120")
            click(queues, 510, 550)  # Set interval
            until(lambda: next(q for q in cli("list")["queues"] if q["id"] == mirror["id"])["sync_interval"] == 120)
            click(queues, 713, 550)  # Check now
            until(lambda: any(e["outcome"] == "sync_checked"
                              for e in cli("queue", "events", mirror["id"])["queue_events"]))
            click(queues, 611, 550)  # Manual only
            until(lambda: next(q for q in cli("list")["queues"] if q["id"] == mirror["id"])["sync_interval"] is None)
            click(queues, 390, 622)
            until(lambda: window("XDM.*Queues$") is None)

            # A real quota: a 2 MiB download exceeds a 1 MiB quota and parks in quota_wait.
            cli("quota", 1, 1)
            probe = cli("submit", f"http://127.0.0.1:{port}/big.bin", base / "big.bin")["task_id"]
            click(main_window, 833, 76)
            time.sleep(0.8)
            assert statuses()[probe] == "queued"
            queued_status = grab("status-queued", (130, 437, 330, 477))
            cli("queue", "start")
            until(lambda: statuses().get(probe) == "quota_wait", timeout=60)
            cli("queue", "stop")
            until(lambda: cli("list")["queue_running"] is False)
            assert cli("bandwidth")["bandwidth"]["quota_exceeded"] is True
            click(main_window, 833, 76)
            time.sleep(0.8)
            quota_status = grab("status-quota", (130, 437, 330, 477))
            assert quota_status != queued_status, "quota_wait must be shown in the Status column"
            # restart_required is shown too, and the Restart row action requeues it via TaskRestart.
            db = sqlite3.connect(base / "state" / "tasks.sqlite3")
            db.execute("UPDATE tasks SET status='restart_required', restart_reason='remote changed' WHERE id=?", (probe,))
            db.commit()
            db.close()
            click(main_window, 833, 76)
            time.sleep(0.8)
            restart_status = grab("status-restart", (130, 437, 330, 477))
            assert restart_status not in (queued_status, quota_status), "restart_required must be shown"
            click(main_window, 120, 457)
            rclick(main_window, 120, 457)
            click(main_window, 205, 613)  # context menu: Restart
            until(lambda: statuses().get(probe) == "queued")
            cli("quota", "off")

            # ---- Paged table with 20,000 tasks (ListPage; sort, filter and search run in the daemon) ----
            for t in cli("list")["tasks"]:
                cli("remove", t["id"])
            gui.terminate()
            gui.wait(timeout=15)
            gui = None
            for chunk in range(40):  # an import is capped at 500 entries per call
                cli("batch", f"http://example.invalid/g{chunk:02d}_[0001-0500].bin", base / f"bulk{chunk:02d}",
                    "--allow-duplicates")
            until(lambda: cli("list", "--limit", 1)["total"] == 20000, timeout=300)
            # Show every sortable column in English so header positions are known.
            saved_settings = json.loads(gui_settings_file.read_text())
            saved_settings["language"] = "en"
            saved_settings["columns"]["size"] = True
            gui_settings_file.write_text(json.dumps(saved_settings))
            perf_env = {**env, "XDM_GUI_PERF": "1"}
            gui_log.flush()
            gui_log.seek(0)
            gui_log.truncate()
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=perf_env, stderr=gui_log, text=True)
            import re
            perf_log = base / "gui.log"

            def perf_lines(kind):
                gui_log.flush()
                return re.findall(rf"\[xdm-gui\] {kind} (.*)", perf_log.read_text())

            def has_total(total):
                return any(f"total={total} " in line for line in perf_lines("refresh"))

            until(lambda: has_total(20000), timeout=60)
            first = next(line for line in perf_lines("refresh") if "total=20000 " in line)
            assert "rows=200 " in first, f"only one page of rows is fetched for 20000 tasks: {first}"
            main_window = until(lambda: window("XDM"))
            time.sleep(1.0)
            # Scroll far down with the scrollbar: the rows there arrive as page fetches.
            subprocess.run(["xdotool", "mousemove", "--window", main_window, "881", "450", "mousedown", "1",
                            "mousemove_relative", "--", "0", "280", "mouseup", "1"], env=env, check=True, timeout=30)
            time.sleep(2.0)
            fetched = perf_lines("page-fetch")
            deep = [line for line in fetched if int(re.search(r"page=(\d+)", line).group(1)) >= 50]
            assert deep, f"scrolling to the end must fetch deep pages; got {fetched}"
            # Search is pushed to the daemon: 500 rows match one import chunk.
            type_field(main_window, 270, 254, "g07_")
            click(main_window, 556, 254)  # Find
            until(lambda: has_total(500), timeout=30)
            # Sorting by a column is a daemon sort: the refresh after the header click still reports the filtered total.
            click(main_window, 70, 413)
            time.sleep(2.5)
            assert not [line for line in perf_lines("refresh") if "total=0 " in line]
            assert [line for line in perf_lines("refresh") if "total=500 sort=output " in line], \
                "the File name header must sort in the daemon (sort=output) while the filter holds"

            # Header sorts for columns without a stored column now use the daemon's keys.
            def sorted_by(key, header_x):
                before = len(perf_lines("refresh"))
                click(main_window, header_x, 413)
                until(lambda: any(f"sort={key} " in line for line in perf_lines("refresh")[before:]), timeout=30)
            sorted_by("size", 200)
            sorted_by("-size", 200)
            sorted_by("progress", 403)
            sorted_by("rate", 488)
            sorted_by("queue", 836)
            sorted_by("output", 70)  # back to File name

            # Ctrl+A selects every task matching the filter, beyond the loaded rows.
            click(main_window, 620, 254)  # Clear the search
            until(lambda: "total=20000 " in (perf_lines("refresh") or [""])[-1], timeout=30)
            click(main_window, 120, 457)
            key("ctrl+a")
            time.sleep(0.5)
            gui_log.flush()
            started = time.monotonic()
            click(main_window, 735, 80)  # Resume -> TaskBulk resume with the filter and expected total
            until(lambda: cli("list", "--limit", 1, "--status", "queued")["total"] == 20000, timeout=300)
            click(main_window, 642, 80)  # Pause
            until(lambda: cli("list", "--limit", 1, "--status", "paused")["total"] == 20000, timeout=300)
            assert cli("list", "--limit", 1, "--status", "queued")["total"] == 0
            print(f"select-all resume+pause of 20000 tasks through the GUI: {time.monotonic() - started:.1f}s")

            # Queues dialog: daemon calls run off the UI thread and still work, including bulk assign.
            click(main_window, 739, 30)
            queues_win = until(lambda: window("XDM.*Queues$"))
            type_field(queues_win, 160, 286, "Bulk")
            type_field(queues_win, 480, 286, "2")
            click(queues_win, 82, 342)  # Create
            bulk_queue = until(lambda: next((q for q in cli("list")["queues"] if q["name"] == "Bulk"), None))
            started = time.monotonic()
            click(queues_win, 390, 446)  # Assign: all 20000 matching tasks
            until(lambda: cli("list", "--limit", 1, "--queue", bulk_queue["id"])["total"] == 20000, timeout=600)
            print(f"select-all assign of 20000 tasks to a queue through the GUI: {time.monotonic() - started:.1f}s")
            assert gui.poll() is None
            samples = []
            for line in perf_lines("refresh") + perf_lines("page-fetch"):
                for key in ("ui_thread_total", "fetch_to_install", "fetch_to_render"):
                    m = re.search(rf"{key}=([0-9.]+)(ms|s|\u00b5s)", line)
                    if m:
                        samples.append((key, float(m.group(1)) * {"ms": 1.0, "s": 1000.0, "\u00b5s": 0.001}[m.group(2)]))
            assert samples, "no page timing in the GUI log"
            worst = max(value for _, value in samples)
            renders = re.findall(r"render=([0-9.]+)(ms|s|\u00b5s) fetch_to_render", perf_log.read_text())
            print(f"20000-task table: first refresh '{first}'")
            print(f"page timings (ms): {[(k, round(v, 1)) for k, v in samples][:12]} worst={worst:.1f}ms; "
                  f"software render samples: {renders[:6]}")
            assert worst < 1500, f"page fetch/refresh too slow: {worst} ms"
            assert gui.poll() is None
            print("isolated GUI task actions, batch dialog, categories editor, connection settings, preferences dialog, add media dialog, and completion actions passed")
        finally:
            if os.environ.get("GUI_ACTIONS_SHOT"):  # debugging aid: screenshot of the isolated display
                subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size", "1200x900",
                                "-i", env["DISPLAY"], "-frames:v", "1", os.environ["GUI_ACTIONS_SHOT"]], env=env)
            log_path = base / "gui.log"
            if log_path.exists():
                print("GUI LOG:\n" + log_path.read_text())
            for process in [gui, daemon, virtual]:
                if process and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


RANGE_SIZE = 8 * 1024 * 1024


class _RangeHandler(BaseHTTPRequestHandler):
    """Throttled server with byte-range support so downloads use several connections."""
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        start, end = 0, RANGE_SIZE - 1
        spec = self.headers.get("Range")
        if spec and spec.startswith("bytes="):
            first, _, last = spec[6:].partition("-")
            start = int(first)
            end = min(int(last), RANGE_SIZE - 1) if last else RANGE_SIZE - 1
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{RANGE_SIZE}")
        else:
            self.send_response(200)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"fixture"')
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        try:
            position = start
            while position <= end:
                chunk = min(16384, end - position + 1)
                self.wfile.write(bytes([position % 251]) * chunk)
                position += chunk
                time.sleep(0.1)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def log_message(self, *args):
        pass


def dialogs_main():
    """Add URL -> Download File Info -> progress window -> Download complete, on an isolated Xvfb."""
    shots = Path(os.environ.get("GUI_DIALOG_SHOTS", "")) if os.environ.get("GUI_DIALOG_SHOTS") else None
    with tempfile.TemporaryDirectory(prefix="xdm-gui-dialogs-") as directory:
        base = Path(directory)
        server = ThreadingHTTPServer(("127.0.0.1", 0), _RangeHandler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        port = server.server_address[1]
        virtual = subprocess.Popen(["Xvfb", "-displayfd", "1", "-nocursor", "-screen", "0", "1280x900x24"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        display = virtual.stdout.readline().strip()
        assert display.isdigit(), "Xvfb did not allocate a display"
        env = {**os.environ, "DISPLAY": f":{display}", "SLINT_BACKEND": "winit-software",
               "XDM_DATA_DIR": str(base / "state"), "XDM_FAKE_OS_ACTIONS": "1"}
        env.pop("WAYLAND_DISPLAY", None)
        daemon = gui = fixture = None
        fixture_log = base / "fixture.log"

        def cli(*args):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=60, check=True)
            return json.loads(result.stdout)

        def until(action, timeout=60):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if gui and gui.poll() is not None:
                    raise AssertionError(f"xdm-gui exited prematurely with code {gui.returncode}")
                value = action()
                if value:
                    return value
                time.sleep(0.1)
            raise AssertionError(f"GUI condition timed out after {timeout}s")

        def window(name):
            result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], env=env,
                                    capture_output=True, text=True, timeout=30)
            return result.stdout.splitlines()[-1] if result.returncode == 0 and result.stdout.strip() else None

        def title(identifier):
            return subprocess.run(["xdotool", "getwindowname", identifier], env=env, capture_output=True,
                                  text=True, timeout=30).stdout.strip()

        def click(identifier, x, y):
            subprocess.run(["xdotool", "windowfocus", identifier], env=env, check=False, timeout=30)
            subprocess.run(["xdotool", "mousemove", "--window", identifier, str(x), str(y), "click", "1"],
                           env=env, check=True, timeout=30)
            time.sleep(0.25)

        def type_text(value):
            subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+a"], env=env, check=True, timeout=30)
            subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "0", str(value)], env=env, check=True, timeout=30)

        def shot(name):
            if shots:
                shots.mkdir(parents=True, exist_ok=True)
                subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1280x900",
                                "-i", env["DISPLAY"], "-frames:v", "1", str(shots / f"{name}.png")], env=env, check=True, timeout=30)

        def tasks():
            return cli("list")["tasks"]

        def add_url(url):
            main = until(lambda: window("XDM.*Downloads$"))
            add = None
            for _ in range(4):
                subprocess.run(["xdotool", "mousemove", "--window", main, "300", "500"], env=env, check=True, timeout=30)
                time.sleep(0.3)
                click(main, ADD_URL_BUTTON[0], ADD_URL_BUTTON[1])
                try:
                    add = until(lambda: window("XDM.*Add URL$"), timeout=3)
                    break
                except AssertionError:
                    continue
            if not add:
                listing = subprocess.run("for w in $(xdotool search --onlyvisible --name ''); do echo $w $(xdotool getwindowname $w) $(xdotool getwindowgeometry $w | tr '\\n' ' '); done",
                                         shell=True, env=env, capture_output=True, text=True).stdout
                print("VISIBLE WINDOWS:\n" + listing)
                print(subprocess.run(f"ps -L -o tid,stat,pcpu,wchan:20,comm -p {gui.pid}; which gdb", shell=True, capture_output=True, text=True).stdout)
            assert add, "Add URL dialog did not open"
            click(add, 340, 79)
            type_text(url)
            return add

        try:
            daemon = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env, stderr=subprocess.PIPE, text=True)
            assert "listening locally" in daemon.stderr.readline()
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env=env, stderr=open(base / "gui.log", "w"), text=True)
            until(lambda: window("XDM.*Downloads$"))
            time.sleep(1)
            url = f"http://127.0.0.1:{port}/fixture.bin"
            shot("main")

            # 1. Add URL -> File Info (size probed over HTTP) -> Start Download.
            add = add_url(url)
            shot("add-url")
            click(add, 178, 237)  # OK
            info = until(lambda: window("Download File Info$"))
            time.sleep(1)  # size probe
            shot("file-info")
            click(info, 278, 190)  # Start Download

            # 2. Progress window: percent advances, connections table shows several segments.
            progress = until(lambda: window(r"^[0-9.]+% fixture\.bin$"))
            first = float(title(progress).split("%")[0])
            until(lambda: float(title(progress).split("%")[0]) > first + 1, timeout=60)
            time.sleep(1)
            shot("progress-details")
            task_id = tasks()[0]["id"]
            # The segment bar fills per connection from bytes received, not only flushed buffers.
            from PIL import Image
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1280x900",
                            "-i", env["DISPLAY"], "-frames:v", "1", str(base / "bar.png")], env=env, check=True, timeout=30)
            bar = Image.open(base / "bar.png").convert("RGB").crop((10, 252, 522, 262))
            filled = sum(1 for pixel in bar.getdata() if abs(pixel[0] - 0x3f) < 12 and abs(pixel[2] - 0xd8) < 12)
            assert filled > 100, f"segment bar shows no per-connection fill ({filled} px)"

            # Speed limiter tab: per-task cap reaches the daemon.
            click(progress, 190, 19)
            shot("progress-limiter")
            click(progress, 22, 71)  # Use Speed Limiter
            click(progress, 50, 120)
            type_text("900")
            subprocess.run(["xdotool", "key", "Return"], env=env, check=True, timeout=30)
            until(lambda: cli("info", task_id)["tasks"][0].get("rate_limit") == 900 * 1024)
            click(progress, 22, 71)  # unlimit again
            until(lambda: cli("info", task_id)["tasks"][0].get("rate_limit") in (None, 0))
            click(progress, 330, 19)
            shot("progress-options")
            click(progress, 75, 19)  # back to the status tab

            # Pause / Resume.
            click(progress, 347, 217)
            until(lambda: cli("info", task_id)["tasks"][0]["status"] == "paused")
            time.sleep(0.6)
            shot("progress-paused")
            paused_title = title(progress)
            click(progress, 347, 217)
            until(lambda: cli("info", task_id)["tasks"][0]["status"] in ("queued", "running"))
            until(lambda: title(progress) != paused_title)

            # Hide details shrinks the window.
            click(progress, 92, 217)
            time.sleep(0.5)
            shot("progress-hidden")
            click(progress, 92, 217)

            # 3. Completion: Download complete dialog, Open folder through the faked opener.
            done = until(lambda: window("^Download complete$"), timeout=120)
            time.sleep(0.5)
            shot("complete")
            assert cli("info", task_id)["tasks"][0]["status"] == "complete"
            click(done, 55, 167)  # Open
            open_log = base / "state" / "gui-open.log"
            until(lambda: open_log.exists() and "fixture.bin" in open_log.read_text())
            click(done, 263, 167)  # Open folder
            until(lambda: len(open_log.read_text().splitlines()) >= 2)
            assert open_log.read_text().splitlines()[1].endswith("downloads"), open_log.read_text()
            click(done, 30, 197)  # Don't show this dialog again
            until(lambda: json.loads((base / "state" / "gui-settings.json").read_text()).get("show_complete_dialog") is False)
            click(done, 397, 167)  # Close
            until(lambda: window("^Download complete$") is None)

            # 4. Download Later adds a paused task; the same URL again raises the duplicate prompt.
            add = add_url(f"http://127.0.0.1:{port}/later.bin")
            click(add, 178, 237)
            info = until(lambda: window("Download File Info$"))
            click(info, 151, 190)  # Download Later
            until(lambda: any(t["url"].endswith("later.bin") and t["status"] == "paused" for t in tasks()))
            add = add_url(f"http://127.0.0.1:{port}/later.bin")
            click(add, 178, 237)
            info = until(lambda: window("Download File Info$"))
            click(info, 151, 190)
            time.sleep(1)
            shot("duplicate")
            click(info, 128, 150)  # Add numbered copy (overlay button)
            until(lambda: len([t for t in tasks() if t["url"].endswith("later.bin")]) == 2)
            copy = max((t for t in tasks() if t["url"].endswith("later.bin")), key=lambda t: t["id"])
            assert copy["output"].endswith("later (2).bin"), copy["output"]

            # 5. A one-shot link survives File Info: the size probe is a HEAD, never a GET.
            fixture = subprocess.Popen([sys.executable, str(ROOT / "tests/integration/http_fixture.py"), str(fixture_log)],
                                       stdout=subprocess.PIPE, text=True)
            fixture_port = json.loads(fixture.stdout.readline())["port"]
            add = add_url(f"http://127.0.0.1:{fixture_port}/oneshot.bin")
            click(add, 178, 237)
            info = until(lambda: window("Download File Info$"))
            until(lambda: fixture_log.exists() and "HEAD" in fixture_log.read_text())
            time.sleep(0.5)
            logged = [json.loads(line) for line in fixture_log.read_text().splitlines()]
            assert not [r for r in logged if r["method"] == "GET"], logged
            click(info, 278, 190)  # Start Download
            oneshot = until(lambda: next((t for t in tasks() if t["url"].endswith("oneshot.bin") and t["status"] == "complete"), None), timeout=90)
            assert Path(oneshot["output"]).stat().st_size == 4 * 1024 * 1024
            logged = [json.loads(line) for line in fixture_log.read_text().splitlines()]
            assert sum(1 for r in logged if r["method"] == "GET" and r["attempt"] == 1) == 1
            assert gui.poll() is None
            print("download dialogs: Add URL, File Info, progress window with connections, speed limiter, pause/resume, complete dialog, duplicate handling passed")
        finally:
            try:
                shot("final")
            except Exception:
                pass
            log_path = base / "gui.log"
            if log_path.exists():
                print("GUI LOG:\n" + log_path.read_text())
            for process in [gui, daemon, fixture, virtual]:
                if process and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


ADD_URL_BUTTON = (72, 80)


if __name__ == "__main__":
    if os.environ.get("GUI_ACTIONS_ONLY") != "dialogs":
        main()
    dialogs_main()
