#!/usr/bin/env python3
"""Reproducible RAM/CPU resource benchmark for XDM (Python 3, stdlib only).

Builds the release ``xdm-cli``/``xdm-daemon`` binaries, serves a deterministic
Range-capable body from a local fixture (``bench/fixture_server.py``), runs the
download and daemon scenarios, and samples the process tree through
``/proc/<pid>/status`` and ``/proc/<pid>/stat`` every 100 ms.

Usage:
    . /tmp/xdm-work/env.sh          # provides the offline cargo toolchain
    python3 bench/run_bench.py [--quick] [--no-build]
    python3 bench/run_bench.py --compare old.json new.json [--threshold 10]

The ``--compare`` mode prints every regression beyond the threshold and exits
non-zero, so it can be used as the performance gate.

Every run uses disposable data directories under ``--tmp`` (default
``/tmp/opencode``); the real XDM data directory is never touched. The daemon is
started with ``XDM_DATA_DIR``/``HOME`` pointed at the temporary tree and with
``XDM_FAKE_OS_ACTIONS=1`` so a completion action can never reach the OS.
"""

import argparse
import hashlib
import json
import os
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

INTERVAL_SECONDS = 0.1
CLK_TCK = os.sysconf("SC_CLK_TCK")
GIB = 1024 ** 3
MIB = 1024 ** 2
ENV_SCRIPT = "/tmp/xdm-work/env.sh"
PROC = Path("/proc")

# Scenario sizes. `--quick` keeps the whole run well under a minute.
DEFAULT_SIZE_MIB = 256
QUICK_SIZE_MIB = 32
DEFAULT_CONCURRENT_MIB = 64
QUICK_CONCURRENT_MIB = 8
DEFAULT_IDLE_SECONDS = 30
QUICK_IDLE_SECONDS = 5
DEFAULT_QUEUED_TASKS = 1000
QUICK_QUEUED_TASKS = 200
DEFAULT_RATE_KIB = 32 * 1024
QUICK_RATE_KIB = 8 * 1024


def log(message):
    print(f"[bench] {message}", file=sys.stderr, flush=True)


# --------------------------------------------------------------------------- #
# Environment and build
# --------------------------------------------------------------------------- #


def load_env():
    """Returns the environment produced by sourcing the offline toolchain file."""
    if not os.path.exists(ENV_SCRIPT):
        log(f"{ENV_SCRIPT} is missing; using the current environment")
        return os.environ.copy()
    result = subprocess.run(
        ["bash", "-c", f"set -a; . {shlex.quote(ENV_SCRIPT)}; env -0"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"could not source {ENV_SCRIPT}: {result.stderr.strip()}")
    env = {}
    for entry in result.stdout.split("\0"):
        if "=" in entry:
            name, value = entry.split("=", 1)
            env[name] = value
    return env


def build_binaries(repo, env):
    """Builds the release binaries. The caller must have sourced the env file."""
    log("cargo build --release --offline -p xdm-cli -p xdm-daemon")
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--offline",
            "-p",
            "xdm-cli",
            "-p",
            "xdm-daemon",
        ],
        cwd=repo,
        env=env,
        check=True,
    )
    for name in ("xdm-cli", "xdm-daemon"):
        binary = repo / "target" / "release" / name
        if not binary.exists():
            raise SystemExit(f"expected binary at {binary}")
        log(f"using {binary}")


# --------------------------------------------------------------------------- #
# /proc sampling
# --------------------------------------------------------------------------- #


def read_status(pid):
    """Reads VmRSS/VmHWM/context switches; None once the process is gone."""
    try:
        with open(PROC / str(pid) / "status", "rb") as handle:
            fields = {}
            for line in handle.read().decode("ascii", "replace").splitlines():
                name, separator, value = line.partition(":")
                if separator:
                    fields[name.strip()] = value.strip()
    except OSError:
        return None
    rss = fields.get("VmRSS", "").split()
    hwm = fields.get("VmHWM", "").split()
    voluntary = fields.get("voluntary_ctxt_switches", "0")
    involuntary = fields.get("nonvoluntary_ctxt_switches", "0")
    return {
        "rss": int(rss[0]) * 1024 if rss else 0,
        "hwm": int(hwm[0]) * 1024 if hwm else 0,
        "vol": int(voluntary) if voluntary.isdigit() else 0,
        "nonvol": int(involuntary) if involuntary.isdigit() else 0,
    }


def read_stat(pid):
    """Returns (utime, stime) in clock ticks; None once the process is gone."""
    try:
        with open(PROC / str(pid) / "stat", "rb") as handle:
            data = handle.read().decode("ascii", "replace")
    except OSError:
        return None
    close = data.rfind(")")
    if close < 0:
        return None
    fields = data[close + 2:].split()
    if len(fields) < 13:
        return None
    return int(fields[11]), int(fields[12])


def tree_pids(root):
    """Walks /proc children files to collect a process tree.

    XDM runs transfers on threads rather than child processes, so this is
    normally just the root. Kernels without CONFIG_PROC_CHILDREN degrade to the
    root only, which is correct for the processes this benchmark measures.
    """
    seen = set()
    stack = [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        try:
            threads = os.listdir(PROC / str(pid) / "task")
        except OSError:
            continue
        for tid in threads:
            try:
                with open(PROC / str(pid) / "task" / tid / "children", "r") as handle:
                    children = handle.read().split()
            except OSError:
                continue
            for child in children:
                child = int(child)
                if child not in seen:
                    stack.append(child)
    return seen


def snapshot(pid):
    """A single aggregate sample over the process tree of `pid`."""
    totals = {"rss": 0, "hwm": 0, "vol": 0, "nonvol": 0, "utime": 0, "stime": 0, "pids": 0}
    for member in tree_pids(pid):
        status = read_status(member)
        stat = read_stat(member)
        if status is None and stat is None:
            continue
        totals["pids"] += 1
        if status:
            totals["rss"] += status["rss"]
            totals["hwm"] += status["hwm"]
            totals["vol"] += status["vol"]
            totals["nonvol"] += status["nonvol"]
        if stat:
            totals["utime"] += stat[0]
            totals["stime"] += stat[1]
    return totals


class Sampler(threading.Thread):
    """Samples the process tree every `interval` seconds in the background."""

    def __init__(self, pid, interval):
        super().__init__(daemon=True)
        self.pid = pid
        self.interval = interval
        self.samples = []
        self._done = threading.Event()

    def run(self):
        while not self._done.is_set():
            self.samples.append(snapshot(self.pid))
            self._done.wait(self.interval)

    def stop(self):
        self._done.set()
        self.join(timeout=5)


class Measure:
    """Brackets a measurement window with exact start/end tree samples."""

    def __init__(self, pid, interval):
        self.pid = pid
        self.interval = interval
        self.samples = []

    def start(self):
        self.t0 = time.monotonic()
        self.samples = [snapshot(self.pid)]
        self.sampler = Sampler(self.pid, self.interval)
        self.sampler.start()
        return self

    def wait(self, seconds):
        time.sleep(seconds)

    def finish(self):
        self.sampler.stop()
        self.t1 = time.monotonic()
        self.samples.extend(self.sampler.samples)
        self.samples.append(snapshot(self.pid))
        return self

    @property
    def wall(self):
        return self.t1 - self.t0


def compute_metrics(samples, transferred, wall, usage=None):
    """RSS/CPU/throughput metrics for one scenario.

    `usage` is the ``os.wait4`` rusage of a short-lived child; it supplies exact
    CPU and context-switch counters that the 100 ms samples can miss in the
    final slice before a process exits. Peak/mean RSS always come from the
    sampled VmRSS/VmHWM: ``ru_maxrss`` is seeded from the parent Python process's
    RSS at fork and would overstate a small child's peak.
    """
    rss_values = [sample["rss"] for sample in samples]
    # VmHWM is the kernel's exact resident high-water mark; mean comes from the
    # VmRSS time series.
    peak_rss = max((sample["hwm"] for sample in samples), default=0)
    mean_rss = int(sum(rss_values) / len(rss_values)) if rss_values else 0
    first, last = samples[0], samples[-1]
    if usage is not None:
        cpu_seconds = usage.ru_utime + usage.ru_stime
        voluntary = usage.ru_nvcsw
        involuntary = usage.ru_nivcsw
    else:
        cpu_seconds = (
            (last["utime"] + last["stime"]) - (first["utime"] + first["stime"])
        ) / CLK_TCK
        voluntary = max(last["vol"] - first["vol"], 0)
        involuntary = max(last["nonvol"] - first["nonvol"], 0)
    gib = transferred / GIB
    return {
        "peak_rss_bytes": peak_rss,
        "mean_rss_bytes": mean_rss,
        "cpu_seconds": round(cpu_seconds, 4),
        "cpu_seconds_per_gib": round(cpu_seconds / gib, 5) if gib > 0 else None,
        "wall_seconds": round(wall, 4),
        "throughput_bytes_per_sec": round(transferred / wall, 1) if wall > 0 else None,
        "context_switches_voluntary": voluntary,
        "context_switches_nonvoluntary": involuntary,
        "samples": len(samples),
    }


# --------------------------------------------------------------------------- #
# Subprocess helpers
# --------------------------------------------------------------------------- #


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_child(command, env, cwd, interval):
    """Runs a short-lived process, sampling it, and returns exit/rusage data."""
    stdout_file = tempfile.TemporaryFile()
    stderr_file = tempfile.TemporaryFile()
    start = time.monotonic()
    proc = subprocess.Popen(command, stdout=stdout_file, stderr=stderr_file, env=env, cwd=cwd)
    first = snapshot(proc.pid)
    sampler = Sampler(proc.pid, interval)
    sampler.start()
    _, status, usage = os.wait4(proc.pid, 0)
    returncode = os.waitstatus_to_exitcode(status)
    end = time.monotonic()
    sampler.stop()
    samples = [first] + sampler.samples
    stdout_file.seek(0)
    stdout = stdout_file.read().decode("utf-8", "replace")
    stdout_file.close()
    stderr_file.seek(0)
    stderr = stderr_file.read().decode("utf-8", "replace")
    stderr_file.close()
    return {
        "wall": end - start,
        "returncode": returncode,
        "usage": usage,
        "samples": samples,
        "stdout": stdout,
        "stderr": stderr,
    }


# --------------------------------------------------------------------------- #
# Fixture server
# --------------------------------------------------------------------------- #


class Fixture:
    def __init__(self, script, size_mib, env):
        self.size_mib = size_mib
        self.proc = subprocess.Popen(
            [sys.executable, str(script), "--size-mib", str(size_mib)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            text=True,
        )
        line = self.proc.stdout.readline()
        if not line:
            stderr = self.proc.stderr.read()
            raise SystemExit(f"fixture failed to start: {stderr.strip()}")
        info = json.loads(line)
        self.port = info["port"]
        self.size = info["size"]
        self.sha256 = info["sha256"]
        self.url = f"http://127.0.0.1:{self.port}/body.bin"
        log(f"fixture {size_mib} MiB on 127.0.0.1:{self.port}")

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)


# --------------------------------------------------------------------------- #
# Daemon IPC
# --------------------------------------------------------------------------- #


def recv_exact(stream, count):
    chunks = []
    remaining = count
    while remaining:
        chunk = stream.recv(remaining)
        if not chunk:
            raise OSError("daemon closed the connection")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def ipc_request(socket_path, token, command, timeout=120):
    payload = json.dumps({"version": 1, "token": token, "command": command}).encode()
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(timeout)
        stream.connect(socket_path)
        stream.sendall(struct.pack("<I", len(payload)) + payload)
        header = recv_exact(stream, 4)
        length = struct.unpack("<I", header)[0]
        return json.loads(recv_exact(stream, length))


class Daemon:
    def __init__(self, proc, socket_path, token, stderr_handle, stderr_path):
        self.proc = proc
        self.socket = socket_path
        self.token = token
        self.stderr_handle = stderr_handle
        self.stderr_path = stderr_path

    def command(self, command, timeout=120):
        return ipc_request(self.socket, self.token, command, timeout)

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)
        self.stderr_handle.close()


def start_daemon(repo, env, data_dir, home_dir):
    data_dir.mkdir(parents=True, exist_ok=True)
    home_dir.mkdir(parents=True, exist_ok=True)
    child_env = env.copy()
    child_env["XDM_DATA_DIR"] = str(data_dir)
    child_env["HOME"] = str(home_dir)
    # Belt and braces: a completion action must never reach the real OS.
    child_env["XDM_FAKE_OS_ACTIONS"] = "1"
    stderr_path = data_dir.parent / "daemon.stderr"
    stderr_handle = open(stderr_path, "wb")
    proc = subprocess.Popen(
        [str(repo / "target" / "release" / "xdm-daemon")],
        env=child_env,
        stdout=subprocess.DEVNULL,
        stderr=stderr_handle,
    )
    socket_path = str(data_dir / "daemon.sock")
    token_path = data_dir / "ipc-token"
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            stderr_handle.close()
            raise RuntimeError(
                f"daemon exited early: {stderr_path.read_text(errors='replace')}"
            )
        if os.path.exists(socket_path) and token_path.exists():
            try:
                token = token_path.read_text().strip()
                ipc_request(socket_path, token, {"type": "list"}, timeout=2)
                return Daemon(proc, socket_path, token, stderr_handle, stderr_path)
            except (OSError, ValueError, json.JSONDecodeError):
                pass
        time.sleep(0.05)
    proc.terminate()
    proc.wait(timeout=10)
    stderr_handle.close()
    raise RuntimeError("daemon did not become ready in time")


# --------------------------------------------------------------------------- #
# Scenario bookkeeping
# --------------------------------------------------------------------------- #


def make_result(name, detail, transferred, metrics, output, expected_sha256):
    actual = sha256_file(output) if output is not None else None
    return {
        "name": name,
        "detail": detail,
        "bytes": transferred,
        "metrics": metrics,
        "output_sha256": actual,
        "expected_sha256": expected_sha256,
        # None when the scenario produces no artifact to verify.
        "hash_ok": (actual == expected_sha256) if expected_sha256 is not None else None,
    }


def cli_download_scenarios(args, fixture, repo, env, output_dir, results):
    cli = str(repo / "target" / "release" / "xdm-cli")
    for connections in (1, 4, 8, 16):
        name = f"cli_{args.size_mib}MiB_c{connections}"
        output = output_dir / f"{name}.bin"
        command = [
            cli,
            "--no-proxy",
            fixture.url,
            str(output),
            "--connections",
            str(connections),
        ]
        run = run_child(command, env, repo, args.interval)
        if run["returncode"] != 0:
            raise RuntimeError(f"{name} failed: {run['stderr'].strip()}")
        results.append(
            make_result(
                name,
                {"connections": connections, "rate_kib": None},
                fixture.size,
                compute_metrics(run["samples"], fixture.size, run["wall"], run["usage"]),
                output,
                fixture.sha256,
            )
        )
        log(f"{name}: {run['wall']:.2f}s")
        output.unlink(missing_ok=True)


def cli_rate_scenario(args, fixture, repo, env, output_dir, results):
    cli = str(repo / "target" / "release" / "xdm-cli")
    name = f"cli_{args.size_mib}MiB_c4_rate{args.rate_kib}KiB"
    output = output_dir / f"{name}.bin"
    command = [
        cli,
        "--no-proxy",
        fixture.url,
        str(output),
        "--connections",
        "4",
        "--rate-kib",
        str(args.rate_kib),
    ]
    run = run_child(command, env, repo, args.interval)
    if run["returncode"] != 0:
        raise RuntimeError(f"{name} failed: {run['stderr'].strip()}")
    results.append(
        make_result(
            name,
            {"connections": 4, "rate_kib": args.rate_kib},
            fixture.size,
            compute_metrics(run["samples"], fixture.size, run["wall"], run["usage"]),
            output,
            fixture.sha256,
        )
    )
    log(f"{name}: {run['wall']:.2f}s")
    output.unlink(missing_ok=True)


def daemon_idle_scenarios(args, fixture, repo, env, results):
    work = Path(tempfile.mkdtemp(prefix="daemon-idle-", dir=args.tmp))
    downloads = work / "downloads"
    downloads.mkdir(parents=True)
    daemon = start_daemon(repo, env, work / "state", work / "home")
    try:
        measure = Measure(daemon.proc.pid, args.interval).start()
        measure.wait(args.idle_seconds)
        measure.finish()
        results.append(
            make_result(
                "daemon_idle_0",
                {"tasks": 0, "idle_seconds": args.idle_seconds},
                0,
                compute_metrics(measure.samples, 0, measure.wall),
                None,
                None,
            )
        )

        stopped = daemon.command({"type": "queue_stop"})
        if not stopped.get("ok"):
            raise RuntimeError(f"queue_stop failed: {stopped}")
        for index in range(args.queued_tasks):
            reply = daemon.command(
                {
                    "type": "submit",
                    "payload": {
                        "request_id": f"idle-{index}",
                        "url": fixture.url,
                        "output": str(downloads / f"idle-{index}.bin"),
                        "headers": [],
                        "priority": 1,
                        "connections": 1,
                    },
                }
            )
            if not reply.get("ok"):
                raise RuntimeError(f"submit {index} failed: {reply}")
        # Let the queued submissions and scheduler wakeups settle before sampling.
        time.sleep(0.5)
        measure = Measure(daemon.proc.pid, args.interval).start()
        measure.wait(args.idle_seconds)
        measure.finish()
        results.append(
            make_result(
                f"daemon_idle_{args.queued_tasks}",
                {"tasks": args.queued_tasks, "idle_seconds": args.idle_seconds, "stopped": True},
                0,
                compute_metrics(measure.samples, 0, measure.wall),
                None,
                None,
            )
        )
        log(
            f"daemon idle: 0 tasks and {args.queued_tasks} queued tasks over "
            f"{args.idle_seconds}s each"
        )
    finally:
        daemon.close()
        shutil.rmtree(work, ignore_errors=True)


def daemon_concurrent_scenario(args, repo, env, results):
    size_mib = args.concurrent_mib
    fixture = Fixture(Path(__file__).parent / "fixture_server.py", size_mib, env)
    work = Path(tempfile.mkdtemp(prefix="daemon-concurrent-", dir=args.tmp))
    downloads = work / "downloads"
    downloads.mkdir(parents=True)
    daemon = None
    try:
        daemon = start_daemon(repo, env, work / "state", work / "home")
        concurrency = 4
        limit = daemon.command(
            {"type": "set_concurrency", "payload": {"limit": concurrency}}
        )
        if not limit.get("ok"):
            raise RuntimeError(f"set_concurrency failed: {limit}")
        outputs = []
        for index in range(concurrency):
            output = downloads / f"concurrent-{index}.bin"
            reply = daemon.command(
                {
                    "type": "submit",
                    "payload": {
                        "request_id": f"concurrent-{index}",
                        "url": fixture.url,
                        "output": str(output),
                        "headers": [],
                        "priority": 1,
                        "connections": 4,
                    },
                }
            )
            if not reply.get("ok"):
                raise RuntimeError(f"submit {index} failed: {reply}")
            outputs.append(output)

        measure = Measure(daemon.proc.pid, args.interval).start()
        deadline = time.monotonic() + max(120, size_mib)
        while time.monotonic() < deadline:
            tasks = daemon.command({"type": "list"})["tasks"]
            states = [task["status"] for task in tasks]
            if any(state == "failed" for state in states):
                raise RuntimeError(f"a download failed: {tasks}")
            if len(tasks) >= concurrency and all(
                state == "complete" for state in states
            ):
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("concurrent downloads did not finish in time")
        measure.finish()

        transferred = fixture.size * concurrency
        metrics = compute_metrics(measure.samples, transferred, measure.wall)
        actual_hashes = {output.name: sha256_file(output) for output in outputs}
        hash_ok = all(value == fixture.sha256 for value in actual_hashes.values())
        result = make_result(
            f"daemon_concurrent_{concurrency}x{size_mib}MiB",
            {
                "concurrency": concurrency,
                "per_task_bytes": fixture.size,
                "connections": 4,
            },
            transferred,
            metrics,
            outputs[0],
            fixture.sha256,
        )
        result["hash_ok"] = hash_ok
        result["hashes"] = actual_hashes
        results.append(result)
        log(
            f"daemon {concurrency}x{size_mib}MiB: {measure.wall:.2f}s, hash "
            f"{'ok' if hash_ok else 'MISMATCH'}"
        )
    finally:
        if daemon is not None:
            daemon.close()
        fixture.close()
        shutil.rmtree(work, ignore_errors=True)


# --------------------------------------------------------------------------- #
# Reporting
# --------------------------------------------------------------------------- #


def write_results(results, args, out_dir):
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    path = out_dir / f"{stamp}.json"
    counter = 1
    while path.exists():
        path = out_dir / f"{stamp}-{counter}.json"
        counter += 1
    document = {
        "version": 1,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "quick": args.quick,
        "interval_seconds": args.interval,
        "config": {
            "size_mib": args.size_mib,
            "concurrent_mib": args.concurrent_mib,
            "idle_seconds": args.idle_seconds,
            "queued_tasks": args.queued_tasks,
            "rate_kib": args.rate_kib,
        },
        "scenarios": results,
    }
    path.write_text(json.dumps(document, indent=2) + "\n")
    return path


def render_table(results):
    lines = [
        "| scenario | bytes (MiB) | wall (s) | throughput (MiB/s) | peak RSS (MiB) | mean RSS (MiB) | CPU (s) | CPU s/GiB | ctx vol | ctx nonvol | hash |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for result in results:
        metrics = result["metrics"]
        transferred = result["bytes"]
        throughput = metrics["throughput_bytes_per_sec"]
        cpu_per_gib = metrics["cpu_seconds_per_gib"]
        if result["hash_ok"] is None:
            hash_cell = "-"
        else:
            hash_cell = "ok" if result["hash_ok"] else "MISMATCH"
        lines.append(
            "| {name} | {mib:.1f} | {wall:.2f} | {throughput} | {peak:.1f} | {mean:.1f} | {cpu:.3f} | {cpu_gib} | {vol} | {nonvol} | {hash} |".format(
                name=result["name"],
                mib=transferred / MIB,
                wall=metrics["wall_seconds"],
                throughput=f"{throughput / MIB:.1f}" if throughput else "-",
                peak=metrics["peak_rss_bytes"] / MIB,
                mean=metrics["mean_rss_bytes"] / MIB,
                cpu=metrics["cpu_seconds"],
                cpu_gib=f"{cpu_per_gib:.3f}" if cpu_per_gib is not None else "-",
                vol=metrics["context_switches_voluntary"],
                nonvol=metrics["context_switches_nonvoluntary"],
                hash=hash_cell,
            )
        )
    return "\n".join(lines)


REGRESSION_DIRECTIONS = {
    "peak_rss_bytes": "lower",
    "mean_rss_bytes": "lower",
    "cpu_seconds": "lower",
    "cpu_seconds_per_gib": "lower",
    "wall_seconds": "lower",
    "throughput_bytes_per_sec": "higher",
}


def compare(old_path, new_path, threshold):
    """Prints regressions beyond `threshold` percent and returns an exit code."""
    old = json.loads(Path(old_path).read_text())
    new = json.loads(Path(new_path).read_text())
    old_map = {scenario["name"]: scenario for scenario in old["scenarios"]}
    new_map = {scenario["name"]: scenario for scenario in new["scenarios"]}
    regressions = []
    missing = []
    checked = 0
    print(f"compare {old_path} -> {new_path} (threshold {threshold:g}%)")
    print("| scenario | metric | old | new | change | verdict |")
    print("| --- | --- | --- | --- | --- | --- |")
    for name in sorted(new_map):
        if name not in old_map:
            missing.append(name)
            continue
        old_metrics = old_map[name]["metrics"]
        new_metrics = new_map[name]["metrics"]
        for metric, direction in REGRESSION_DIRECTIONS.items():
            old_value = old_metrics.get(metric)
            new_value = new_metrics.get(metric)
            if old_value in (None, 0) or new_value is None:
                continue
            checked += 1
            change = (new_value - old_value) / abs(old_value) * 100
            if direction == "lower":
                regressed = change > threshold
            else:
                regressed = -change > threshold
            if regressed:
                regressions.append((name, metric, old_value, new_value, change))
                print(
                    f"| {name} | {metric} | {old_value:g} | {new_value:g} | "
                    f"{change:+.2f}% | REGRESSION |"
                )
    for name in sorted(old_map):
        if name not in new_map:
            missing.append(name)
    if not regressions:
        print(f"no regressions beyond {threshold:g}% ({checked} metric comparisons)")
    else:
        print(f"{len(regressions)} regression(s) beyond {threshold:g}%")
    if missing:
        print("scenarios only present in one file (not compared): " + ", ".join(sorted(set(missing))))
    return 1 if regressions else 0


# --------------------------------------------------------------------------- #
# Self-test
# --------------------------------------------------------------------------- #


def self_test():
    sys.path.insert(0, str(Path(__file__).parent))
    import fixture_server

    assert fixture_server.parse_range(None, 100) is None
    assert fixture_server.parse_range("bytes=0-", 100) == (0, 99)
    assert fixture_server.parse_range("bytes=10-20", 100) == (10, 20)
    assert fixture_server.parse_range("bytes=90-200", 100) == (90, 99)
    assert fixture_server.parse_range("bytes=100-", 100) == "unsatisfiable"
    assert fixture_server.parse_range("garbage", 100) is None
    expected = hashlib.sha256(fixture_server.BLOCK[:1000]).hexdigest()
    assert fixture_server.expected_sha256(1000) == expected

    # The tree of this process must include at least itself with a live RSS.
    sample = snapshot(os.getpid())
    assert sample["pids"] >= 1 and sample["rss"] > 0, sample

    # Metrics honour an explicit rusage for exact CPU accounting.
    usage = type(
        "Usage", (), {"ru_utime": 1.0, "ru_stime": 0.5, "ru_maxrss": 4096, "ru_nvcsw": 7, "ru_nivcsw": 3}
    )()
    metrics = compute_metrics([{"rss": MIB, "hwm": 2 * MIB, "vol": 0, "nonvol": 0, "utime": 0, "stime": 0}], GIB, 2.0, usage)
    assert metrics["cpu_seconds"] == 1.5
    assert metrics["cpu_seconds_per_gib"] == 1.5
    assert metrics["context_switches_voluntary"] == 7
    assert metrics["peak_rss_bytes"] == 2 * MIB  # from the sampled VmHWM
    assert metrics["throughput_bytes_per_sec"] == GIB / 2

    # Framed IPC against a throwaway Unix socket.
    tmp = tempfile.mkdtemp(prefix="xdm-bench-selftest-")
    path = os.path.join(tmp, "sock")

    def server():
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(path)
            listener.listen(1)
            conn, _ = listener.accept()
            with conn:
                header = recv_exact(conn, 4)
                length = struct.unpack("<I", header)[0]
                request = json.loads(recv_exact(conn, length))
                reply = json.dumps({"echo": request["command"]}).encode()
                conn.sendall(struct.pack("<I", len(reply)) + reply)

    thread = threading.Thread(target=server)
    thread.start()
    echoed = None
    for _ in range(200):
        try:
            echoed = ipc_request(path, "token", {"type": "list"}, timeout=5)
            break
        except OSError:
            time.sleep(0.01)
    thread.join(timeout=5)
    shutil.rmtree(tmp, ignore_errors=True)
    assert echoed == {"echo": {"type": "list"}}, echoed
    print("self-test passed")
    return 0


# --------------------------------------------------------------------------- #
# Entrypoint
# --------------------------------------------------------------------------- #


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--quick", action="store_true", help="shrink sizes and idle windows")
    parser.add_argument("--no-build", action="store_true", help="use existing release binaries")
    parser.add_argument("--compare", nargs=2, metavar=("OLD", "NEW"), help="compare two result files")
    parser.add_argument("--threshold", type=float, default=10.0, help="regression threshold percent")
    parser.add_argument("--out-dir", type=Path, default=Path(__file__).parent / "results")
    parser.add_argument("--tmp", type=Path, default=Path("/tmp/opencode"))
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--interval", type=float, default=INTERVAL_SECONDS, help="sample interval seconds")
    parser.add_argument("--size-mib", type=int, default=None)
    parser.add_argument("--concurrent-mib", type=int, default=None)
    parser.add_argument("--idle-seconds", type=int, default=None)
    parser.add_argument("--queued-tasks", type=int, default=None)
    parser.add_argument("--rate-kib", type=int, default=None)
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    if args.self_test:
        return self_test()
    if args.compare:
        return compare(args.compare[0], args.compare[1], args.threshold)

    args.size_mib = args.size_mib or (QUICK_SIZE_MIB if args.quick else DEFAULT_SIZE_MIB)
    args.concurrent_mib = args.concurrent_mib or (
        QUICK_CONCURRENT_MIB if args.quick else DEFAULT_CONCURRENT_MIB
    )
    args.idle_seconds = args.idle_seconds or (
        QUICK_IDLE_SECONDS if args.quick else DEFAULT_IDLE_SECONDS
    )
    args.queued_tasks = args.queued_tasks or (
        QUICK_QUEUED_TASKS if args.quick else DEFAULT_QUEUED_TASKS
    )
    args.rate_kib = args.rate_kib or (QUICK_RATE_KIB if args.quick else DEFAULT_RATE_KIB)

    args.tmp.mkdir(parents=True, exist_ok=True)
    env = load_env()
    if not args.no_build:
        build_binaries(args.repo, env)

    work = Path(tempfile.mkdtemp(prefix="xdm-bench-", dir=args.tmp))
    fixture = None
    try:
        fixture = Fixture(Path(__file__).parent / "fixture_server.py", args.size_mib, env)
        results = []
        cli_download_scenarios(args, fixture, args.repo, env, work, results)
        cli_rate_scenario(args, fixture, args.repo, env, work, results)
        daemon_idle_scenarios(args, fixture, args.repo, env, results)
        daemon_concurrent_scenario(args, args.repo, env, results)
        fixture.close()
        fixture = None
        path = write_results(results, args, args.out_dir)
        print(render_table(results))
        print()
        print(f"results: {path}")
        return 0
    finally:
        if fixture is not None:
            fixture.close()
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
