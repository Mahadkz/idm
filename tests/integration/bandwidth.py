"""Global/per-host rate caps, host connection budget and usage quota (N44-N47)."""
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIB = 1024 * 1024
BLOCK = bytes(range(256)) * (MIB // 256)


def expected(size):
    digest, done = hashlib.sha256(), 0
    while done < size:
        count = min(MIB, size - done)
        digest.update(BLOCK[:count])
        done += count
    return digest.hexdigest()


class Stats:
    lock = threading.Lock()
    inflight = {}
    peak = {}
    log = []


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *unused):
        pass

    def handle(self):
        try:
            super().handle()
        except OSError:
            pass

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        size = int(self.path.strip("/").split(".")[0].removeprefix("f") or 1) * MIB
        host = (self.headers.get("Host") or "").split(":")[0]
        start, end, status = 0, size - 1, 200
        match = re.fullmatch(r"bytes=(\d+)-(\d*)", self.headers.get("Range") or "")
        if match:
            start = int(match.group(1))
            end = min(int(match.group(2)) if match.group(2) else size - 1, size - 1)
            status = 206
        with Stats.lock:
            now = Stats.inflight[host] = Stats.inflight.get(host, 0) + 1
            Stats.peak[host] = max(Stats.peak.get(host, 0), now)
            Stats.log.append((round(time.monotonic() % 1000, 3), 'start', self.client_address[1], now, self.headers.get('Range')))
        try:
            self.send_response(status)
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            self.send_header("ETag", '"fixed"')
            if status == 206:
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.end_headers()
            if self.command == "HEAD":
                return
            position = start
            while position <= end:
                count = min(64 * 1024, end - position + 1)
                offset = position % MIB
                chunk = (BLOCK * 2)[offset:offset + count]
                self.wfile.write(chunk)
                position += count
        except OSError:
            pass
        finally:
            with Stats.lock:
                Stats.inflight[host] -= 1
                Stats.log.append((round(time.monotonic() % 1000, 3), 'end', self.client_address[1], Stats.inflight[host], None))


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    with tempfile.TemporaryDirectory(prefix="xdm-bandwidth-") as directory:
        base = Path(directory)
        env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
        env["XDM_DATA_DIR"] = str(base / "state")
        daemon = None

        def cli(*args, success=True):
            result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *args],
                                    env=env, capture_output=True, text=True, timeout=10)
            assert (result.returncode == 0) == success, result.stderr + result.stdout
            return json.loads(result.stdout)

        def start():
            process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                       stderr=subprocess.PIPE, text=True)
            assert "listening locally" in process.stderr.readline()
            return process

        def tasks():
            return {t["id"]: t for t in cli("list")["tasks"]}

        def submit(host, name, mib, rate=None):
            url = f"http://{host}:{port}/f{mib}.bin"
            reply = cli("submit", url, str(base / name))
            if rate:
                cli("rate", str(reply["task_id"]), str(rate))
            return reply["task_id"]

        def wait_for(predicate, timeout=30):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                current = tasks()
                if predicate(current):
                    return current
                time.sleep(0.1)
            raise AssertionError(current)

        def window(ids, seconds):
            """Per-task and total MiB/s over `seconds`, measured on live counters."""
            first = tasks()
            began = time.monotonic()
            time.sleep(seconds)
            second = tasks()
            span = time.monotonic() - began
            rates = {i: (second[i]["downloaded"] - first[i]["downloaded"]) / MIB / span for i in ids}
            return rates, sum(rates.values())

        def finish_all(ids):
            cli("limit-global", "unlimited")
            wait_for(lambda t: all(t[i]["status"] == "complete" for i in ids), 60)

        def verify(name, mib):
            assert hashlib.sha256((base / name).read_bytes()).hexdigest() == expected(mib * MIB), name

        try:
            daemon = start()
            cli("proxy", "direct")
            cli("queue", "stop")

            # N46 global aggregate cap: two tasks share 4 MiB/s fairly.
            cli("limit-global", "4096")
            a, b = submit("127.0.0.1", "g1.bin", 40), submit("127.0.0.1", "g2.bin", 40)
            cli("queue", "start")
            wait_for(lambda t: all((t[i]["downloaded"] or 0) > 0 for i in (a, b)))
            rates, total = window((a, b), 3)
            assert total <= 4.4, (rates, total)
            assert all(rate >= 1.5 for rate in rates.values()), rates
            cli("limit-global", "8192")
            time.sleep(0.3)
            rates, total = window((a, b), 2)
            assert 5.5 <= total <= 8.8, ("live raise", rates, total)
            status = cli("bandwidth")["bandwidth"]
            assert status["global_kib"] == 8192 and status["active_http_tasks"] == 2, status
            finish_all((a, b))
            verify("g1.bin", 40); verify("g2.bin", 40)
            print("global cap: <=4.4 MiB/s total, fair share, live change passed")

            # N46 per-host cap: only the capped host name is slowed.
            cli("limit-host", "localhost", "1024")
            cli("limit-host", "bad host!", "1", success=False)
            fast = submit("127.0.0.1", "h1.bin", 40, rate=6144)
            slow = submit("localhost", "h2.bin", 8)
            wait_for(lambda t: all((t[i]["downloaded"] or 0) > 0 for i in (fast, slow)))
            rates, _ = window((fast, slow), 2.5)
            assert rates[slow] <= 1.25 and rates[fast] >= 4.0, rates
            cli("limit-host", "localhost", "remove")
            time.sleep(0.4)
            rates, _ = window((fast, slow), 1) if tasks()[slow]["status"] == "running" else ({slow: 99}, 0)
            assert rates[slow] > 1.5, rates
            wait_for(lambda t: all(t[i]["status"] == "complete" for i in (fast, slow)), 60)
            verify("h1.bin", 40); verify("h2.bin", 8)
            assert cli("bandwidth")["bandwidth"]["host_limits"] == []
            print("per-host cap isolates the capped host and live removal passed")

            # N44 connection budget; first the control without a budget.
            def peak_for(label, budget):
                Stats.peak.clear()
                Stats.log.clear()
                if budget:
                    cli("host-connections", "127.0.0.1", str(budget))
                ids = [submit("127.0.0.1", f"{label}{n}.bin", 16, rate=3072) for n in (1, 2)]
                wait_for(lambda t: all(t[i]["status"] == "complete" for i in ids), 60)
                for n in (1, 2):
                    verify(f"{label}{n}.bin", 16)
                return Stats.peak.get("127.0.0.1", 0)

            free = peak_for("c", None)
            assert free > 3, free
            held = peak_for("d", 3)
            assert 1 <= held <= 3, (held, [e for e in Stats.log if e[3] is not None][-60:])
            status = cli("bandwidth")["bandwidth"]
            assert status["host_connections"] == [["127.0.0.1", 3]], status
            cli("host-connections", "127.0.0.1", "remove")
            cli("host-connections", "x", "0", success=False)
            print(f"connection budget: unbudgeted peak {free}, budget 3 peak {held} passed")

            # N47 quota: pause at the limit, resume with the next window, survive restart.
            cli("quota", "6", "0.002")  # 6 MiB per 7.2 s
            q = submit("127.0.0.1", "q.bin", 10, rate=4096)
            wait_for(lambda t: t[q]["status"] == "quota_wait", 15)
            status = cli("bandwidth")["bandwidth"]
            assert status["quota_exceeded"] and status["quota_used_bytes"] >= 6 * MIB, status
            assert status["quota_resets_at"] is not None, status
            assert status["quota_used_bytes"] <= 8 * MIB, status
            time.sleep(1)
            assert tasks()[q]["status"] == "quota_wait"
            assert cli("bandwidth")["bandwidth"]["quota_used_bytes"] == status["quota_used_bytes"]
            daemon.terminate(); daemon.wait(timeout=5)
            daemon = start()
            status = cli("bandwidth")["bandwidth"]
            assert status["quota_used_bytes"] >= 6 * MIB and status["quota_mib"] == 6, status
            assert tasks()[q]["status"] == "quota_wait"
            began = time.monotonic()
            wait_for(lambda t: t[q]["status"] in ("running", "complete"), 15)
            assert time.monotonic() - began < 9
            wait_for(lambda t: t[q]["status"] == "complete", 30)
            verify("q.bin", 10)
            cli("quota", "off")
            assert not cli("bandwidth")["bandwidth"]["quota_exceeded"]
            print("quota pause, persisted ledger, automatic resume and hash passed")
        finally:
            if daemon and daemon.poll() is None:
                daemon.terminate(); daemon.wait(timeout=5)
            server.shutdown()


if __name__ == "__main__":
    main()
