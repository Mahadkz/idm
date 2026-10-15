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
| Real-browser tests: Chrome/Chromium and Firefox headless, install/load extension, handoff, media capture | Not started | |
| Native Windows: build, unit tests, integration suites, named-pipe IPC, DPAPI, registry proxy, power actions (fake adapter), GUI smoke | Not started | |
| macOS | Out of scope for v1 (Linux and Windows first, per project decision) | |

## 3. Production engineering

| Item | Status | Evidence |
|---|---|---|
| CI on every push: Linux build + all suites (Xvfb), Windows build + tests, browser tests, warnings-as-errors, Windows cross-build | Not started | |
| Versioning: `--version` on every binary, IPC protocol version negotiation, schema version and migration tests from every previous release | Not started | |
| Logging: leveled logs with rotation and size cap, no secrets, log location documented | Not started | |
| Crash handling: panic hook writes a redacted crash report; daemon restarts cleanly; diagnostics bundle command | Not started | |
| Single-instance daemon lock, graceful shutdown that checkpoints all transfers, data-directory permissions | Not started | |
| Database safety: integrity check, backup before migration, recovery from corruption | Not started | |
| Security: threat model document, all security reviews' findings closed, fuzzing in CI, dependency audit (cargo audit/deny), no secrets in logs/exports/argv | In progress | security reviews in git history. Open: browser-captured Cookie/Authorization headers for media jobs are passed to ffmpeg `-headers` on its command line (visible in /proc to other local users); fix by fetching media segments in-process or via a private loopback relay |
| Performance gate: benchmark in CI with regression thresholds; idle CPU/RAM budgets | Partial | bench/ |
| Packaging: Linux AppImage and .deb, Windows installer (per-user, upgrade/uninstall), extension packages (.zip/.xpi), third-party license notices | Partial | packaging/, install/ |
| Updates: update check with signed metadata (no silent auto-install) | Not started | |
| Documentation: user guide, FAQ, troubleshooting, privacy statement, contributor guide | Not started | |
