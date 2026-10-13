# Supplied IDM artifact audit: completion and implementation boundary

**Audit status (2026-10-12): complete for the supplied installer, extension packages, and controlled Linux/Wine experiments.** This does not mean all IDM behaviors, arbitrary websites, or native Windows integration were reproduced. The aim is a clean-room functional specification with explicit evidence levels and a build plan. This document records the audit at its closure; implementation subsequently began. See [current implementation status](implementation_status.md).

The subsequent [feature mapping pass](feature_mapping_status.md) adds individual records for every control, semantic review of all catalog entries, and 170 detailed desktop/browser/core/queue contracts. It retains unresolved callers and platform behavior separately from mapped feature inventory.

The principal records are the [desktop investigation](idm_reverse_engineering.md), [browser/media analysis](idm_browser_media_analysis.md), [native-core analysis](idm_native_core_analysis.md), [105-dialog catalog](analysis/idm_dialog_catalog.md), [runtime evidence](analysis/runtime/index.json), [final runtime and package evidence](analysis/runtime_final/index.json), and [architecture specification](architecture_spec.md). The original binaries, JavaScript, imagery, and decompiler output stay outside the new project.

## What was actually reverse engineered

The installer was unpacked into 191 named files with integrity-checked zlib records; 47 PE files were inventoried. The main executable's 105 dialog templates, 1,055 controls, two menu trees, 156 populated PE string entries, and 1,604 functional translated strings were cataloged. Relevant native host, parser, muxer, and main executable paths were selectively decompiled. The separately supplied Chromium MV3 extension was inspected and run against mocked browser APIs for 15 contract assertions. The installed Chromium, Edge, legacy Chromium, and Firefox extension packages were also identified and compared. The original application was run under Wine on isolated, deterministic local HTTP/media fixtures, with GUI, HTTP requests, file hashes, and restart outcomes captured.

REA was inspected but its runnable dependencies were absent. The supplied Hermes decompiler was **not used**: no Hermes bytecode was found; the extension code is JavaScript and the desktop components are native PE binaries. Ghidra was the applicable decompiler for selected native functions. Neither tool is needed for a fresh Rust implementation.

## Feature and UI map

Evidence codes: **R** observed in original under Wine; **S** source/resource/installed-package evidence; **H** shipped help; **M** extension mock only; **V** future platform/product validation. A screen or string marked S/H is a feature lead, not proof of current behavior.

| Surface | Observed or specified behavior | Evidence | New-product decision |
|---|---|---|---|
| Main shell | Menus, toolbar, task table, category tree, queue and grabber nodes, drop target, tray choices | R/S/H | Original visual design with familiar task workflow; no copied assets |
| Task rows | Name, size, status, rate, remaining time, path, source/referrer and queue marker; sorting/column controls | R/S/H | Durable row model and event-driven updates |
| Task actions | Add URL, start, pause/stop, resume, redownload, delete, open, move/rename, properties, refresh expired address | R/S/H | Explicit state machine and per-action error handling |
| Download dialogs | URL/credentials, category, path, filename, description, start later, eager transfer, duplicate choices | R/S/H | Staged task creation; eager transfer is optional later milestone |
| Range acceleration | Initial GET probing, parallel and dynamically split Range requests, partial-file recovery, correct fixture hashes | R/S/H | Independent bounded segment scheduler with strict response validation |
| Failure handling | No-range retry, malformed range fallback, 503 recovery, 403 error, changed-object restart prompt | R | Fixture-driven safety tests before optimization |
| Categories and paths | Type/site rules, general and built-in folders, remembered path, category list filters | R/S/H | Typed rules with deterministic precedence |
| Queue controls | Main, synchronization, custom queues, ordering, manual start/stop, per-queue simultaneous-file limit | R/S/H | Persisted queue model; one-file concurrency validated |
| Scheduling | One-shot/daily/weekday starts, stop time, periodic sync, usage quota, completion program and power choices | S/H | Deliver in stages; require explicit opt-in and countdown for power actions |
| Speed controls | Per-file limiter setting and startup toggle; connection-count options and per-server overrides | R/S/H | Separate per-task and global token buckets; measure actual behavior in new engine |
| Search and batch | Find File scopes; generated URL batch UI; clipboard/import/export menus | R/S/H | Search and simple batch first; documented import/export schema |
| Text import/export | URL list import preview, deferred task creation, URL-per-line export, `.ef2` plain-text block sample | R | Support portable text/JSON formats; do not imitate `.ef2` blindly |
| Site Grabber | Four-stage crawl wizard, selection tree/table, filters, save/project scheduler | R/S/H | Separate later service with crawl limits and offline-link rules |
| Settings | Nine reached tab families: general, types, connections, paths, downloads, proxy, logins, dial-up/VPN, sounds; nested media and shortcut panels | R/S/H | Cross-platform settings mapped to applicable OS capabilities |
| Browser handoff | Native Messaging framing, native host, cookies/UA/referrer/headers, filename interception, context menus, fallback | S/M | Fresh versioned schema and bounded cancel/ack transaction |
| Media discovery | Direct URLs, page/player metadata, response MIME, HLS/DASH indications, XHR/fetch capture, panel geometry | S/M | Quality catalog from manifest + observed network; clear streams only |
| Media output | Native parser/muxer exports and audio-only strings; direct MP4 succeeded, ordinary manifest URL saved manifest | R/S | Build/test own clear HLS/DASH resolver and FFmpeg-assisted mux path |
| OS integration | Windows drivers, monitor DLL, shell extensions, helper/broker and native-host manifests | S | Replace with browser/native-host APIs and OS-specific registration; no driver clone |

The dialog catalog is the exhaustive UI resource ledger. Historical, empty, and unreachable dialogs remain in that ledger; the new-product scope is set by behavior and user value, not by blindly implementing every resource ID. The visual target is comparable density and command discoverability, not a pixel copy.

## Browser variants and contracts

The supplied/installed Chromium MV3 `6.43.1` package has a service worker, `downloads.onDeterminingFilename`, `webRequest`, `declarativeNetRequest`, and content scripts. The installed Edge `6.43.1` package is MV2 with a persistent background and blocking `webRequest`. The installed Firefox `6.43.1` XPI is MV2, uses `webRequestBlocking` and the Gecko ID `mozilla_cc3@internetdownloadmanager.com`, but its background does **not** register `downloads.onDeterminingFilename`. All three connect to native host `com.tonec.idm`; the host manifests distinguish Chromium allowed origins and Firefox allowed extension ID. The older installed Chromium `6.40.8.2` MV2 package is historical evidence, not the baseline. Exact hashes, permissions, hooks, and script hashes are preserved in [browser_packages.json](analysis/runtime_final/browser_packages.json).

The original Chromium component's tested handoff is: record request/response context → correlate browser download → pause → submit an inner type-13 native message → cancel browser task after accepted acknowledgment or release suggestion and resume on fallback. Cookie, User-Agent, referrer, request/response headers, body, MIME, filename, size, and page URL fields are mapped in [architecture_spec.md](architecture_spec.md). A mock disconnect-after-send left a pending callback in one bounded observation; the new extension must enforce a timeout and once-only completion. Firefox needs its own adapter and browser tests, not an assumption that Chromium APIs carry over.

## Controlled runtime conclusions

Parallel Range activity and recovery were verified by exact hashes on 64 MiB fixtures. Separate segment files plus a task log appeared on pause; the replacement's proposed sparse shared-output design is therefore an independent engineering choice. A one-file queue limit serialized three 8 MiB tasks. A malformed `Content-Range` produced a safe full-transfer fallback, two initial 503s eventually recovered, and 403 ended in Error without an output. On changed ETag **and** Last-Modified, the app prompted for a restart; accepting it produced exactly the new 8 MiB object. The trace cannot isolate ETag behavior, and no `If-Range` was observed. Graceful relaunch kept 13 rows, the custom queue, and its changed Main queue limit; a separate forced-process-termination test recovered a partial 64 MiB task. Read the [fixture results](analysis/runtime_final/index.json) for precise hashes and captures.

The standalone original media DLL probe is **inconclusive**. One conversion export faulted under Wine; another emitted an MP4 that FFprobe recognized but FFmpeg did not cleanly decode. Calling an export outside its parent application does not validate its ABI, initialization, or expected input. It neither proves nor disproves integrated IDM stream assembly.

## Scope limits and validation register

No native Windows machine was used. Windows monitor drivers, broker security, browser registration and capture, shell menus, tray/power actions, and DRM notification were not exercised. No real browser was paired with the original native host under Wine. The extension mock demonstrates component branching only. Actual Firefox interception, Edge blocking API behavior, player overlay positioning, authenticated page/POST replay, protected-content detection, available-quality enumeration, separate audio/video selection, and original integrated HLS/DASH muxing remain unverified. Platform tests are required for **our** implementation; copying the original's opaque pipe protocol is not a prerequisite.

Other edge cases remain product test cases rather than blockers to a reviewable architecture: >4 GiB files, disk full, compressed/redirected/expiring responses, proxy auth, clock changes and missed schedules, concurrent queue limits, startup/shutdown transitions, grabber authenticated crawling, and `.ef2` round-trip. The fixture matrix in [the architecture specification](architecture_spec.md) turns these into milestone acceptance tests. No claim of every site, format, or resolution is supportable: availability depends on the source, browser permissions, credentials, URL lifetime, codecs, and DRM.

## Audit closure and next decision

The supplied artifacts have yielded a complete bounded inventory, reproducible evidence, explicit unknowns, and a clean-room architecture/directory plan. Further reverse engineering of proprietary internals was not necessary to begin an independent engine. The user subsequently authorized implementation; [current implementation status](implementation_status.md) tracks the resulting code.
