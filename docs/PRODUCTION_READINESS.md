# XDM production readiness

The release gate. XDM is production ready only when every item below is **Done** with the cited evidence, or explicitly marked out of scope with the reason. "Done" means implemented, tested by an automated check that runs in CI, and documented.

Status values: Done · In progress · Not started · Out of scope (reason).

## 1. Desktop GUI — IDM layout parity

Match IDM's structure and workflows using the mapped dialogs (`docs/idm_control_review.md`, `docs/feature_coverage.md`) and independent artwork; no IDM icons, images or branding.

| Item | Status | Evidence |
|---|---|---|
| Main window: menu bar (Tasks, File, Downloads, View, Help), large-icon toolbar in IDM order, category tree sidebar (All Downloads → categories, Unfinished, Finished, Grabber projects, Queues), task list with IDM columns, status bar | Not started | |
| "Download File Info" dialog on new downloads: URL, category, save-as path, description, remember path, Start Download / Download Later / Cancel | Not started | |
| Per-download progress window: status, size, downloaded, rate, time left, resume capability, per-connection segment bar and connection table, Pause / Cancel, Speed Limiter tab, on-completion options | Not started | |
| "Download complete" dialog: Open, Open with, Open folder, Close, "don't show again" | Not started | |
| Options dialog tabs: General, File types, Save to, Downloads, Connection, Proxy/Socks, Site Logins, Dial Up/VPN (Windows), Sounds | Not started | |
| Scheduler dialog (queues list, schedule, files in queue, start/stop, completion actions), Speed Limiter menu and dialog | Partial | gui_actions.py |
| Grabber wizard (multi-step), batch download dialog, add-media dialog, properties dialog | Partial | gui_actions.py |
| Tray icon with menu and completion balloons; start minimized; close-to-tray | Not started | |
| Keyboard access, focus order, screen-reader names; dark/light themes; DPI scaling; translations | Partial | |
| Remembered window geometry, column widths/order, sort and category selection | Partial | |

## 2. Engine, daemon and integrations

| Item | Status | Evidence |
|---|---|---|
| Behavior contracts: core, browser, behavior-review — remaining Missing items closed or out of scope | In progress | docs/parity_ledger.md |
| Real-browser tests: Chrome/Chromium and Firefox headless, install/load extension, handoff, media capture | Partial | Chromium: tests/integration/chromium_real.mjs (handoff, daemon-down fallback; no media-capture case). Firefox: tests/integration/firefox_real.mjs (real headless Firefox via geckodriver: temporary add-on install, native hello, media sniffing + media handoff, link click handed off with URL/referrer/cookie, no browser copy, daemon-down fallback); CI job browsers/firefox. Linux only; Windows browsers untested |
| Native Windows: build, unit tests, integration suites, named-pipe IPC, DPAPI, registry proxy, power actions (fake adapter), GUI smoke | Not started | |
| macOS | Out of scope for v1 (Linux and Windows first, per project decision) | |

## 3. Production engineering

| Item | Status | Evidence |
|---|---|---|
| CI on every push: Linux build + all suites (Xvfb), Windows build + tests, browser tests, warnings-as-errors, Windows cross-build | In progress | .github/workflows/{linux,windows,browsers,audit}.yml, tests/run_all.py; awaiting first GitHub run; Firefox real-browser test and Windows named-pipe Python client not yet written |
| Versioning: `--version` on every binary, IPC protocol version negotiation, schema version and migration tests from every previous release | Partial | `--version` on cli/daemon/gui/native-host and `Hello` handshake with protocol check (tests/integration/ops_hardening.py, xdm-ipc ops tests); schema `user_version` + backup-on-migrate test (daemon `migration_backs_up_old_database_and_keeps_three`). Not done: migration tests from every previous release (only the oldest fixture schema) |
| Logging: leveled logs with rotation and size cap, no secrets, log location documented | Partial | In-house logger `<data>/logs/xdm-daemon.log` (5 MiB x 3, `XDM_LOG`), redaction tests in xdm-ipc `ops::tests`; ops_hardening.py checks no secret in log. Daemon logs lifecycle/client errors only; log location not yet in user docs |
| Crash handling: panic hook writes a redacted crash report; daemon restarts cleanly; diagnostics bundle command | Partial | Panic hook writes redacted crash report to `<data>/crash/` (10 kept; unit test), `xdm-cli daemon diagnostics OUT.zip or OUT_DIR` (ops_hardening.py). Not exercised: a real daemon panic end to end, GUI hook on a display, automatic daemon restart |
| Single-instance daemon lock, graceful shutdown that checkpoints all transfers, data-directory permissions | Partial | flock/LockFileEx lock (lock.rs test + ops_hardening.py second daemon refused, stale lock reused), SIGTERM graceful shutdown with resumable checkpoint (ops_hardening.py), data dir 0700 / token 0600 tests. Windows lock, ctrl handler and permissions compile-checked only, untested |
| Database safety: integrity check, backup before migration, recovery from corruption | Partial | `quick_check` at startup with refusal message (`corrupt_database_is_refused_with_guidance`), backup before migration keeping 3, WAL + synchronous=NORMAL. No automatic corruption recovery (manual restore from backup) |
| Security: threat model document, all security reviews' findings closed, fuzzing in CI, dependency audit (cargo audit/deny), no secrets in logs/exports/argv | In progress | security reviews in git history. Fixed: media jobs reach ffmpeg/ffprobe only through a private loopback relay (random path token, per-job lifetime, origin-scoped headers and Basic/Digest logins); argv carries no credentials (evidence: `tests/integration/media_relay.py`). Relay serves only URLs it issued (token is not a secret vs local users; it grants no arbitrary fetch), refuses private/loopback/link-local upstreams except the manifest's own origin, and honors the job proxy. Referer and User-Agent are sent to every origin, other captured headers only to the manifest origin; encrypted HLS and DRM remain unsupported |
| Performance gate: benchmark in CI with regression thresholds; idle CPU/RAM budgets | Partial | bench/ |
| Packaging: Linux AppImage and .deb, Windows installer (per-user, upgrade/uninstall), extension packages (.zip/.xpi), third-party license notices | Partial | packaging/ (release.sh, build-deb.sh, build-appimage.sh, windows/build-installer.sh, extensions/build.sh, licenses.sh), install/; .deb/AppImage extracted and CLI run, NSIS installer built but untested on Windows, web-ext lint and lintian not run, packages unsigned |
| Updates: update check with signed metadata (no silent auto-install) | Not started | |
| Documentation: user guide, FAQ, troubleshooting, privacy statement, contributor guide | Done | docs/user/guide.md, docs/user/faq.md, docs/user/troubleshooting.md, docs/user/privacy.md, CONTRIBUTING.md |
