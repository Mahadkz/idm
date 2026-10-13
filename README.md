# XDM

An independent, open-source download manager in active development. Linux and Windows are the first targets; macOS is planned later. The supplied IDM materials were used for a bounded functional audit in [`docs/idm_audit_completion.md`](docs/idm_audit_completion.md); XDM is a fresh implementation.

The new application source is licensed under [GPL-3.0-only](LICENSE). The native GUI prototype uses Slint under its GPLv3 option. Proprietary reference materials and audit artifacts are not part of this license grant.

The Rust workspace now contains a standalone HTTP engine, persistent queue daemon, local native-messaging host, Chromium extension prototype, clear HLS/DASH media CLI, and an early native desktop GUI. The engine and daemon are independent of the UI toolkit. The architecture and contracts are in [`docs/architecture_spec.md`](docs/architecture_spec.md). The [feature coverage ledger](docs/feature_coverage.md) accounts for every supplied dialog/control/menu identity; the [dynamic behavior review](docs/idm_behavior_review.md) covers all 1,604 language entries and records detailed behavior requirements. Runtime callers and behavior equivalence remain separately tracked.

The [mapping status and implementation order](docs/feature_mapping_status.md) links the individual desktop, browser/media, engine and scheduler reviews: 170 detailed contracts, with evidence, current differences and acceptance scenarios.

Build and run on Linux with a recent stable Rust toolchain. The GUI build needs `pkg-config`, Fontconfig development files, and FreeType development files (on Ubuntu: `pkgconf`, `libfontconfig-dev`, `libfreetype-dev`):

```sh
cargo build --workspace
cargo run -p xdm-daemon
cargo run -p xdm-gui
```

The daemon must be running for the GUI, native host, and `xdm-cli daemon` commands. `XDM_DATA_DIR` can isolate its token, socket, and SQLite database during development.

```sh
cargo run -p xdm-cli -- URL OUTPUT_PATH
cargo run -p xdm-cli -- URL OUTPUT_PATH --connections 8
cargo run -p xdm-cli -- URL OUTPUT_PATH --header 'User-Agent: my-agent' --header 'Cookie: session=example'
cargo run -p xdm-cli -- URL OUTPUT_PATH --rate-kib 512
cargo run -p xdm-cli -- URL OUTPUT_PATH --restart
```

Press Ctrl-C to pause. Run the same command again to resume a range-capable transfer. `--restart` discards that task's `.part` and `.part.meta` files and downloads from byte zero. The engine rejects changed validators rather than mixing versions. Supplied headers are scoped to the original URL origin; a cross-origin redirect stops instead of forwarding them. The CLI accepts custom headers but does not yet use a protected credential store.

Queue and clear-stream commands:

```sh
cargo run -p xdm-cli -- daemon submit URL OUTPUT_PATH
cargo run -p xdm-cli -- daemon list
cargo run -p xdm-cli -- daemon preview urls.txt
cargo run -p xdm-cli -- daemon import urls.txt /absolute/download/folder
cargo run -p xdm-cli -- daemon import tasks.json /absolute/download/folder --start
cargo run -p xdm-cli -- daemon export tasks.json json
cargo run -p xdm-cli -- daemon export urls.txt urls
cargo run -p xdm-cli -- daemon info TASK_ID
cargo run -p xdm-cli -- daemon redownload TASK_ID NEW_ABSOLUTE_OUTPUT_PATH
cargo run -p xdm-cli -- daemon remove TASK_ID
cargo run -p xdm-cli -- daemon pause TASK_ID
cargo run -p xdm-cli -- daemon resume TASK_ID
cargo run -p xdm-cli -- daemon limit 3
cargo run -p xdm-cli -- daemon priority TASK_ID immediate
cargo run -p xdm-cli -- daemon rate TASK_ID 512
cargo run -p xdm-cli -- daemon rate TASK_ID unlimited
cargo run -p xdm-cli -- daemon queue create "Video queue" 1
cargo run -p xdm-cli -- daemon queue assign QUEUE_ID TASK_ID
cargo run -p xdm-cli -- daemon queue enable QUEUE_ID
cargo run -p xdm-cli -- daemon queue disable QUEUE_ID
cargo run -p xdm-cli -- daemon queue stop
cargo run -p xdm-cli -- daemon queue at 1790000000
cargo run -p xdm-cli -- daemon queue stop-at 1790003600
cargo run -p xdm-cli -- daemon queue start
cargo run -p xdm-cli -- media inspect 'https://example.com/master.m3u8'
cargo run -p xdm-cli -- media download 'https://example.com/master.m3u8' video.mp4 VIDEO_INDEX AUDIO_INDEX
cargo run -p xdm-cli -- media queue 'https://example.com/master.m3u8' video.mp4 VIDEO_INDEX AUDIO_INDEX
```

`daemon redownload` creates a new task from the selected task at a fresh absolute destination, retaining its request headers, media selection, priority, connections, category, and rate cap. The earlier output stays in place. Pause active tasks before redownloading or removing them. `daemon remove` removes the task record while retaining output files and checkpoints; a removed browser submission ID is rejected on later replay. `daemon info` returns task configuration and error details without stored request headers. The GUI exposes these actions with destination and removal dialogs.

URL/JSON import and export are available in the CLI and the desktop **Import / Export…** dialog. Import defaults to paused tasks; `--start` puts them in the queue. Each batch commits atomically and rejects destination conflicts. Exports omit request headers and URL usernames/passwords; URL query parameters remain. See [portable download lists](docs/transfer_lists.md) for the schema and current 500-entry/512 KiB limits.

Media commands require `ffprobe` and `ffmpeg` on `PATH`. They currently handle public, clear HLS/DASH manifests, enumerate tracks, and stream-copy selected video/audio into MP4 when codecs permit it. The Chromium popup can inspect detected manifests and queue a selected video/audio pair through the native host. Encrypted HLS and DASH manifests with ContentProtection are refused. Authentication replay, live capture, transcoding, and DRM support are absent.

Named queues persist their name, running state and job limit. New queues start stopped; existing/browser/imported tasks start in Main, and redownload retains queue membership. The GUI Queues dialog creates/renames queues, changes limits, moves a selected stopped task, and filters the task list. `queue enable` starts an individual queue only while the global queue is running; `queue start`/`stop` control that global gate. Limits apply per queue under the global job cap. Reducing a limit lets existing transfers finish; it prevents excess new starts.

`daemon queue at` and `stop-at` take Unix timestamps. One-shot queue start/stop times persist across daemon restarts. A queue stop interrupts active tasks and keeps their checkpoints for a later queue start. Priority can be `immediate`, `normal`, or `low` for queued, paused, or failed tasks. Browser submissions are sorted into built-in Video, Music, Documents, Compressed, and Programs folders by filename or MIME type; unrecognized files stay in Downloads. The desktop GUI offers category filters, search, priority buttons, manual queue start/stop, and local-time one-shot start/stop controls. A queue stop affects the transfer queue only; it does not shut down the computer.

For browser development, load [`extensions/chromium`](extensions/chromium) unpacked. Interception is opt-in in its popup. Register the local host with [`install/linux/register_native_host.py`](install/linux/register_native_host.py) or [`install/windows/Register-NativeHost.ps1`](install/windows/Register-NativeHost.ps1), supplying the installed extension ID and native-host executable path. On Linux, a real Chrome for Testing session with an isolated profile has passed accepted-download handoff and daemon-unavailable browser fallback. Installed Google Chrome, Edge, and Brave user profiles and native Windows browser integration still need validation.

## Installation

`packaging/linux/build-tarball.sh` builds the release binaries offline and assembles `target/dist/xdm-<version>-<arch>-linux.tar.gz` containing `bin/`, `extensions/`, `install/`, and `LICENSE`. On Linux, extract the tarball (or use a checkout with built `target/release` binaries) and run the user-level installer; it needs no root and never touches system directories:

```sh
./install/linux/install.sh                  # ~/.local/bin, desktop entry, icon, systemd user unit
./install/linux/install.sh --autostart      # additionally enable and start xdm-daemon.service
./install/linux/install.sh --dry-run        # print planned actions only
./install/linux/uninstall.sh                # remove exactly what install created
./install/linux/uninstall.sh --purge        # also delete ~/.local/share/xdm (never your downloads)
```

The installer records every created path in `~/.local/state/xdm/install-manifest`, which the uninstaller walks, so re-running install is idempotent and uninstall removes nothing it did not create. Browser integration is registered through [`install/linux/register_native_host.py`](install/linux/register_native_host.py) and [`install/linux/register_firefox_native_host.py`](install/linux/register_firefox_native_host.py) for Chrome, Chromium, Edge, Brave, and Firefox; pass `--extension-id` for the unpacked Chromium extension or let the installer derive it from `extensions/chromium`. `XDM_NO_SYSTEMD=1` makes both scripts skip `systemctl` entirely (used by `packaging/linux/test-install.sh`, which exercises install and uninstall against a disposable `HOME`).

On Windows, run `install/windows/Install-XDM.ps1` from a PowerShell prompt. It installs per-user into `%LOCALAPPDATA%\Programs\XDM`, creates a Start-menu shortcut, optionally adds an `HKCU\...\Run` entry with `-Autostart`, and registers the native messaging host for Chrome/Chromium/Edge and Firefox via [`install/windows/Register-NativeHost.ps1`](install/windows/Register-NativeHost.ps1) and [`install/windows/Register-FirefoxNativeHost.ps1`](install/windows/Register-FirefoxNativeHost.ps1). `Uninstall-XDM.ps1` reverses the recorded manifest; both scripts support `-WhatIf` and `-Purge` follows the same data-directory rules as Linux.

Verification:

```sh
cargo test --workspace
python3 tests/integration/cli_http.py
python3 tests/integration/daemon_ipc.py
python3 tests/integration/media_clear.py
python3 tests/integration/scheduler.py
python3 tests/integration/named_queues.py
python3 tests/integration/task_rate.py
python3 tests/integration/task_actions.py
python3 tests/integration/transfer_lists.py
python3 tests/integration/gui_actions.py
node tests/integration/extension_handoff.mjs
node tests/integration/firefox_handoff.mjs
XDM_CHROME_FOR_TESTING=/path/to/chrome node tests/integration/chromium_real.mjs
```

The GUI interaction test requires `Xvfb` and `xdotool`. It allocates its own virtual display, uses an isolated daemon database, and closes its processes afterwards. It does not use the current desktop display.

The native GUI is an early queue client with live HTTP byte counts, speed, estimated remaining time, and progress bars. HTTP task rate caps can be changed during a transfer from the GUI or CLI and persist across daemon restarts. Clear-media tasks still use the FFmpeg pipeline without this rate control. The daemon's concurrent-job limit is configurable through the CLI and persists in SQLite. Recurring schedules, completion power actions, custom categories, GUI settings, global bandwidth cap, dynamic segment reallocation, and authenticated browser media remain future work. See the [feature implementation status](docs/implementation_status.md) for the audited IDM comparison. Windows cross-build support requires the `x86_64-pc-windows-gnu` Rust target and MinGW linker. The Windows daemon, named-pipe IPC, and an 8 MiB download passed a Wine smoke test; native Windows browser and GUI interaction validation remain necessary.
