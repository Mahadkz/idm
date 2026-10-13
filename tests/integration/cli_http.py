"""End-to-end CLI checks against disposable HTTP faults; run after cargo build -p xdm-cli."""

import hashlib
import json
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target/debug/xdm-cli"
FIXTURE = ROOT / "docs/analysis/fault_fixture.py"
EXPECTED = hashlib.sha256(bytes(range(256)) * (8 * 1024 * 1024 // 256)).hexdigest()


def run(*args, success=True):
    result = subprocess.run([str(CLI), *map(str, args)], text=True, capture_output=True, timeout=60)
    assert (result.returncode == 0) == success, (args, result.returncode, result.stderr)
    return result


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    with tempfile.TemporaryDirectory(prefix="xdm-cli-test-") as temp:
        directory = Path(temp)
        server = subprocess.Popen(
            [sys.executable, str(FIXTURE), str(directory), "--delay", "0.05"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            port = json.loads(server.stdout.readline())["port"]
            url = lambda name: f"http://127.0.0.1:{port}/{name}"
            for name in ["queue-a.bin", "retry.bin", "wrong-range.bin"]:
                output = directory / f"out-{name}"
                run(url(name), output)
                assert digest(output) == EXPECTED, name
            denied = directory / "denied.bin"
            run(url("denied.bin"), denied, success=False)
            assert not denied.exists()

            paused = directory / "paused.bin"
            command = [str(CLI), url("changed.bin"), str(paused), "--connections", "4"]
            worker = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(0.3)
            worker.send_signal(signal.SIGINT)
            stdout, stderr = worker.communicate(timeout=15)
            assert worker.returncode != 0 and "paused" in stderr, (stdout, stderr)
            metadata = Path(str(paused) + ".part.meta")
            partial = Path(str(paused) + ".part")
            assert metadata.exists() and partial.exists()
            checkpoint = json.loads(metadata.read_text())
            assert any(segment["cursor"] > segment["start"] for segment in checkpoint["segments"])
            changed = directory / "changed.bin"
            Path(str(changed) + ".part").write_bytes(partial.read_bytes())
            Path(str(changed) + ".part.meta").write_bytes(metadata.read_bytes())
            run(*command[1:])
            assert digest(paused) == EXPECTED
            with urllib.request.urlopen(url("control/version2"), timeout=5):
                pass
            result = run(url("changed.bin"), changed, "--connections", "4", success=False)
            assert "identity changed" in result.stderr and not changed.exists()

            limited = directory / "limited.bin"
            started = time.monotonic()
            run(url("queue-a.bin"), limited, "--header", "User-Agent: xdm-fixture", "--rate-kib", "8192")
            elapsed = time.monotonic() - started
            assert digest(limited) == EXPECTED and elapsed >= 0.75, elapsed
            requests = [json.loads(line) for line in (directory / "requests.jsonl").read_text().splitlines()]
            assert any(record.get("headers", {}).get("user-agent") == "xdm-fixture" for record in requests)
            print(json.dumps({"result": "passed", "cases": ["range", "503 retry", "bad range fallback", "403", "pause/resume", "changed validator", "headers/rate"], "rate_case_seconds": round(elapsed, 2)}))
        finally:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)


if __name__ == "__main__":
    main()
