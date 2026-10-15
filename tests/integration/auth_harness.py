"""Shared helpers for the N11/N17/N23/N26 integration tests: an isolated daemon plus CLI wrapper."""
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class Daemon:
    def __init__(self, prefix, env_extra=None, drop_env=("http_proxy", "https_proxy", "all_proxy", "no_proxy",
                                                            "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY")):
        self.tmp = tempfile.TemporaryDirectory(prefix=prefix)
        self.base = Path(self.tmp.name)
        env = {key: value for key, value in os.environ.items() if key not in drop_env}
        env["XDM_DATA_DIR"] = str(self.base / "state")
        env.update(env_extra or {})
        self.env = env
        self.process = subprocess.Popen([str(ROOT / "target/debug/xdm-daemon")], env=env,
                                        stderr=subprocess.PIPE, text=True)
        assert "listening locally" in self.process.stderr.readline()

    def cli(self, *args, success=True, raw=False, stdin=None):
        result = subprocess.run([str(ROOT / "target/debug/xdm-cli"), "daemon", *map(str, args)],
                                env=self.env, capture_output=True, text=True, timeout=30, input=stdin)
        assert (result.returncode == 0) == success, result.stderr + result.stdout
        if raw:
            return result.stdout
        return json.loads(result.stdout) if result.stdout.strip() else {}

    def info(self, task_id):
        return self.cli("info", task_id)["tasks"][0]

    def until(self, task_id, predicate, timeout=40):
        deadline = time.monotonic() + timeout
        row = self.info(task_id)
        while time.monotonic() < deadline:
            if predicate(row):
                return row
            time.sleep(0.1)
            row = self.info(task_id)
        raise AssertionError(row)

    def secret(self, name, text):
        path = self.base / name
        path.write_text(text + "\n")
        return path

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=5)
        self.tmp.cleanup()
