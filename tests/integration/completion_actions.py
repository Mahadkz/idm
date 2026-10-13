"""Completion actions: notify recording, opt-in destructive countdown with
cancel/suppression, program launch with literal arguments. Runs with
XDM_FAKE_OS_ACTIONS=1 so no real power action can ever execute; the fake
adapter records what would have run into the data dir's os_actions.json."""
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = bytes(range(256)) * (256 * 1024)  # 4 MiB
PROGRAM_SCRIPT = '''#!/usr/bin/env python3
import json, sys
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    handle.write(json.dumps(sys.argv[2:]))
'''

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if self.path == "/missing.bin":
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
        else:
            self.send_response(200)
            self.send_header("Content-Length", str(len(PAYLOAD)))
            self.end_headers()
            self.wfile.write(PAYLOAD)

    def log_message(self, *args):
        pass


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-completion-") as directory:
        base = Path(directory)
        state = base / "state"
        env = {**os.environ,
               "XDM_DATA_DIR": str(state),
               "XDM_FAKE_OS_ACTIONS": "1"}
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        port = server.server_address[1]
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, text=True, capture_output=True, timeout=10)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout)

        def rpc(kind, **payload):
            import socket
            import struct
            request = {"version": 1, "token": (state / "ipc-token").read_text(),
                       "command": {"type": kind, "payload": payload}}
            data = json.dumps(request).encode()
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(10)
                stream.connect(str(state / "daemon.sock"))
                stream.sendall(struct.pack("=I", len(data)) + data)
                with stream.makefile("rb") as reader:
                    size = struct.unpack("=I", reader.read(4))[0]
                    return json.loads(reader.read(size))

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def records(action, queue_id):
            path = state / "os_actions.json"
            if not path.exists():
                return []
            entries = []
            for line in path.read_text().splitlines():
                entry = json.loads(line)
                if entry["action"] == action and entry["queue_id"] == queue_id:
                    entries.append(entry)
            return entries

        def until(predicate, timeout=20, step=0.2):
            deadline = time.monotonic() + timeout
            result = None
            while time.monotonic() < deadline:
                result = predicate()
                if result:
                    return result
                time.sleep(step)
            raise AssertionError(result)

        try:
            daemon = start()
            cli("queue", "stop")

            # Queue completes -> notify recorded through the fake adapter.
            notify_queue = cli("queue", "create", "Notify", 1)["queue_id"]
            cli("queue", "completion", notify_queue, "notify")
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "notify.bin")["task_id"]
            cli("queue", "assign", notify_queue, task)
            cli("queue", "start-one", notify_queue)
            until(lambda: records("notify", notify_queue))
            events = cli("queue", "events", notify_queue)["queue_events"]
            assert [(e["outcome"], e["action"], e["action_status"]) for e in events] == [
                ("completed", "notify", "recorded")]

            # A default client keeps the recorded-only legacy semantics and
            # never sees a countdown.
            legacy_queue = cli("queue", "create", "Legacy", 1)["queue_id"]
            cli("queue", "completion", legacy_queue, "shutdown")
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "legacy.bin")["task_id"]
            cli("queue", "assign", legacy_queue, task)
            cli("queue", "start-one", legacy_queue)
            until(lambda: cli("queue", "events", legacy_queue)["queue_events"])
            events = cli("queue", "events", legacy_queue)["queue_events"]
            assert [(e["outcome"], e["action"], e["action_status"]) for e in events] == [
                ("completed", "shutdown", "recorded")]
            assert not records("shutdown", legacy_queue)
            assert not cli("completion", "status")["completion_pending"]

            # Destructive without opt-in is rejected.
            guarded_queue = cli("queue", "create", "Guarded", 1)["queue_id"]
            rejected = rpc("queue_set_completion_action", queue_id=guarded_queue,
                           action={"kind": "shutdown"}, confirm=False, countdown_seconds=None)
            assert not rejected["ok"], rejected
            assert "confirm-destructive" in rejected["error"]

            # With opt-in: countdown visible, then recorded.
            power_queue = cli("queue", "create", "Power", 1)["queue_id"]
            cli("queue", "completion", power_queue, "shutdown", "--confirm-destructive", "--countdown", 4)
            assert next(row for row in cli("list")["queues"]
                        if row["id"] == power_queue)["completion_action"] == "shutdown"
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "power.bin")["task_id"]
            cli("queue", "assign", power_queue, task)
            cli("queue", "start-one", power_queue)
            pending = until(lambda: [row for row in cli("completion", "status")["completion_pending"]
                                     if row["queue_id"] == power_queue]) [0]
            assert pending["action"] == "shutdown"
            assert 0 < pending["seconds_remaining"] <= 4, pending
            until(lambda: not [row for row in cli("completion", "status")["completion_pending"]
                               if row["queue_id"] == power_queue], timeout=10)
            events = cli("queue", "events", power_queue)["queue_events"]
            assert events[0]["action_status"] == "recorded", events
            assert records("shutdown", power_queue)

            # Cancel during the countdown: not recorded.
            cancel_queue = cli("queue", "create", "Cancelled", 1)["queue_id"]
            cli("queue", "completion", cancel_queue, "shutdown", "--confirm-destructive", "--countdown", 30)
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "cancel.bin")["task_id"]
            cli("queue", "assign", cancel_queue, task)
            cli("queue", "start-one", cancel_queue)
            until(lambda: [row for row in cli("completion", "status")["completion_pending"]
                           if row["queue_id"] == cancel_queue])
            cli("completion", "cancel", cancel_queue)
            assert not [row for row in cli("completion", "status")["completion_pending"]
                        if row["queue_id"] == cancel_queue]
            time.sleep(1.5)
            assert not records("shutdown", cancel_queue)
            events = cli("queue", "events", cancel_queue)["queue_events"]
            assert events[0]["action_status"] == "cancelled", events

            # A manual stop after the cycle suppresses the armed action.
            stopped_queue = cli("queue", "create", "Stopped", 1)["queue_id"]
            cli("queue", "completion", stopped_queue, "shutdown", "--confirm-destructive", "--countdown", 30)
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "stopped.bin")["task_id"]
            cli("queue", "assign", stopped_queue, task)
            cli("queue", "start-one", stopped_queue)
            until(lambda: [row for row in cli("completion", "status")["completion_pending"]
                           if row["queue_id"] == stopped_queue])
            cli("queue", "stop-one", stopped_queue)
            until(lambda: not [row for row in cli("completion", "status")["completion_pending"]
                               if row["queue_id"] == stopped_queue])
            events = cli("queue", "events", stopped_queue)["queue_events"]
            assert events[0]["action_status"] == "suppressed", events
            assert not records("shutdown", stopped_queue)

            # Program launch passes every argument literally, spaces and shell
            # metacharacters included; no shell interpolation happens.
            script = base / "report.py"
            script.write_text(PROGRAM_SCRIPT)
            script.chmod(0o755)
            argv_file = base / "argv.json"
            program_queue = cli("queue", "create", "Program", 1)["queue_id"]
            expected_args = ["hello world", "a;b|>$HOME &&", "$(reboot)", "  spaced  ", "--weird-flag"]
            cli("queue", "completion", program_queue, "program", script, argv_file, *expected_args)
            task = cli("submit", f"http://127.0.0.1:{port}/file.bin", base / "program.bin")["task_id"]
            cli("queue", "assign", program_queue, task)
            cli("queue", "start-one", program_queue)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not argv_file.exists():
                time.sleep(0.2)
            assert argv_file.exists(), "the program was not launched"
            assert json.loads(argv_file.read_text()) == expected_args
            entry = until(lambda: records("program", program_queue) or None)[0]
            assert entry["program"] == str(script)
            assert entry["args"] == [str(argv_file), *expected_args]
            assert entry["exit"] == 0
            events = cli("queue", "events", program_queue)["queue_events"]
            assert events[0]["action_status"] == "executed", events
            # Power actions that were never recorded: the daemon never launched one.
            all_entries = json.loads(json.dumps(
                [json.loads(line) for line in (state / "os_actions.json").read_text().splitlines()]))
            assert all(entry["action"] in ("notify", "shutdown", "program") for entry in all_entries)
            print("completion notify, opt-in countdown, cancel, suppression and literal program args passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            server.shutdown()


if __name__ == "__main__":
    main()
