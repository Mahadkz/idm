# Architecture specification and implementation plan

Status: supplied-artifact audit complete within Linux/Wine scope. The user authorized implementation with Linux and Windows first, minimum RAM and CPU as priorities, and macOS later. A first Rust engine, daemon, native host, browser-extension prototype, clear-media CLI, and native-GUI prototype now exist; the feature matrix remains the target and is not a claim of parity. See the [audit completion and feature matrix](idm_audit_completion.md) and [current implementation status](../README.md).

## Evidence and boundaries

Target identities, all 105 desktop dialog resources, menu trees, settings families and runtime observations are in [the IDM investigation](idm_reverse_engineering.md). Follow-up experiments and native decompilation are in [the browser/media audit](idm_browser_media_analysis.md).

The supplied extension is MV3 version 6.43.1. `background.js` is a service worker; `content.js` runs at document start in all HTTP(S) frames. Web-accessible page-context helpers include `document.js`, `captured.js`, `captured.html`, and a debugging script. Original source gates execution on Windows. The fresh extension will require independent cross-platform support.

Native binaries, extracted extension code, logos, images, help content and recovered pseudocode are investigation inputs only. The replacement will have original source and assets. Native Windows drivers and legacy IE integration are separate compatibility surfaces; the browser extension cannot be assumed to reproduce them.

## Observed extension-to-desktop contracts

### Browser transport

Native Messaging uses stdin/stdout: an unsigned four-byte native-endian length followed by that many UTF-8 JSON bytes. The byte count excludes the prefix. Reads/writes may be partial; framing implementations must loop. Stdout contains protocol frames only. Registration uses a stdio host manifest, an executable path and allowed extension identities. These are browser contracts, independent of IDM. [Chrome protocol and registration](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging), [Firefox native messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging).

The original background attempts WebSocket addresses `127.0.0.1:1001` and `0.1.0.1:1001` with subprotocol `plugin.v3.internetdownloadmanager.com`, and uses native host `com.tonec.idm`. Those proprietary endpoint names and protocol are not proposed replacement identifiers.

The original host translates browser-facing JSON strings to a separate Windows named-pipe format. Native decompilation establishes a session suffix on `\\.\pipe\IDMNetworkMonitor.` and asynchronous I/O workers. The complete internal pipe framing and authentication contract remain unresolved. The controlled host execution under Wine produced no reply.

### Original application message grammar

The native port posts a JSON **string**, whose contents have this observed form:

```text
MSG#sequence#type#variant#flags[:numeric-arguments][,field=byte-length:text or number];
```

Strings use UTF-8 byte lengths. Binary/form-body encodings and flag meanings vary by path. This is a bounded grammar description, not a complete interchangeable implementation of the original protocol.

| Family | Observed purpose |
|---|---|
| 0 | Keepalive |
| 1 | Acknowledgment/reply |
| 2, 3, 4 | Initialization and configuration |
| 8, 9 | Media-panel geometry/visibility |
| 13 | Browser download handoff and detected network media |
| 14, 15 | User-requested link and link batch |
| 16 | Captured response body |
| 18 | Host site-matching configuration |
| 19 | Extracted site-specific metadata |
| 21, 23 | Page/media metadata and identifiers |

| Field | Observed data |
|---|---|
| 6, 7 | Resource URL and parent page URL |
| 17 | Earlier/original resource URL |
| 50, 51, 54 | Referrer, serialized cookies, user agent |
| 56 | Original-URL cookies in redirect paths |
| 11, 13, 18 | Request, response, earlier request headers |
| 14, 19 | Request body and earlier body |
| 100, 103 | Filename and browser-reported size |
| 4, 127 | Extension/classification and MIME metadata |
| 115 | Captured response text in the tested manifest path |
| 121, 122 | Proxy context variants |

The numbers describe observed producers; every occurrence is not guaranteed to share these meanings. The detailed source-path references are in the investigation report.

## Event and handoff sequence

1. `webRequest` before-request/before-send-headers events collect URLs, method, request body and headers. Response-header/response-started events add MIME, size, redirect and media metadata. Completion/error events manage correlation lifetimes.
2. `downloads.onCreated` correlates eligible downloads with recent requests and pauses them.
3. `downloads.onDeterminingFilename` filters candidates, collects browser context and sends type 13. Cancellation is a separate API operation.
4. In the tested path, acknowledgment status 3 cancels the browser download. Rejection or inability to send invokes the filename callback and resumes the browser download.
5. Context-menu requests use type 14; multiple-link submission additionally uses type 15. These explicit user actions differ from interception.
6. Declarative session rules redirect configured main-frame matches through `captured.html`. Firefox-oriented source branches do not establish equivalent Chromium blocking behavior.

Filename suggestion callbacks must be released exactly once. The mock disconnect-after-send observation did not show immediate release; the replacement must implement a bounded fallback. Chromium MV3 and Firefox adapters need separate integration tests. [Downloads API](https://developer.chrome.com/docs/extensions/reference/api/downloads), [webRequest API](https://developer.chrome.com/docs/extensions/reference/api/webRequest).

## Media discovery and response capture

Detection combines request type, MIME, URL classification, host-supplied rules, and page metadata. It includes direct video/audio, HLS and DASH. Blob URLs are browser-local references, not transferable HTTP resources; the underlying network requests or manifest bodies are needed.

Content scripts inspect media elements and source children, nested frames, visibility and geometry. Page-context hooks capture configured XHR/fetch/Response data and relay it through content scripts to the background. The original floating panel appears to involve native desktop UI coordinated with reported geometry; a complete injected DOM button implementation has not been established.

Tested manifest path: detected type 13 → simulated host body request → content-port tuple type 19 → captured text returned to background → native type 16 with field 115. This was tested with browser mocks, not a real installed browser/native host pair.

Original manual CLI downloads of HLS/DASH URLs saved manifest files. This does not establish the extension-driven stream behavior. Native parser/muxer exports and selected decompiled wrappers establish separate parsing and audio/video assembly subsystems, with runtime coverage still pending.

The replacement must distinguish:

- Direct downloadable file, including a single video/audio resource.
- Clear HLS/DASH presentation with independently selectable representations and audio tracks.
- Protected presentation, reported as unsupported without decryption.
- Browser-local or inaccessible resource, reported with its actual limitation.

Resolution discovery must distinguish representations enumerated from a manifest from those merely observed during playback. MP4 is a container; MP3 output can require transcoding. Available qualities, codecs, mux compatibility, URL expiration and authentication determine usable choices. No universal every-site/every-format guarantee is justified by the audit.

## Proposed fresh contract for review

Use versioned JSON objects instead of the original numeric grammar. The daemon owns tasks and durable state; the native host only validates framing and bridges to a per-user local endpoint. Unix sockets and Windows named pipes provide the native transport; a GUI RPC/event adapter sits above the same application service. Browser registration must be generated for each supported browser/platform. Linux and Windows are the first release/benchmark targets; the original macOS goal remains a later portability target unless the user removes it.

Example browser submission, illustrating the proposed schema:

```json
{
  "version": 1,
  "requestId": "stable-handoff-id",
  "type": "download.submit",
  "payload": {
    "url": "https://example.invalid/file.mp4",
    "method": "GET",
    "filenameHint": "file.mp4",
    "source": {"pageUrl": "https://example.invalid/player", "browserDownloadId": 42},
    "requestContext": {
      "headers": [{"name": "User-Agent", "value": "browser-agent"}],
      "referrer": "https://example.invalid/player",
      "cookies": [{"url": "https://example.invalid/file.mp4", "header": "fixture=example"}]
    }
  }
}
```

`requestId` is an opaque idempotency key. Header lists preserve repeated fields. Cookie context is scoped to its resource URL; authenticated redirects must not blindly forward credentials to unrelated origins. Bodies require explicit replayability and encoding. Browser-provided names are hints; the engine validates paths and resolves output naming.

Proposed reply: `{version, requestId, type: "download.result", payload: {accepted, taskId, reason}}`. An accepted reply follows durable task registration. A timeout, malformed reply or disconnect releases pending browser interception. Durable deduplication prevents repeated submissions from creating multiple daemon tasks; atomic ownership transfer across browser cancellation and daemon acknowledgment still needs a specified recovery sequence. Do not claim exactly-once transfer merely from an idempotency key.

Other message families: `hello`/capabilities, `media.candidate`, `media.body.chunk`, `media.selection`, `task.command`, `task.event` and `integration.status`. Body chunks need bounded sizes, sequence validation, an overall byte limit and completion hash. The proposed concrete boundaries and limits follow below; wire-level schemas will be frozen and tested in the implementation milestone.

### Implemented v1 subset

The current independent Chromium extension uses Native Messaging with `org.xdm.browser`. The host accepts at most 1 MiB framed JSON messages and bridges accepted tasks to the daemon's per-user local IPC. These messages are implemented now; the other families above remain proposals:

| Browser message | Required input | Host response and effect |
|---|---|---|
| `hello` | `version: 1` | `hello.result` with `download.submit`, `media.inspect`, `media.submit` capabilities |
| `download.submit` | `version`, `requestId`, `payload.url`, optional `filenameHint`, `mime`, `requestContext.headers`, `referrer`, `cookies[0].header` | `download.result` with `accepted`, `taskId` or `reason`; durable daemon task is created before acceptance |
| `media.inspect` | `version`, `payload.url` to clear `.m3u8` or `.mpd` | `media.inspect.result` with `ok`, FFprobe `tracks` or `reason`; no task created |
| `media.submit` | `version`, `requestId`, `payload.url`, `videoIndex`, optional `audioIndex` | `media.result` with `accepted`, `taskId` or `reason`; selected stream is queued |

The internal IPC envelope is `{version: 1, token, command: {type, payload}}`, length-prefixed over a Unix socket on Linux or named pipe on Windows. Commands implemented are `submit`, `submit_media`, `import_list`, `export_list`, `list`, `get_task`, `redownload`, `remove`, `pause`, `resume`, `set_priority`, `set_task_rate`, `set_concurrency`, `queue_start`, `queue_stop`, `queue_schedule_start`, and `queue_schedule_stop`. Schedule payloads use Unix seconds (`at_unix`); `list` reports task summaries plus `queue_running`, `queue_start_at`, and `queue_stop_at`. Task summaries include `kind`, `category`, `status`, `priority`, `downloaded`, `total`, `rate_limit`, `bytes_per_second`, and `eta_seconds`. `set_task_rate` carries `{task_id, bytes_per_second}` with null for unlimited; caps are persisted for HTTP tasks and apply to active transfers immediately. Live counters are in memory while active; paused/recovered progress comes from durable checkpoints. Speed is sampled over intervals of at least 250 ms and ETA is based on that sample, so it changes with observed throughput. FFmpeg media tasks currently have no equivalent cap or live speed counter. `get_task` takes `{task_id}` and returns a single task in `tasks`, including connection count, timestamps and media track indices, without request headers. `redownload` takes `{task_id, request_id, output}` and creates a distinct queued task at a fresh absolute path with inherited configuration. Existing outputs/checkpoints and parent-directory traversal are rejected. `remove` takes `{task_id}`, requires no active worker, and retains filesystem data. Removal stores a durable submission receipt so replay of a removed request is rejected and task IDs are never reused. `import_list` takes `{request_id, directory, entries, enqueue}` and returns `task_ids`. Its SQLite transaction commits all tasks and a durable batch receipt together; conflicts reject the whole batch. `export_list` returns `transfer_list` with version 1 portable entries, excluding request headers and URL user credentials. Both are bounded to 500 tasks/512 KiB. The [download-list schema](transfer_lists.md) documents the shared parser, preview, and paused/queued import choices. Named queue commands are `queue_create`, `queue_set_running`, `queue_set_concurrency`, `queue_rename`, `queue_remove`, and atomic `assign_queue`. `list` includes `queues`; each task has `queue_id`. Main is queue 1; migrations place existing tasks there. New queues start stopped. A global running gate and job cap apply above separate per-queue state/limits. Stopping one queue checkpoints only its workers and leaves other queues running. Active tasks must stop before assignment, empty queues can be deleted, and Main cannot be deleted/renamed. The dispatcher uses indexed head candidates from eligible queues, choosing by priority and then oldest task ID. This is a local v1 implementation protocol, not a stable public API yet.

The Chromium worker currently listens to `webRequest.onBeforeSendHeaders` for GET context and `onHeadersReceived` for MIME/media URL candidates. `downloads.onDeterminingFilename` pauses an opted-in browser download, sends `download.submit`, cancels only on acceptance, and otherwise resumes it with a 10-second fallback. Link and media context menus call the same handoff. The popup lists observed media URLs and, for clear manifests, lets the user select FFprobe-reported video and audio tracks. The native host classifies browser file destinations; the daemon stores task category and persists one-shot queue start/stop times. Queue stop checkpoints active downloads and requeues them. The browser capture is still a URL observation path: page-context XHR/fetch capture, a floating player button, Firefox support, and authenticated media replay are not implemented.

The Linux `tests/integration/chromium_real.mjs` check launches Chrome for Testing with an isolated profile and Native Messaging host registration. It verified that an opted-in browser download was canceled after durable XDM acceptance and that XDM's output matched the fixture hash. With the daemon stopped, the browser completed the next download. Do not set the DevTools `Browser.setDownloadBehavior` override in this test: in the tested build it bypassed `downloads.onDeterminingFilename` and made the browser finish the file itself. This test does not validate installed Google Chrome/Edge/Brave profiles or native Windows browser behavior. Chrome for Testing supports unpacked-extension command-line loading where current branded Chrome builds do not ([Chromium extensions announcement](https://groups.google.com/a/chromium.org/g/chromium-extensions/c/FxMU1TvxWWg)); its Native Messaging host location is [browser-specific](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging).

## Phase 1 validation record and outstanding items

Persisted evidence: [component results](analysis/browser_media/extension_contract_results.json), [native inventory](analysis/browser_media/native_function_inventory.json), [original media download results](analysis/browser_media/manual_media_downloads.json).

Completed: source inspection, resource/contract inventory, selected native decompilation, 15 component assertions, and original manual media downloads under Wine. Outstanding: real host negotiation, authentication fidelity, POST replay, Firefox interception, overlay placement, quality/audio selection and stream muxing. None is marked verified by the mock results.

The system architecture uses Rust for the engine, daemon, host, and CLI, with the engine independent of the GUI. The native desktop toolkit remains a benchmark gate, described below. The user has confirmed Linux and Windows first and authorized implementation.

The subsequent [native core follow-up](idm_native_core_analysis.md) identifies range generation, response parsing and recovery path construction in IDMan.exe. It reinforces two distinctions for the final plan: HEAD is not the only viable probing operation, and the proposed shared sparse output differs from IDM's observed segment-file storage.

## Proposed system decomposition

Rust is the proposed language for the daemon, engine, native host and CLI. A native desktop UI is the performance baseline: Qt Widgets/C++ is the first full-featured candidate, and a small Rust/Slint prototype is the challenger if it delivers the required table, tree, dialogs, accessibility and tray behavior with lower measured total cost. The download engine has no dependency on either UI toolkit, browser APIs, or windowing. A per-user daemon owns all task state; CLI, desktop, scheduler, and browser are clients of the same application service. The GUI can exit while downloads continue, so idle GUI memory is not imposed on daemon-only operation.

```text
Chromium MV3 / Firefox-compatible adapter
                │ Native Messaging: framed JSON
                ▼
      small per-browser native host
                │ authenticated per-user IPC
                ▼
      daemon: task service + durable store
       ├── transfer engine (HTTP, ranges, checkpoints)
       ├── media service (clear HLS/DASH, selection, mux)
       ├── queue + scheduler + categories
       └── event stream / settings / integration status
                ▲
       CLI and native desktop client
```

The daemon listens only on a user-owned Unix-domain socket on Linux (and later macOS) and a user-scoped named pipe on Windows. It checks peer/user identity where the OS provides it and uses a stored installation secret for request authentication. Native-host registration is generated by platform/browser installer steps: Chromium-family hosts allow extension origins; Firefox hosts allow its extension ID. Windows uses per-user registry registration; macOS browser-specific manifest directories are addressed in its later portability milestone. No unauthenticated TCP control endpoint is part of the baseline. The GUI subscribes to a versioned local event stream and can reconnect from a durable task snapshot.

## Durable task model and transfer engine

Each task has a stable UUID, source URL and method, replayable request context, origin/page metadata, destination, category, priority, queue ID, created/updated times, observed size/MIME/validators, requested and actual limits, byte counters, segment map, retry budget, and last error. States are `queued`, `probing`, `running`, `pausing`, `paused`, `retry_wait`, `needs_attention`, `failed`, `finalizing`, and `complete`. Commands carry an idempotency key and expected task revision. Browser cookies/headers are encrypted or kept in an OS-protected store where available; GUI task exports omit secrets by default. A filename hint never becomes an arbitrary filesystem path.

1. Probe with a bounded GET or HEAD where useful; do not require HEAD or `Accept-Ranges` alone. Record final URL, identity validators, length and encoding. Test actual Range behavior with a bounded request before opening parallel workers.
2. Partition known-size, byte-addressable content into 4–16 workers subject to host and task caps. Validate each `206`, `Content-Range`, total length, response encoding, and bytes received. Workers write at explicit offsets into a pre-sized temporary file; sparse allocation is an optimization where supported, with a portable fallback. `SetFileValidData` requires special Windows privilege and is **not** assumed available. Unknown-size, compressed, or range-ignoring responses use a single stream.
3. Track committed byte intervals and outstanding leases. At safe checkpoints, fsync the data as policy requires and atomically replace `.part.meta` with version, URL/redirect identity, validators, total, interval hashes/checksums, and durable offsets. On restart, verify metadata and the data file before reuse. Never merge bytes across changed representation/validator without explicit validation; restart or ask the user.
4. Retry transport and selected transient server failures with bounded exponential backoff and jitter. Distinguish HTTP 403/401, malformed 206, changed object, expired URL, disk full, cancellation, and offline status. Reallocate the largest remaining verified interval to idle workers without overlapping owned writes; a stalled worker's lease expires only after its socket is canceled and committed progress reconciled.
5. A hierarchical token bucket enforces optional global, per-task and per-host caps. Connection counts and speed limits are separate. Pause stops new work, cancels active reads, checkpoints committed bytes, then transitions to `paused`. Completion verifies length/coverage and optional digest, flushes, then atomically renames if the destination filesystem supports it. A cross-filesystem move uses copy/verify/replace.

Use an embedded transactional database for tasks, queues and settings, plus a per-task checkpoint sidecar for partial bytes. The database and sidecar have independent schema versions and upgrade paths. Network request headers that identify a browser session require origin-scoped forwarding: cookie and Authorization material must not be sent across unrelated redirects. POST replay is accepted only when the request body is bounded, captured with consent, and explicitly replayable; otherwise the task surfaces `needs_attention`.

## Browser capture and media selection

Provide separate Chromium MV3 and Firefox adapters sharing only the domain model and native message schema. The Chromium adapter observes eligible download creation and filename determination, pauses during handoff, and cancels only after the daemon's durable acceptance reply. Every pending filename callback has a once-only timeout/fallback path. Firefox uses capabilities available to its supported manifest/browser versions and has independent interception tests. Context menus support "Download with" on links, page media, selected links, and eligible browser downloads. Filter/site exclusions and modifier keys are user settings; the user can let the browser handle a candidate.

The extension collects direct URLs, response MIME/length, source page, and visible player metadata. A limited page-context hook can observe clear XHR/fetch media requests and manifest bodies. The content script may render an original floating action button aligned to an eligible visible player; its controls remain usable without the overlay through a toolbar popup. The native host validates message size (target: 1 MiB per frame), schema and source extension. Captured manifest bodies use chunked messages (target: at most 256 KiB each, 16 MiB total) and a completion hash. These are initial contract limits to be adjusted by fixture tests, not observations of IDM.

The media service parses clear HLS master/media playlists and DASH MPDs, enumerates **available** video representations and compatible audio groups, labels resolution, bitrate, codec, language and output container, and distinguishes available variants from the single variant merely seen in playback traffic. It downloads a chosen combination with segment retry/ordering and discontinuity handling, then invokes an optional installed FFmpeg for remux or transcode where licensing and platform packaging permit. MP3 extraction requires an explicit audio transcode; an MP4 output requires compatible codecs or an explicit conversion. A direct MP4 is sent to the ordinary transfer engine. DRM/encrypted-license streams are reported unsupported; there is no key acquisition or decryption path. Live streams, expiring signed URLs and site-specific extraction can be added as separate, tested capabilities, never as a universal guarantee.

## Queue, scheduler, categories, and desktop

Queues use `immediate`, `normal`, and `low` priority with stable FIFO order within a class, per-queue simultaneous-task limits, and a separate global active-task cap. Manually started immediate tasks may preempt only at safe segment/checkpoint boundaries. Named ordinary queues remove completed work; a synchronization queue retains watched URLs and rechecks validators on a configured interval. Schedule records specify one-off/local-calendar recurrence, timezone, next-run instant, stop cutoff, retry policy and missed-run behavior. Compute next runs in a timezone-aware library and persist execution IDs to avoid duplicate starts after clock changes or restart. Completion actions (run command, sleep, restart, shutdown) are opt-in, show a cancelable countdown, and operate only when the selected queue's configured success/failure rule is satisfied. Do not execute them during fixture tests.

Category routing has explicit ordered rules over extension, MIME and optional site pattern; user choice overrides auto-routing. Built-in classes include Compressed, Documents, Music, Programs and Video. Paths are normalized per OS; conflict choices are rename, overwrite after confirmation, or use existing completed file when verified. The UI provides a task table with progress/ETA/speed, per-segment activity, category and queue tree, detailed progress, context actions, search, import/export, scheduler, grabber and settings. The visual language may be familiar but uses original icons, text and layout assets. Feature parity is tracked by the audit matrix rather than copying all 105 dialog templates.

## Proposed repository layout

```text
crates/
  xdm-model/          # versioned task, queue, browser and media types
  xdm-http/           # independent HTTP probe, range scheduler, limiter, checkpoint
  xdm-media/          # clear HLS/DASH parsing, variant graph, segment fetch, mux hook
  xdm-store/          # migrations, task persistence, secrets and sidecars
  xdm-service/        # queue, scheduler, categories, application commands/events
  xdm-ipc/            # local authenticated protocol and native-message framing
  xdm-daemon/         # long-running owner process
  xdm-cli/            # transfer and task control without a GUI
  xdm-native-host/    # stdin/stdout bridge only
apps/
  crates/xdm-gui/     # native Slint queue-client prototype; toolkit benchmark pending
extensions/
  chromium/           # MV3 manifest and browser adapter
  firefox/            # Firefox manifest and adapter
  shared/             # original schema/client logic only
install/
  linux/ macos/ windows/  # native-host registration and packaging
tests/
  fixtures/           # deterministic HTTP and clear media servers
  integration/        # daemon, CLI, IPC, browser and GUI smoke tests
docs/                 # audit, contracts, test matrix and user documentation
```

## Performance-first decision gate

The language alone cannot guarantee the lowest resource use. The daemon will keep a fixed-size I/O buffer pool, bound concurrent sockets, avoid whole-file/media buffering, batch database writes and progress events, and sleep when idle. The task table will use a model/view data source rather than one widget per cell, repaint only changed rows, and coalesce progress updates. The native host will be short-lived and will not load the desktop toolkit. FFmpeg runs only for an explicit remux/transcode job.

Before committing to the GUI toolkit, build the same representative shell in Qt Widgets and Slint: 10,000 task rows, live progress updates, category/queue tree, context menu, settings dialog, tray behavior, and accessibility navigation. Compare **whole process-tree proportional set size (PSS) on Linux and private bytes/working set on Windows**, idle CPU and wakeups, scrolling/frame latency, startup time, binary/dependency size, and performance during four concurrent downloads. Measure with identical release builds on specified reference hardware, cold and warm starts, repeated runs and medians. Choose the toolkit that meets functionality and has the best measured memory/CPU profile; do not infer that smaller installer size means smaller RAM use. Qt Widgets is a mature native model/view baseline; Slint offers a native Rust UI with selectable backends/renderers. [Qt model/view](https://doc.qt.io/qt-6/modelview.html), [Slint backends](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/).

The Rust core and a C++ Qt UI, if selected, communicate through the existing local IPC rather than an in-process FFI layer. This costs one additional client process but keeps the daemon small when no GUI is open and isolates UI crashes. A Rust/Slint UI would still be a separate client for the same reason. Any claim that Qt, Slint, Rust or C++ is categorically fastest awaits this benchmark. Qt's open-source modules have license conditions that will be checked before packaging; the baseline uses only needed Widgets/Gui/Core modules. [Qt licensing](https://doc.qt.io/qt-6/licensing.html).

## Milestones and acceptance gates

| Milestone | Deliverable | Required evidence before proceeding |
|---|---|---|
| 0. Contract lock | Review architecture; approve Rust engine and Linux/Windows priority | User authorized implementation; completed |
| 1. CLI transfer engine | GET/HEAD probing, single and 4–16 range streams, pause/resume/checkpoint, retries and limits | Unit tests for partition math/range parsing/retry state; local range, no-range, malformed 206, 403/503, changed validator, crash/restart hashes; >4 GiB sparse fixture |
| 2. Daemon and task service | Durable store, priority/named queues, schedule, categories, local IPC | Task/queue restart persistence, one-vs-multiple concurrency, missed schedule/DST fixtures, peer-auth test |
| 3. Native host and browser | Chromium/Firefox handoff, context menus, media candidate capture, registration installers | Frame boundary/partial-I/O tests; browser integration on Linux and Windows, accepted/canceled/fallback/timeouts, cookie/referrer/redirect fixtures |
| 4. Clear media | HLS/DASH variants, audio selection, segment assembly, FFmpeg hook | Synthetic master/media MPD playlists, all advertised qualities, separate audio, multi-language, discontinuity, valid playable output, protected stream refusal |
| 5. Desktop and polish | Native toolkit benchmark, then task table, dialogs, tree, progress, settings, queue/scheduler, import/export, grabber | Whole-app resource benchmark and GUI smoke/interaction tests on Linux and Windows; macOS compatibility if retained; accessibility, appearance and data migration checks |

The local fixture and original-product tests above define reference behavior, not code to transplant. Native Windows is needed to validate **our** Windows integration and compare Windows-only original features, but it is not needed to finish this Linux artifact audit or begin a platform-independent engine after approval.
