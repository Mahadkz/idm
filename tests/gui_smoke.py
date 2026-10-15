#!/usr/bin/env python3
"""GUI smoke: start the daemon and xdm-gui with the software renderer, require that the window
renders a snapshot (XDM_GUI_PERF log) and that the GUI stays alive, then stop both.
Works on Windows (needs a desktop session, as on GitHub's windows runner) and on Linux under
Xvfb (xvfb-run -a python3 tests/gui_smoke.py). Run after cargo build --workspace."""
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXE = ".exe" if os.name == "nt" else ""


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-gui-smoke-") as temp:
        env = {**os.environ, "XDM_DATA_DIR": str(Path(temp) / "state"), "HOME": temp, "USERPROFILE": temp,
               "SLINT_BACKEND": "winit-software", "XDM_GUI_PERF": "1", "XDM_FAKE_OS_ACTIONS": "1"}
        daemon = subprocess.Popen([str(ROOT / f"target/debug/xdm-daemon{EXE}")], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        gui = None
        try:
            time.sleep(2)
            gui = subprocess.Popen([str(ROOT / f"target/debug/xdm-gui{EXE}")], env=env,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            lines, rendered = [], threading.Event()

            def pump():
                for line in gui.stderr:
                    lines.append(line)
                    if "snapshot_ok=true" in line:
                        rendered.set()
            threading.Thread(target=pump, daemon=True).start()
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline and not rendered.is_set():
                if gui.poll() is not None:
                    raise SystemExit(f"xdm-gui exited early with code {gui.returncode}\n" + "".join(lines[-30:]))
                time.sleep(0.5)
            if not rendered.is_set():
                raise SystemExit("xdm-gui never logged a rendered snapshot\n" + "".join(lines[-30:]))
            time.sleep(3)
            if gui.poll() is not None:
                raise SystemExit(f"xdm-gui crashed after rendering (code {gui.returncode})")
            print("gui smoke ok: window rendered and stayed alive")
        finally:
            for process in (gui, daemon):
                if process and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(10)
                    except subprocess.TimeoutExpired:
                        process.kill()


if __name__ == "__main__":
    main()
