"""Independent queue start, manual order, completed-member exit, retries, schedules and completion events."""
import http.server
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class NotFound(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    do_HEAD = do_GET

    def log_message(self, *args):
        pass


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-queue-policies-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state")}
        fixture = subprocess.Popen([sys.executable, str(ROOT / "docs/analysis/fault_fixture.py"),
                                    str(base / "http"), "--delay", "0.003"],
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        missing = http.server.ThreadingHTTPServer(("127.0.0.1", 0), NotFound)
        threading.Thread(target=missing.serve_forever, daemon=True).start()
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=5)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout) if result.stdout.strip() else None

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def until(predicate, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                state = cli("list")
                if predicate(state):
                    return state
                time.sleep(0.1)
            raise AssertionError(state)

        def queue(state, queue_id):
            return next(row for row in state["queues"] if row["id"] == queue_id)

        def task(state, task_id):
            return next(row for row in state["tasks"] if row["id"] == task_id)

        def new_task(port, name, queue_id=None, rate=512):
            task_id = cli("submit", f"http://127.0.0.1:{port}/{name}.bin", base / f"{name}.bin")["task_id"]
            if rate:
                cli("rate", task_id, rate)
            if queue_id:
                cli("queue", "assign", queue_id, task_id)
            return task_id

        try:
            port = json.loads(fixture.stdout.readline())["port"]
            daemon = start()
            cli("limit", 4)

            # Q03: a queue started on its own runs while the global gate is stopped.
            cli("queue", "stop")
            b = cli("queue", "create", "Independent", 1)["queue_id"]
            main_task = new_task(port, "main-held")
            b_task = new_task(port, "indep", b)
            cli("queue", "start-one", b)
            state = until(lambda s: queue(s, b)["active"] == 1)
            assert state["queue_running"] is False and queue(state, b)["started_alone"]
            time.sleep(0.5)
            state = cli("list")
            assert task(state, main_task)["status"] == "queued" and queue(state, 1)["active"] == 0
            cli("queue", "stop")  # Stop all dominates independent starts.
            state = until(lambda s: queue(s, b)["active"] == 0)
            assert not queue(state, b)["started_alone"] and task(state, b_task)["status"] == "queued"
            cli("queue", "start-one", b)
            until(lambda s: queue(s, b)["active"] == 1)
            cli("queue", "stop-one", b)
            until(lambda s: queue(s, b)["active"] == 0)
            cli("queue", "start-one", 99, success=False)

            # Q06: completed members leave the queue and keep their history.
            cli("rate", b_task, "unlimited")
            cli("queue", "start-one", b)
            state = until(lambda s: task(s, b_task)["status"] == "complete")
            row = task(state, b_task)
            assert row["queue_id"] == 0 and row["history_queue_id"] == b
            assert queue(state, b)["complete"] == 1 and queue(state, b)["queued"] == 0
            cli("queue", "stop-one", b)
            cli("queue", "remove", b)
            cli("rate", main_task, "unlimited")
            cli("queue", "start")
            until(lambda s: task(s, main_task)["status"] == "complete")
            cli("queue", "stop")

            # Q05: manual order persists across restart and drives selection.
            c = cli("queue", "create", "Ordered", 1)["queue_id"]
            ids = [new_task(port, f"order-{i}", c) for i in range(3)]
            cli("move", ids[2], "top")
            cli("move", ids[0], "down")
            expected = [ids[2], ids[1], ids[0]]

            def ordered(state):
                rows = [r for r in state["tasks"] if r["queue_id"] == c]
                return [r["id"] for r in sorted(rows, key=lambda r: (r["position"], r["id"]))]

            assert ordered(cli("list")) == expected
            cli("move", ids[0], 9, success=False)
            cli("move", main_task, "top", success=False)  # completed: not in a queue
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            assert ordered(cli("list")) == expected
            cli("queue", "start-one", c)
            state = until(lambda s: queue(s, c)["active"] == 1)
            assert task(state, expected[0])["status"] == "running"
            cli("queue", "stop-one", c)
            until(lambda s: queue(s, c)["active"] == 0)
            for task_id in ids:
                cli("rate", task_id, "unlimited")
            cli("queue", "start-one", c)
            until(lambda s: all(task(s, i)["status"] == "complete" for i in ids))

            # Q14: completion events fire once; a recorded action is never executed.
            d = cli("queue", "create", "Finish", 1)["queue_id"]
            cli("queue", "completion", d, "format-disk", success=False)
            cli("queue", "completion", d, "shutdown")
            done_task = new_task(port, "finish", d, rate=None)
            cli("queue", "start-one", d)
            until(lambda s: task(s, done_task)["status"] == "complete")
            time.sleep(1)
            events = cli("queue", "events", d)["queue_events"]
            assert [(e["outcome"], e["action"], e["action_status"]) for e in events] == [("completed", "shutdown", "recorded")]
            assert queue(cli("list"), d)["run_state"] == "completed"
            e = cli("queue", "create", "Stopped", 1)["queue_id"]
            cli("queue", "completion", e, "sleep")
            new_task(port, "stopped", e)
            cli("queue", "start-one", e)
            until(lambda s: queue(s, e)["run_state"] == "running")
            cli("queue", "stop-one", e)
            events = cli("queue", "events", e)["queue_events"]
            assert [(x["outcome"], x["action_status"]) for x in events] == [("manual_stop", "suppressed")]

            # Q07: retry budget for transient errors, none for permanent ones.
            r = cli("queue", "create", "Retry", 2)["queue_id"]
            cli("queue", "retry", r, -5, 1, success=False)
            cli("queue", "retry", r, 2, 1)
            with socket.socket() as closed:
                closed.bind(("127.0.0.1", 0))
                dead_port = closed.getsockname()[1]
            transient = new_task(dead_port, "dead", r, rate=None)
            permanent = new_task(missing.server_address[1], "gone", r, rate=None)
            cli("queue", "start-one", r)
            state = until(lambda s: task(s, permanent)["status"] == "failed", 30)
            assert task(state, permanent)["retry_count"] == 0 and "404" in task(state, permanent)["error"]
            state = until(lambda s: task(s, transient)["status"] == "failed", 60)
            assert task(state, transient)["retry_count"] == 2
            assert queue(state, r)["retry_max"] == 2 and queue(state, r)["retry_delay"] == 1
            assert queue(state, r)["run_state"] in ("completed_with_failures", "running")
            until(lambda s: queue(s, r)["run_state"] == "completed_with_failures")
            cli("queue", "stop-one", r)

            # Q08/Q09: one-time and daily schedules, per queue.
            s = cli("queue", "create", "Scheduled", 1)["queue_id"]
            new_task(port, "scheduled", s)
            now = int(time.time())
            cli("queue", "schedule", s, "once", now - 5, now - 1, success=False)
            cli("queue", "schedule", s, "once", now + 3, now + 90)
            assert not queue(cli("list"), s)["started_alone"]
            until(lambda st: queue(st, s)["started_alone"], 15)
            cli("queue", "stop-one", s)
            time.sleep(2.5)  # stop inside the window is not undone by the schedule
            assert not queue(cli("list"), s)["running"]
            cli("queue", "schedule", s, "daily", "09:00", "10:00", "none-day", success=False)
            local = time.localtime()
            minute = local.tm_hour * 60 + local.tm_min
            hhmm = lambda m: f"{m % 1440 // 60:02d}:{m % 1440 % 60:02d}"
            cli("queue", "schedule", s, "daily", hhmm(minute - 1), hhmm(minute + 30), "all")
            state = until(lambda st: queue(st, s)["started_alone"], 15)
            assert queue(state, s)["schedule_kind"] == "daily" and queue(state, s)["schedule_days"] == 127
            cli("queue", "stop-one", s)
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            time.sleep(2.5)  # same occurrence is not executed twice across restart
            state = cli("list")
            assert not queue(state, s)["started_alone"] and queue(state, s)["schedule_kind"] == "daily"
            cli("queue", "schedule", s, "none")
            assert queue(cli("list"), s)["schedule_kind"] == "none"
            cli("queue", "stop")
            print("independent start, manual order, completed-member exit, retries, schedules and completion events passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            fixture.terminate(); fixture.wait(timeout=5)
            missing.shutdown()


if __name__ == "__main__":
    main()
