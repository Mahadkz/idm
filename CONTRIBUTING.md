# Contributing to XDM

XDM is an open-source download manager written in Rust, with a Slint desktop window and Chromium and Firefox extensions. Read [`docs/ENGINEERING_RULES.md`](docs/ENGINEERING_RULES.md) before you change code. Its rules are binding: they cover safe file writes, no shell execution, secret handling, IPC compatibility, Windows portability, and how tests must be written.

## Build

### Linux

You need a recent stable Rust toolchain. The GUI also needs `pkg-config`, Fontconfig development files and FreeType development files. On Ubuntu:

```sh
sudo apt-get install pkgconf libfontconfig-dev libfreetype-dev
```

Without root access, run `tools/dev-setup.sh`. It extracts those headers and a MinGW cross-compiler for Windows builds into `~/.local/share/xdm-toolchain` and writes `env.sh` there. Source it before building:

```sh
. ~/.local/share/xdm-toolchain/env.sh
```

Then build the workspace:

```sh
cargo build --workspace
```

The binaries are `xdm-daemon`, `xdm-gui`, `xdm-cli`, `xdm-native-host`.

### Windows cross build

After `tools/dev-setup.sh`:

```sh
PATH="$XDM_CROSS_BIN:$PATH" cargo build --workspace --target x86_64-pc-windows-gnu
```

Every change must build for `x86_64-pc-windows-gnu` (rule 8 of the engineering rules).

### Packages

`packaging/release.sh` builds every release artifact into `target/dist`: the AppImage, the `.deb`, the Windows installer, the tarball, the browser extension packages and `SHA256SUMS`. `packaging/extensions/build.sh` builds only the extensions.

## Run locally

- Start the daemon with `cargo run -p xdm-daemon`, then the window with `cargo run -p xdm-gui`.
- `XDM_DATA_DIR` points the daemon at a separate data directory, which keeps development databases away from your real ones.
- Extension development: load `extensions/chromium` unpacked, or load `extensions/firefox` as a temporary add-on.

## Test

### Unit tests

```sh
cargo test --workspace
```

### Integration suites

After `cargo build --workspace`:

```sh
python3 tests/run_all.py
```

The runner executes every suite in `tests/integration` with a per-suite timeout. Options:

- `--list` shows the suites.
- `--only NAME ...` and `--skip NAME ...` select suites by name or file stem.
- `--timeout SECONDS` sets the per-suite timeout (default 600).
- `--strict` turns a suite skipped for a missing prerequisite into a failure. CI uses it.

A suite with a missing prerequisite is reported as SKIP with the reason, never as a pass. The prerequisites are `ffmpeg` and `ffprobe` for media suites, `Xvfb` and `xdotool` for the GUI suites, `node` for the extension suites, and `XDM_CHROME_FOR_TESTING` for the real-browser suite.

The Node suites run as `node tests/integration/extension_handoff.mjs` and `node tests/integration/firefox_handoff.mjs`. CI uses Node 22.

### GUI tests

GUI suites run only on their own virtual display. `tests/integration/gui_actions.py` starts `Xvfb` with `-displayfd` and drives the window with `xdotool`. Nothing is launched on your desktop. The GUI smoke test is `tests/gui_smoke.py`. On Linux, install `xvfb`, `xdotool` and `python3-pil`. Rule 14 of the engineering rules requires `SLINT_BACKEND=winit-software` for GUI tests.

### CI

The workflows are in `.github/workflows`:

- `linux.yml` builds with `cargo build --workspace --all-targets`, runs `cargo test --workspace`, runs `tests/run_all.py --strict --skip chromium_real`, and cross-builds for Windows.
- `windows.yml` builds, tests, runs the suites that can run there, and runs the GUI smoke test.
- `browsers.yml` runs the real Chrome for Testing handoff and the Firefox checks.
- `audit.yml` runs `cargo-deny` for advisories and licences.

A change that adds behaviour needs a test that fails without the change (rule 13).

## Engineering rules

[`docs/ENGINEERING_RULES.md`](docs/ENGINEERING_RULES.md) is the list of binding rules. Highlights for contributors:

- Never overwrite user files; publish with a temporary file and a checked rename or hard link.
- Never run a shell; spawn programs with an argument vector.
- Secrets never leave their store and never appear in arguments, logs or exports.
- IPC changes are append-only: add new `Command` variants, never new fields on existing ones.
- SQLite schema changes are additive, idempotent migrations.
- Builds and tests must be warning-free: `cargo build --workspace` and `cargo test --workspace`.
- Do not copy proprietary text, code or assets from any other product.

## Documentation

User documentation is in [`docs/user/`](docs/user): the guide, FAQ, troubleshooting and privacy statement. Every command, option, menu label and path in those files must match the source. If you change one, update the matching page in the same commit. Docs rows in [`docs/PRODUCTION_READINESS.md`](docs/PRODUCTION_READINESS.md) describe tested behaviour only.

## Commits and pull requests

Look at `git log` for the house style:

- Subject lines start with an area followed by a colon and a short summary, for example `CI: ...`, `Media relay: ...`, `Windows: ...`, `run_all: ...`. Other subjects start with a verb or a noun phrase describing the change.
- Keep one logical change per commit.
- Merges from feature branches use the form `Merge agent/<name>: <summary>`. Do not commit a half-resolved merge.
- Describe what you ran and what you did not run. If a change was not tested on Windows or in a real browser, say so in the message or the pull request.

Work on a branch and open a pull request against `main`.
