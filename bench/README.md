# XDM resource benchmark

Reproducible RAM/CPU harness for the download path. Python 3 stdlib only.

## Prerequisites

The offline cargo toolchain lives in `/tmp/xdm-work/env.sh`; source it before
running so `cargo` and its native build dependencies are on `PATH`:

```sh
. /tmp/xdm-work/env.sh
python3 bench/run_bench.py            # full: 256 MiB, 1000 queued, 30 s idle
python3 bench/run_bench.py --quick    # 32 MiB, 200 queued, 5 s idle
python3 bench/run_bench.py --no-build # reuse target/release binaries
python3 bench/run_bench.py --self-test
```

`run_bench.py` itself sources `env.sh` through `bash` when it builds, so the
`cargo build` step works even from a shell that has not sourced it; sourcing by
hand is only needed if you invoke `cargo`/the binaries yourself.

## What it measures

It starts `bench/fixture_server.py` (a threaded, deterministic, Range-capable
HTTP body on `127.0.0.1`, no artificial delay) and runs:

- `cli_<N>MiB_c1|c4|c8|c16` — `xdm-cli` download with 1/4/8/16 connections.
- `cli_<N>MiB_c4_rate<K>KiB` — the same throttled with `--rate-kib`.
- `daemon_idle_0` / `daemon_idle_<K>` — a daemon idle for the window with no
  tasks and with K tasks submitted over IPC while the queue is stopped.
- `daemon_concurrent_4x<M>MiB` — 4 concurrent downloads through the daemon.

Every scenario samples `/proc/<pid>/status` (VmRSS, VmHWM, context switches)
and `/proc/<pid>/stat` (utime+stime) of the process tree every 100 ms and
reports peak/mean RSS, CPU seconds, CPU seconds per GiB, wall time, throughput
and voluntary/nonvoluntary context switches. Output hashes are verified against
the fixture's SHA-256.

Results are written to `bench/results/<timestamp>.json` and printed as a
markdown table.

## Using it as a performance gate

```sh
python3 bench/run_bench.py --compare old.json new.json [--threshold 10]
```

Prints every metric that regressed by more than the threshold (lower is better
for RSS/CPU/wall, higher is better for throughput) and exits non-zero when any
regression is found.

## Isolation

All state lives in a disposable `mkdtemp` tree under `--tmp` (default
`/tmp/opencode`). The daemon runs with `XDM_DATA_DIR`/`HOME` pointed at that
tree and with `XDM_FAKE_OS_ACTIONS=1`, so the real user data directory and the
OS are never touched.

## Interpreting wall time

`wall_seconds` is measured by the harness around the whole process, including spawn and sampling start-up, so throttled scenarios read roughly 0.5–0.9 s longer than the configured cap implies. Judge rate-limit accuracy from the process's own runtime (a 256 MiB download at a 32 MiB/s cap measured 8.03 s end to end on 2026-10-13), not from the throttled scenario's `throughput` column.
