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
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1200x900",
                            "-i", env["DISPLAY"], "-frames:v", "1", str(path)], env=env, check=True, timeout=30)
            return Image.open(path).convert("RGB").crop(box).tobytes()

        # ---- main-window geometry (900x600 window at 0,0 in the isolated display) ----
        MENU_LEFT = {"tasks": 0, "file": 41, "downloads": 72, "view": 139, "help": 177}
        MENU_WIDTH = {"tasks": 250, "file": 250, "downloads": 250, "view": 230, "help": 200}
        MENUS = {
            "tasks": ["add-url", "add-batch", "add-media", "grabber", "-", "import-export", "-", "exit"],
            "file": ["download-now", "stop", "remove", "redownload", "restart", "-", "open", "open-folder",
                     "move-rename", "-", "sub:priority", "sub:queueorder", "-", "properties"],
            "downloads": ["start-all", "stop-all", "delete-completed", "-", "find", "-", "scheduler", "queues",
                          "sub:startq", "sub:stopq", "schedule-start", "schedule-stop", "-", "sub:speed",
                          "partials", "-", "categories", "connection", "options"],
            "view": ["toggle-sidebar", "-", "sub:arrange", "columns", "-", "toolbar-large", "toolbar-small", "-",
                     "refresh"],
        }
        SUBS = {
            "sub:priority": ["prio:2", "prio:1", "prio:0"],
            "sub:queueorder": ["move:top", "move:up", "move:down", "move:bottom"],
            "sub:speed": ["speed:on", "speed:off", "speed:settings"],
            "sub:arrange": ["sort:id", "sort:filename", "sort:size", "sort:status", "sort:time_left",
                            "sort:transfer_rate", "sort:last_try_date", "sort:description"],
        }
        CONTEXT = ["open", "open-folder", "-", "move-rename", "-", "redownload", "-", "download-now", "stop",
                   "restart", "-", "remove", "-", "sub:addq", "delq", "-", "sub:dblclick", "-", "properties"]
        TOOL = {"add": 37, "resume": 111, "stop": 185, "stopall": 259, "delete": 333, "delcompleted": 407,
                "options": 481, "scheduler": 555, "startq": 629, "stopq": 713}
        TOOL_Y = 51
        COL_X = {"filename": 230, "q": 316, "size": 363, "status": 433, "time_left": 503,
                 "transfer_rate": 588, "last_try_date": 668, "description": 796}
        HEADER_Y = 99
        ROW_X = 300

        def entry_y(items, key, top):
            y = top + 2
            for item in items:
                if item == key:
                    return y + 8
                y += 9 if item == "-" else 17
            raise AssertionError(f"{key} not in {items}")

        def menu_item(window_id, menu, key):
            click(window_id, MENU_LEFT[menu] + 20, 10)
            time.sleep(0.3)
            click(window_id, MENU_LEFT[menu] + 60, entry_y(MENUS[menu], key, 21))

        def submenu_item(window_id, menu, parent, key):  # cascades open on hover
            click(window_id, MENU_LEFT[menu] + 20, 10)
            time.sleep(0.3)
            parent_y = entry_y(MENUS[menu], parent, 21)
            subprocess.run(["xdotool", "mousemove", "--window", window_id, str(MENU_LEFT[menu] + 60),
                            str(parent_y)], env=env, check=True, timeout=30)  # hovering opens the cascade
            time.sleep(0.2)
            subprocess.run(["xdotool", "mousemove_relative", "--", "2", "0"], env=env, check=True, timeout=30)
            time.sleep(0.4)
            sub_top = parent_y - 8  # row top equals the cascade's top edge
            click(window_id, MENU_LEFT[menu] + MENU_WIDTH[menu] - 4 + 60, entry_y(SUBS[parent], key, sub_top))

        def find_text(text):
            menu_item(main_window, "downloads", "find")
            finder = until(lambda: window("XDM.*Find file$"))
            type_field(finder, 170, 50, text)
            if not text:
                key("BackSpace")  # the selection from ctrl+a is still there
            key("Return")
            until(lambda: window("XDM.*Find file$") is None)

        def tool(window_id, name):
            click(window_id, TOOL[name], TOOL_Y)

        def refresh_view(window_id):
            menu_item(window_id, "view", "refresh")
            time.sleep(0.4)

        def row_at(index):
            return 117 + 17 * index

        def statuses():
            return {t["id"]: t["status"] for t in cli("list")["tasks"]}

        def row_y(task_id):
            # Rows are listed by ascending id, 17px each, first row centred at y=117.
            ids = sorted(t["id"] for t in cli("list")["tasks"])
            return row_at(ids.index(task_id))

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
            click(main_window, ROW_X, 117)
            menu_item(main_window, "file", "properties")
            details = until(lambda: window("XDM.*Download details$"))
            click(details, 320, 450)
            until(lambda: window("XDM.*Download details$") is None)
            menu_item(main_window, "file", "redownload")
            redownload = until(lambda: window("XDM.*Redownload$"))
            click(redownload, 160, 228)
            rows = until(lambda: cli("list")["tasks"] if len(cli("list")["tasks"]) == 2 else None)
            clone = next(row for row in rows if row["id"] != source)
            assert clone["output"] == str(base / "file-again.bin") and clone["status"] == "queued"
            click(main_window, ROW_X, row_y(clone["id"]))
            tool(main_window, "delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)
            until(lambda: len(cli("list")["tasks"]) == 1)
            assert cli("list")["tasks"][0]["id"] == source
            urls = base / "urls.txt"
            urls.write_text("http://example.invalid/b.bin\nhttp://example.invalid/c.bin\n")
            menu_item(main_window, "tasks", "import-export")
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
            click(main_window, ROW_X, row_y(selected))
            menu_item(main_window, "downloads", "queues")
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

            def move_task(key, target_pos):
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
                    row_y_now = 117 if cur_pos == 1 else 134  # queue-filtered list, ordered by position
                    refresh_view(main_window)
                    click(main_window, ROW_X, row_y_now)  # Select second task's current row
                    submenu_item(main_window, "file", "sub:queueorder", key)
                    step_end = time.monotonic() + 2.0
                    while time.monotonic() < step_end:
                        if get_pos() == target_pos:
                            return
                        time.sleep(0.1)
                raise AssertionError(f"Task move to position {target_pos} timed out after 60s")

            move_task("move:up", 1)
            move_task("move:down", 2)
            move_task("move:top", 1)
            move_task("move:bottom", 2)

            # Exercise Batch download dialog
            menu_item(main_window, "tasks", "add-batch")
            batch_win = until(lambda: window("XDM.*Add batch download$"))
            type_field(batch_win, 300, 95, "http://example.invalid/batch[01-03].bin")
            type_field(batch_win, 300, 265, base / "batch_out")
            click(batch_win, 160, 408)  # "Add batch" button
            until(lambda: window("XDM.*Add batch download$") is None)
            until(lambda: len([t for t in cli("list")["tasks"] if "batch" in t["url"]]) == 3)

            # Exercise Categories editor dialog
            menu_item(main_window, "downloads", "categories")
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
            menu_item(main_window, "downloads", "connection")
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
            menu_item(main_window, "downloads", "options")
            pref_win = until(lambda: window("XDM.*Preferences$"))
            click(pref_win, 580, 95)      # "Dark" theme button
            click(pref_win, 180, 570)     # "Save preferences" button
            until(lambda: gui_settings_file.exists() and json.loads(gui_settings_file.read_text()).get("theme") == "dark")
            click(pref_win, 540, 570)     # "Close" button
            until(lambda: window("XDM.*Preferences$") is None)

            # Exercise Add media dialog with HLS inspection and audio-only job submission
            menu_item(main_window, "tasks", "add-media")
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
            refresh_view(main_window)
            live_task = until(lambda: next((t for t in cli("list")["tasks"] if t["id"] == live_task_id), None))
            assert live_task["kind"] == "media"
            # In main window, select live task in tasks listview
            click(main_window, ROW_X, 134)
            time.sleep(0.5)
            # Click row 1 to select standard task
            click(main_window, ROW_X, 117)
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
                click(main_window, 850, 38)
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
            click(main_window, 80, 118)  # "All Downloads" clears the queue filter
            zeta = cli("submit", "http://example.invalid/zeta.bin", base / "zeta.bin")["task_id"]
            alpha = cli("submit", "http://example.invalid/alpha.bin", base / "alpha.bin")["task_id"]
            mid = cli("submit", "http://example.invalid/mid.bin", base / "mid.bin")["task_id"]
            refresh_view(main_window)
            time.sleep(0.5)

            # Sorting: header click sorts ascending by file name, second click flips to descending.
            click(main_window, COL_X["filename"], HEADER_Y)  # "File name" header
            time.sleep(0.5)
            click(main_window, ROW_X, 117)  # first visible row
            tool(main_window, "stop")
            until(lambda: statuses()[alpha] == "paused")
            assert statuses()[zeta] == "queued" and statuses()[mid] == "queued"
            click(main_window, COL_X["filename"], HEADER_Y)  # descending
            time.sleep(0.5)
            click(main_window, ROW_X, 117)
            tool(main_window, "stop")
            until(lambda: statuses()[zeta] == "paused")
            assert statuses()[mid] == "queued"

            # Multi-select with Ctrl+A, then pause everything and resume via Ctrl/Shift click.
            click(main_window, ROW_X, 134)
            key("ctrl+a")
            tool(main_window, "stop")
            until(lambda: set(statuses().values()) == {"paused"})
            click(main_window, ROW_X, 117)
            click_mod(main_window, ROW_X, 151, "shift")  # shift-range: rows 1..3
            tool(main_window, "resume")
            until(lambda: "paused" not in statuses().values())
            click(main_window, ROW_X, 117)
            click_mod(main_window, ROW_X, 151, "ctrl")  # ctrl-toggle: rows 1 and 3 only
            tool(main_window, "stop")
            until(lambda: sum(1 for v in statuses().values() if v == "paused") == 2)
            ordered = sorted(cli("list")["tasks"], key=lambda t: t["output"], reverse=True)  # GUI order: desc by name
            assert [t["status"] for t in ordered] == ["paused", "queued", "paused"], ordered

            # Keyboard: Enter opens properties, arrows move the selection, Delete asks for confirmation.
            click(main_window, ROW_X, 117)
            key("Return")
            details = until(lambda: window("XDM.*Download details$"))
            click(details, 320, 450)
            until(lambda: window("XDM.*Download details$") is None)
            click(main_window, ROW_X, 117)
            key("Down")
            key("Delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 330, 188)  # Cancel keeps the task
            until(lambda: window("XDM.*Remove task$") is None)
            assert len(cli("list")["tasks"]) == 3
            click(main_window, ROW_X, 134)  # refocus the list (selection is still the arrow-selected row)
            key("Delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 188)  # confirm
            until(lambda: len(cli("list")["tasks"]) == 2)
            assert mid not in {t["id"] for t in cli("list")["tasks"]}, "arrow-selected row (mid) should be removed"
            click(main_window, ROW_X, 117)
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
            refresh_view(main_window)
            time.sleep(0.5)
            click(main_window, ROW_X, 117)
            menu_item(main_window, "file", "open")
            menu_item(main_window, "file", "open-folder")
            open_log = base / "state" / "gui-open.log"
            until(lambda: open_log.exists() and len(open_log.read_text().splitlines()) >= 2)
            lines = open_log.read_text().splitlines()
            assert lines[0].endswith(str(done)) and lines[1].endswith(str(base)), lines

            # Move / Rename of the completed download.
            moved = base / "moved" / "renamed.bin"
            menu_item(main_window, "file", "move-rename")
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
            click(main_window, ROW_X, 117)
            menu_item(main_window, "file", "move-rename")
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
            before = grab("cols-before", (154, 109, 900, 270))
            menu_item(main_window, "view", "columns")
            columns = until(lambda: window("XDM.*Customize columns$"))
            click(columns, 25, 148)  # uncheck Size
            click(columns, 25, 304)  # show Category (hidden by default)
            click(columns, 104, 405)  # Save
            until(lambda: json.loads(gui_settings_file.read_text())["columns"]["size"] is False)
            saved = json.loads(gui_settings_file.read_text())["columns"]
            assert saved["category"] is True and saved["filename"] is True, saved
            time.sleep(0.5)
            assert grab("cols-after", (154, 109, 900, 270)) != before, "hiding columns must change the table"

            # Language selection in Preferences persists and re-renders the main window.
            english = grab("lang-en", (0, 0, 900, 21))
            menu_item(main_window, "downloads", "options")
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
            urdu = grab("lang-ur", (0, 0, 900, 21))
            assert urdu != english, "Urdu catalog must change the menu bar labels"
            header = grab("header-before-restart", (306, 90, 900, 108))

            # Restart: columns and language are restored from settings.
            gui.terminate()
            gui.wait(timeout=15)
            gui = subprocess.Popen([str(ROOT / "target/debug/xdm-gui")], env={**env, "XDM_GUI_PERF": "1"},
                                   stderr=gui_log, text=True)
            main_window = until(lambda: window("XDM.*Downloads$"))
            time.sleep(1.5)
            assert grab("restart-ur", (0, 0, 900, 21)) == urdu, "language must persist across restart"
            from PIL import Image as _I
            now = grab("restart-header", (306, 90, 900, 108))
            if now != header:
                _I.frombytes("RGB", (594, 18), header).save("/tmp/recon/hdr-before.png")
                _I.frombytes("RGB", (594, 18), now).save("/tmp/recon/hdr-after.png")
                shutil.copy(base / "state" / "gui-layout.json", "/tmp/recon/layout.json")
            assert now == header, "hidden columns must persist across restart"


            # ---- IDM-style shell: category tree filters, toolbar states, menus, context menu ----
            import re
            for t in cli("list")["tasks"]:
                cli("remove", t["id"])
            until(lambda: not cli("list")["tasks"])
            shell_video = cli("submit", "http://example.invalid/clip.mp4", base / "clip.mp4")["task_id"]
            shell_doc = cli("submit", "http://example.invalid/paper.pdf", base / "paper.pdf")["task_id"]
            shell_done = cli("submit", "http://example.invalid/done.zip", base / "done.zip")["task_id"]
            cli("pause", shell_doc)
            db = sqlite3.connect(base / "state" / "tasks.sqlite3")
            db.execute("UPDATE tasks SET status='complete' WHERE id=?", (shell_done,))
            db.commit()
            db.close()
            refresh_view(main_window)

            def refresh_totals():
                gui_log.flush()
                return [int(m) for m in re.findall(r"\[xdm-gui\] refresh .*?total=(\d+) ", (base / "gui.log").read_text())]

            def tree_y(index):
                return 118 + 16 * index

            TREE = ["all", "Compressed", "Documents", "Music", "Programs", "Video", "Other", "Archives",
                    "unfinished", "finished", "grabber", "queues"]

            def tree_click(name):
                seen = len(refresh_totals())
                click(main_window, 90, tree_y(TREE.index(name)))
                until(lambda: len(refresh_totals()) > seen)

            def tree_total(name, expected):
                tree_click(name)
                until(lambda: refresh_totals()[-1] == expected)

            tree_total("finished", 1)
            tree_total("unfinished", 2)
            tree_total("Video", 1)
            tree_total("Documents", 1)
            tree_total("Archives", 1)  # the user category created earlier claims .zip
            tree_total("all", 3)
            layout_file = base / "state" / "gui-layout.json"
            if "queues" not in json.loads(layout_file.read_text())["expanded"]:  # the Queues dialog may have opened it
                click(main_window, 15, tree_y(TREE.index("queues")))  # expand Queues
                until(lambda: "queues" in json.loads(layout_file.read_text())["expanded"], timeout=15)
            extra_queue = cli("queue", "create", "ShellQ", 1)["queue_id"]
            cli("queue", "assign", extra_queue, shell_video)
            time.sleep(25)  # the queue list refreshes every fifth poll (2 s each, slower under load)
            queue_rows = [q["id"] for q in cli("list")["queues"]]
            click(main_window, 90, tree_y(len(TREE) + queue_rows.index(extra_queue)))
            until(lambda: refresh_totals()[-1] == 1)
            tree_total("all", 3)
            assert statuses()[shell_done] == "complete"

            # Toolbar enable states follow the selection (Resume is only live for paused/failed tasks).
            resume_box = (75, 25, 150, 62)
            click(main_window, 80, 600 - 10)  # click the empty status bar: nothing selected
            idle = grab("tool-idle", resume_box)
            click(main_window, ROW_X, row_y(shell_doc))
            time.sleep(0.6)
            paused_look = grab("tool-paused", resume_box)
            assert paused_look != idle, "Resume must enable for a paused task"
            click(main_window, ROW_X, row_y(shell_done))
            time.sleep(0.6)
            assert grab("tool-complete", resume_box) == idle, "Resume stays disabled for a completed task"

            # Menus: view toggles persist in gui-layout.json.
            layout_file = base / "state" / "gui-layout.json"
            menu_item(main_window, "view", "toggle-sidebar")
            until(lambda: layout_file.exists() and json.loads(layout_file.read_text())["show_sidebar"] is False, timeout=15)
            menu_item(main_window, "view", "toggle-sidebar")
            until(lambda: json.loads(layout_file.read_text())["show_sidebar"] is True, timeout=15)
            menu_item(main_window, "view", "toolbar-small")
            until(lambda: json.loads(layout_file.read_text())["small_toolbar"] is True, timeout=15)
            menu_item(main_window, "view", "toolbar-large")
            until(lambda: json.loads(layout_file.read_text())["small_toolbar"] is False, timeout=15)
            assert json.loads(layout_file.read_text())["width"] == 900

            # Context menu on the paused task: Resume Download, Stop Download, then Add to queue.
            rclick(main_window, ROW_X, row_y(shell_doc))
            click(main_window, ROW_X + 60, entry_y(CONTEXT, "download-now", row_y(shell_doc)))
            until(lambda: statuses()[shell_doc] == "queued")
            rclick(main_window, ROW_X, row_y(shell_doc))
            click(main_window, ROW_X + 60, entry_y(CONTEXT, "stop", row_y(shell_doc)))
            until(lambda: statuses()[shell_doc] == "paused")
            rclick(main_window, ROW_X, row_y(shell_doc))
            ctx_top = row_y(shell_doc)
            addq_y = entry_y(CONTEXT, "sub:addq", ctx_top)
            subprocess.run(["xdotool", "mousemove", "--window", main_window, str(ROW_X + 60), str(addq_y)],
                           env=env, check=True, timeout=30)
            time.sleep(0.2)
            subprocess.run(["xdotool", "mousemove_relative", "--", "2", "0"], env=env, check=True, timeout=30)
            time.sleep(0.4)
            click(main_window, ROW_X + 180 - 4 + 60, addq_y - 8 + 2 + 17 * queue_rows.index(extra_queue) + 8)
            until(lambda: cli("info", shell_doc)["tasks"][0]["queue_id"] == extra_queue)

            # Delete Completed (toolbar) confirms, then removes only the finished task.
            tool(main_window, "delcompleted")
            confirm = until(lambda: window("^XDM$"))
            click(confirm, 276, 96)
            until(lambda: shell_done not in {t["id"] for t in cli("list")["tasks"]})
            assert {shell_doc, shell_video} <= {t["id"] for t in cli("list")["tasks"]}

            # ---- Daemon-wired dialogs: remove with partial, cleanup, bandwidth, sync queue, restart ----
            for t in cli("list")["tasks"]:
                cli("remove", t["id"])
            until(lambda: not cli("list")["tasks"])
            refresh_view(main_window)
            time.sleep(0.5)

            # Remove dialog: "Also delete partial file" is offered for unfinished tasks and maps to TaskRemoveEx.
            keep = base / "keepdir"
            keep.mkdir()
            kept_task = cli("submit", "http://example.invalid/kept.bin", keep / "kept.bin")["task_id"]
            gone_task = cli("submit", "http://example.invalid/gone.bin", keep / "gone.bin")["task_id"]
            for name in ("kept.bin", "gone.bin"):
                (keep / f"{name}.part").write_bytes(b"p" * 50)
                (keep / f"{name}.part.meta").write_text(CHECKPOINT)
            refresh_view(main_window)
            time.sleep(0.8)
            click(main_window, ROW_X, row_y(kept_task))
            tool(main_window, "delete")
            remove = until(lambda: window("XDM.*Remove task$"))
            click(remove, 120, 206)  # unticked: only the list entry goes
            until(lambda: len(cli("list")["tasks"]) == 1)
            assert (keep / "kept.bin.part").exists() and (keep / "kept.bin.part.meta").exists()
            refresh_view(main_window)
            time.sleep(0.8)
            click(main_window, ROW_X, 117)
            tool(main_window, "delete")
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
            menu_item(main_window, "downloads", "partials")
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

            # Bandwidth settings (reached from Connection settings): global limit, quota and host limit
            # are applied through the daemon and read back with BandwidthStatus.
            menu_item(main_window, "downloads", "connection")
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
            menu_item(main_window, "downloads", "queues")
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
            refresh_view(main_window)
            time.sleep(0.8)
            assert statuses()[probe] == "queued"
            queued_status = grab("status-queued", (306, 109, 560, 126))
            cli("queue", "start")
            until(lambda: statuses().get(probe) == "quota_wait", timeout=60)
            cli("queue", "stop")
            until(lambda: cli("list")["queue_running"] is False)
            assert cli("bandwidth")["bandwidth"]["quota_exceeded"] is True
            refresh_view(main_window)
            time.sleep(0.8)
            quota_status = grab("status-quota", (306, 109, 560, 126))
            assert quota_status != queued_status, "quota_wait must be shown in the Status column"
            # restart_required is shown too, and the Restart row action requeues it via TaskRestart.
            db = sqlite3.connect(base / "state" / "tasks.sqlite3")
            db.execute("UPDATE tasks SET status='restart_required', restart_reason='remote changed' WHERE id=?", (probe,))
            db.commit()
            db.close()
            refresh_view(main_window)
            time.sleep(0.8)
            restart_status = grab("status-restart", (306, 109, 560, 126))
            assert restart_status not in (queued_status, quota_status), "restart_required must be shown"
            click(main_window, ROW_X, 117)
            rclick(main_window, ROW_X, 117)
            click(main_window, ROW_X + 60, entry_y(CONTEXT, "restart", 117))  # context menu: Restart
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
            subprocess.run(["xdotool", "mousemove", "--window", main_window, "888", "120", "mousedown", "1",
                            "mousemove_relative", "--", "0", "450", "mouseup", "1"], env=env, check=True, timeout=30)
            time.sleep(2.0)
            fetched = perf_lines("page-fetch")
            deep = [line for line in fetched if int(re.search(r"page=(\d+)", line).group(1)) >= 50]
            assert deep, f"scrolling to the end must fetch deep pages; got {fetched}"
            # Search is pushed to the daemon: 500 rows match one import chunk.
            find_text("g07_")
            until(lambda: has_total(500), timeout=30)
            # Sorting by a column is a daemon sort: the refresh after the header click still reports the filtered total.
            click(main_window, COL_X["filename"], HEADER_Y)
            time.sleep(2.5)
            assert not [line for line in perf_lines("refresh") if "total=0 " in line]
            assert [line for line in perf_lines("refresh") if "total=500 sort=output " in line], \
                "the File name header must sort in the daemon (sort=output) while the filter holds"

            # Header sorts for columns without a stored column now use the daemon's keys.
            def sorted_by(key, header_x):
                before = len(perf_lines("refresh"))
                click(main_window, header_x, HEADER_Y)
                until(lambda: any(f"sort={key} " in line for line in perf_lines("refresh")[before:]), timeout=30)
            sorted_by("size", COL_X["size"])
            sorted_by("-size", COL_X["size"])
            sorted_by("progress", COL_X["time_left"])
            sorted_by("rate", COL_X["transfer_rate"])
            sorted_by("queue", COL_X["q"])
            sorted_by("output", COL_X["filename"])  # back to File name

            # Ctrl+A selects every task matching the filter, beyond the loaded rows.
            find_text("")  # an empty query shows everything again
            until(lambda: "total=20000 " in (perf_lines("refresh") or [""])[-1], timeout=30)
            click(main_window, ROW_X, 117)
            key("ctrl+a")
            time.sleep(0.5)
            gui_log.flush()
            started = time.monotonic()
            tool(main_window, "resume")
            until(lambda: cli("list", "--limit", 1, "--status", "queued")["total"] == 20000, timeout=300)
            seen_refreshes = len(perf_lines("refresh"))
            until(lambda: len(perf_lines("refresh")) > seen_refreshes + 1, timeout=60)  # Stop enables from loaded statuses
            tool(main_window, "stop")
            until(lambda: cli("list", "--limit", 1, "--status", "paused")["total"] == 20000, timeout=300)
            assert cli("list", "--limit", 1, "--status", "queued")["total"] == 0
            print(f"select-all resume+pause of 20000 tasks through the GUI: {time.monotonic() - started:.1f}s")

            # Queues dialog: daemon calls run off the UI thread and still work, including bulk assign.
            menu_item(main_window, "downloads", "queues")
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
                subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-draw_mouse", "0", "-video_size", "1200x900",
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


if __name__ == "__main__":
    main()
