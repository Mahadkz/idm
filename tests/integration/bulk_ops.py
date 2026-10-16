"""TaskBulk: bulk actions by filter or ids on a 20,000-task on-disk daemon, with timings."""
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-bulk-") as directory:
        base = Path(directory)
        env = {**os.environ, "XDM_DATA_DIR": str(base / "state"), "XDM_FAKE_OS_ACTIONS": "1"}
        daemon = subprocess.Popen([str(DAEMON)], env=env, stderr=subprocess.PIPE, text=True)
        try:
            assert "listening locally" in daemon.stderr.readline()

            def cli(*args, check=True):
                result = subprocess.run([str(CLI), "daemon", *map(str, args)], env=env,
                                        capture_output=True, text=True, timeout=900)
                if check:
                    assert result.returncode == 0, result.stdout + result.stderr
                return json.loads(result.stdout)

            def count(**flt):
                args = ["list", "--limit", 1]
                for key, value in flt.items():
                    args += [f"--{key}", value]
                return cli(*args)["total"]

            cli("queue", "stop")
            for chunk in range(40):
                cli("batch", f"http://example.invalid/b{chunk:02d}_[0001-0500].bin", base / f"o{chunk:02d}",
                    "--allow-duplicates")
            assert count() == 20000
            assert count(status="paused") == 20000  # imports without --start are paused

            timings = {}

            def timed(name, *args, check=True):
                started = time.monotonic()
                reply = cli("bulk", *args, check=check)
                timings[name] = round((time.monotonic() - started) * 1000)
                return reply

            # Stale expectation refuses and changes nothing.
            refused = timed("refused", "resume", "--expect", 19999, check=False)
            assert refused["ok"] is False and "expected" in refused["error"]
            assert count(status="paused") == 20000
            # Sub-filter with the right expectation.
            reply = timed("resume_500", "resume", "--search", "b07_", "--expect", 500)
            assert reply["bulk"]["done"] == 500 and reply["bulk"]["failed"] == 0
            assert count(status="queued") == 500
            # Whole table.
            reply = timed("resume_20k", "resume", "--expect", 20000)
            assert reply["bulk"]["matched"] == 20000 and reply["bulk"]["done"] == 19500
            assert reply["bulk"]["failed"] == 500 and count(status="queued") == 20000
            reply = timed("pause_20k", "pause", "--expect", 20000)
            assert reply["bulk"]["done"] == 20000 and count(status="paused") == 20000
            reply = timed("priority_20k", "priority", 2, "--expect", 20000)
            assert reply["bulk"]["done"] == 20000
            assert cli("list", "--limit", 1)["tasks"][0]["priority"] == 2
            bad = timed("priority_bad", "priority", 7, check=False)
            assert bad["ok"] is False
            queue = cli("queue", "create", "Bulk", 2)["queue_id"]
            reply = timed("assign_20k", "assign-queue", queue, "--expect", 20000)
            assert reply["bulk"]["done"] == 20000 and count(queue=queue) == 20000
            ids = [t["id"] for t in cli("list", "--limit", 3)["tasks"]]
            reply = timed("remove_ids", "remove", "--ids", ",".join(map(str, ids)), "--expect", 3)
            assert reply["bulk"]["done"] == 3 and count() == 19997
            again = cli("bulk", "remove", "--ids", str(ids[0]))
            assert again["bulk"]["failed"] == 1 and again["bulk"]["failures"][0]["task_id"] == ids[0]
            reply = timed("remove_20k", "remove-with-partials", "--expect", 19997)
            assert reply["bulk"]["done"] == 19997 and count() == 0
            print("bulk timings (ms):", timings)
            print("bulk operations passed")
        finally:
            daemon.terminate()
            daemon.wait(timeout=15)


if __name__ == "__main__":
    main()
