# IDM desktop and browser integration investigation

Status: supplied-artifact audit completed within the Linux/Wine scope on 2026-10-12. This is an evidence inventory, not a claim that every IDM behavior has been reproduced or tested. The [completion report](idm_audit_completion.md) identifies platform-dependent validation still required.

## Scope and evidence rules

The target is the supplied IDM 6.43 build 15 installer and the separately supplied Integration Module 6.43.1 CRX. Analysis is performed on Linux. Original binaries are not modified. The following investigation notes describe the audit before implementation began; current XDM implementation status is tracked in [the feature checklist](implementation_status.md) and [coverage ledger](feature_coverage.md).

Findings use these evidence levels:

- **Static:** directly present in executable resources, imports, exports, strings, installer payload, or extension source.
- **Documented:** described by help shipped inside this installer. Help can contain historical behavior and screenshots.
- **Derived:** an interpretation of static evidence; explicitly distinguish it from executed behavior.
- **Runtime:** observed while running this exact payload; record the environment, trigger, and result.
- **Unknown:** requires additional tracing or execution. A resource's presence does not prove that its screen is reachable.

The inventories intentionally retain inactive, historical, and empty resources. They must not all be treated as requirements for a new application.

## Input identity and extraction

| Input | SHA-256 |
|---|---|
| `IDM/idman643build15.exe` | `a902e77169a3e4d8a88ee2ccaab11d3d43ab209483625a3602438ed4a22bbfd9` |
| `IDM/NGPAMPAPPNMEPGILOJFOHADHHMBHLAEK_6_43_1_0.crx` | `1d2297ccbdbbf06e9e2546bb9f9b9985433eb7c02d75f09bbcbc9784ece583f7` |

The installer is a PE32 x86 GUI executable whose version metadata identifies it as an installer, not the download-manager application. Its embedded payload starts at file offset 95,232. Sequential extraction found 192 compressed records: an installation manifest and 191 named files. Every record was decompressed with zlib integrity checking; the manifest's numbered file names were mapped to record numbers. Records end at offset 12,548,643, leaving 10,685 bytes outside the parsed records. The PE security directory is present; certificate trust has not been verified.

A normal `7z l` exposed only an embedded ZIP fragment and reported a header error. That was not sufficient to recover the application. The successful extraction used the installer record structure: 32 bytes of per-record header followed by a zlib stream, with compressed size from the second 32-bit header word. Other header words have not been assigned semantics.

The CRX is version 3. Its ZIP begins after the CRX header and its declared protobuf header length. Extraction did not execute extension code.

## Evidence artifacts

| Artifact | Contents |
|---|---|
| [Payload inventory](analysis/idm_payload_inventory.json) | All 192 records, original names, offsets, sizes, hashes, and import/export/resource inventories for 47 PE files |
| [UI resource inventory](analysis/idm_ui_resources.json) | All 105 dialog templates, 1,055 controls, two menu trees, and 156 populated PE string entries in `IDMan.exe` |
| [Language catalog](analysis/idm_language_catalog.json) | 1,604 functional entries from the shipped translation template, including dynamically used labels and messages |
| [Dialog catalog](analysis/idm_dialog_catalog.md) | One row per dialog, resource identity, title, control count, and representative controls |
| [Runtime evidence index](analysis/runtime/index.json) | 38 selected screenshots, environment identity, output hashes and coverage limits |
| [HTTP request capture](analysis/runtime/http_requests.jsonl) | Requests and fixture transfer events from the original application |
| [Browser/media follow-up](idm_browser_media_analysis.md) | Extension component assertions, synthetic media downloads under Wine, and selected native host/parser/muxer decompilation |
| [Browser/media evidence index](analysis/browser_media/index.json) | Persisted result JSON, HTTP logs, fixture hashes, screenshot and native function metadata |
| [Native core follow-up](idm_native_core_analysis.md) | 19 string-reference-selected main-executable functions, HTTP range/parser paths and partial-state path construction |
| [Final runtime/package evidence](analysis/runtime_final/index.json) | Queue concurrency, fault/retry/validator outcomes, import/export, restart persistence, and installed extension package inventory |

UI records retain resource IDs, language IDs, data RVAs, file offsets, styles, fonts, classes, and dialog-unit bounds. These are local investigation evidence, not UI assets or code for redistribution in the new project. Original images, binaries, extension scripts, and help text remain outside the proposed implementation.

Temporary extraction location for this investigation: `/tmp/xdm-idm-desktop-18i671p5`. Temporary extension extraction: `/tmp/xdm-idm-inspection-3ifum5xo`. A reboot or temporary-directory cleanup can remove these files; input hashes and the persisted inventories allow target identity to be checked again.

## Tool suitability

| Tool | Actual use and suitability |
|---|---|
| `7z` | Archive inspection and extraction of embedded CHM help; not sufficient by itself for the installer payload |
| Python + zlib | Installer record recovery and integrity checks |
| `pefile` | PE import/export/resource inspection and binary-offset mapping |
| `objdump` | x86 disassembly, including targeted native-host I/O and pipe-opening paths |
| Resource parser | Parsed standard and extended Windows dialog templates, standard menu templates, and string tables; zero parse errors in the three inspected executable resource inventories |
| REA | Its local checkout was inspected for tool capabilities. Its compiled runtime and dependencies are absent; it has not been executed |
| Ghidra | Portable 12.1.4 with private JDK 21 performed native decompilation: 83 host functions, nine selected mux/conversion functions, three selected parser functions, and 19 string-reference-selected IDMan.exe functions; recovered pseudocode remains in private temporary storage |
| Hermes decompiler | Not used. The supplied extension has JavaScript source, not `.hbc`; the recovered desktop components are native PE binaries. No Hermes payload has been identified |
| Wine | Ran the installer and installed application in a private prefix and Xvfb display; confirmed installed application hash matches the recovered payload. A portable Wine 11.0 WoW64 build was used; Openbox was added to the private display for reliable window management |

Resource extraction is not machine-code decompilation. Targeted disassembly is not a full recovery of the core algorithm. These distinctions remain visible throughout this report.

The later Ghidra investigation established session-specific native pipe forwarding and selected parser/muxer wrapper structure. It does not constitute a complete recovery of the main application's download scheduler. See the follow-up for exact scope and addresses.

## Recovered component boundaries

| Component | Static evidence | Interpretation and remaining uncertainty |
|---|---|---|
| `IDMan.exe` | 6,201,712-byte native x86 PE; 105 dialogs and two menus; no CLR directory | Desktop application containing the main task UI; internal subsystem ownership still needs call tracing |
| `IDMMsgHost.exe` | stdio manifests; Win32 read/write, pipe, thread, and COM imports | Browser Native Messaging bridge to desktop integration infrastructure |
| `IDMNetMon.dll`, 64-bit and ARM64 counterparts | `ControlMonitoring` export; architecture-specific binaries | Monitoring subsystem; exact capture behavior is not established by its filename or export alone |
| `idmwfp*.sys`, `idmtdi*.sys` | Driver files plus INF/CAT records | Windows-specific integration exists beyond the browser extension; drivers cannot be functionally validated through Wine |
| `IDMIntegrator64.exe`, `IDMIntegratorAA.exe` | Distinct PE executables | Architecture-specific integration helpers; workflows require tracing |
| `IDMShellExt*.dll`, `idmbrbtn*.dll`, IE helper DLLs | Separate x86, x64, and ARM64 shell/browser components | Shell and legacy browser integration are separate surfaces |
| `idmBroker.exe`, `MediumILStart.exe` | Broker and launch-helper executables | Process/privilege boundary helpers; precise authority checks are not yet established |
| `IDMVMPrs*.dll` | Exports include `GetMPDParserInfo`, `Parse1`, `UTF8ToHtml` and XML parser functions | Manifest/parser functionality exists; exact DASH profile coverage remains unknown |
| `idmvconv.dll` | Conversion and muxing exports for TS, AAC, MP4, FLV, MKV, and WebM audio | Separate conversion/muxing component, important for video plus audio assembly |
| `libcrypto.dll`, `libssl.dll` | Separate crypto/TLS libraries and license file | TLS/crypto dependencies; their presence is not evidence of DRM support |
| `IDMFType.dat`, `idmfc.dat`, exception list | Shipped classification/configuration records | Data-driven detection exists; formats need separate analysis |
| CHM help and language files | Main, tutorial, scheduler, and grabber documentation; many locales | Functional and UI evidence, with historical content mixed into the shipped build |

## Main window and interaction inventory

Primary screen resource: **D102**. Static controls include a list view, category tree, splitter-related controls, and category label. Runtime list columns and toolbar buttons are assembled dynamically and also appear in the language catalog and main help.

The layout evidenced by shipped help is menu bar, toolbar, category/queue/project tree on the left, and a sortable download table on the right. The included main-window screenshot identifies itself as version 5.11, so it does not establish the exact 6.43 appearance. The current executable's menu includes dark-mode support, font selection, and a Neon toolbar entry absent from that old screenshot.

| Surface | Evidence | Functionality to verify at runtime |
|---|---|---|
| Task table | D102, catalog 20369–20375, main help | Queue marker, name, status, size, time remaining, rate, description; sorting, selection, keyboard navigation, column widths |
| Additional columns | D209, M129 arrange-files submenu | Last attempt, save path, referrer, parent page; show/hide/reorder columns |
| Toolbar | Language catalog and main help | Add URL, resume, stop, stop all, delete, remove completed, options, scheduler, start/stop queue, grabber; enabled states depend on selection |
| Task context menu | Main help and catalog | Pause/stop, start, redownload, properties, open file/folder, queue membership, refresh address; dynamic menu assembly is not fully recovered |
| Category tree | D102, D204, main help | Built-in file categories, unfinished/completed filters, named queues, saved grabber projects |
| Category context menu | Main help, D204 | Create/edit/delete category; extension/site patterns and destination rules |
| Queue context menu | Scheduler help | New/edit/delete/schedule/start/stop queue; commands and enabled states need runtime capture |
| Tray | M129 tray settings and main help | Tray presence/style, background operation, queue control; dynamic tray menu still needs observation |
| Drop target | D187, main help | Separate floating window accepts dragged URLs; show/hide through menu or tray |
| Search | D487, D505, D506; M129 commands 34040/34041 | Search bar/dialog, next result, name/description/site/URL/page/referrer search, case and whole-string matching |
| Appearance | M129, D209, catalog | Toolbar size/order/theme, tray style, language, fonts, dark mode; current rendering needs execution |

### Static main menu tree

Menu **M129** contains six top-level branches:

- **Tasks:** new download, generated batch, clipboard batch, site grabber, drop target, export to IDM/text, import from IDM/text, exit.
- **File:** stop, remove, download now, redownload.
- **Downloads:** pause all, stop all, remove completed, find/find next, scheduler, start/stop named queue submenus, speed limiter on/off/settings, options.
- **View:** category visibility, ordering by task fields, toolbar customization/size/theme, tray style, URL-list customization, dark-mode support, font selection/reset, language.
- **Help:** contents, tutorials, scheduler/queue help, grabber help, tips, homepage/support, updates, about, tell-a-friend.
- **Registration:** purchase and registration UI.

The authoritative JSON menu tree records the command IDs, nesting, separators, and static enabled flags. Dynamic queue and language entries need runtime observation.

Menu **M256** is the grabber project menu: new/load/save project, save settings as a template, recent projects, close, grabber settings, project schedule, and help.

## Download lifecycle, files, and recovery

| Capability | Evidence | Established behavior or outstanding question |
|---|---|---|
| Manual URL and credentials | D148 | URL history field and optional server login/password |
| Download confirmation | D130 | URL, category, destination, description, start/later/cancel; unknown-size and preview affordances need runtime observation |
| Download while confirmation is visible | D291, main Options help | Documented optional eager transfer before the user finishes choosing the destination; cancellation cleanup needs testing |
| Deferred tasks | D130, D291, D345 | Add later, optional queue selector, optional immediate queue start |
| Progress | D103, D363 | Status, byte totals, rate, ETA, resume capability, per-connection details and segment visualization |
| Pause/stop/resume | D103, main help | Documented persistence and server-dependent resumption; exact state transitions and restart behavior untested |
| Completion actions | D364 and catalog | Open/file-folder actions, optional completion dialog, modem hang-up, app exit, power action |
| File properties | D135 | Destination, source URL, description, origin page/referrer, credentials, last attempt/result |
| Duplicate detection | D298, D291 | Numbered duplicate, overwrite, or show completed/resume existing; duplicate key and detection timing unknown |
| Delete semantics | D338/D339 | Remove tasks versus optionally delete completed files; incomplete deletion confirmation |
| Redownload | M129 command 33009 | Separate restart command; overwrite and partial-file cleanup need testing |
| URL/session refresh | Catalog 20872/20877 and binary strings | User-facing path to refresh expired download addresses; replacement matching and byte reuse unknown |
| ZIP preview | D196 | Archive member list; supported archive/range conditions unknown |
| Batch generation | M129 command 34024, catalog | Parameterized batches; complete syntax and expansion order need runtime capture |
| Clipboard batch/import/export | M129, D191, shipped help | Text/IDM imports, selected/all/queued exports; persisted file formats not decoded yet |
| File timestamps | D143/D291 | Server creation-date option and optional ignore-modification-time behavior on resume |
| Antivirus hook | D494 | Configurable scanner executable and parameters; launch timing/result handling unknown |
| Sounds | D195 | Event selection, file choice, test playback; full event list dynamically populated |

### Transfer algorithm evidence

`IDMan.exe` contains HTTP range and validator strings including `Accept-Ranges`, `Content-Range`, `Content-Length`, `If-Range`, `If-Modified-Since`, and `ETag`. This demonstrates recognition of those protocol concepts, not correct behavior for every server response.

The shipped `Segmentation.htm` describes allocating a free connection to half of the largest remaining segment, reusing completed connections, avoiding excessively small splits, assembling segments at completion, and periodically saving segment positions. This is **documented behavior**, not yet verified by packet capture or core-function decompilation. In particular, it does not establish that IDM writes all segments directly into one preallocated sparse file.

Outstanding core experiments: HEAD rejection, GET-only servers, ignored Range responses, inconsistent Content-Range, changed validators, missing lengths, disconnects after partial bodies, retry backoff, stall redistribution, compressed representations, redirects, expiring signed URLs, disk-full handling, kill/restart recovery, and files over 4 GiB.

## Options: every recovered settings family

| Family | Resources | Settings and related dialogs |
|---|---|---|
| General/integration | D139, D231, D229, D296/D297 | Startup, clipboard monitoring, browser list/add browser, legacy IE monitoring, advanced integration, force/bypass keys, context-menu configuration, download panels |
| File types/exclusions | D294, D302–D305 | Captured extensions, site exceptions, address exceptions, twice-cancelled-download prompt, takeover-failure notice |
| Connections | D141, D226 | Default connection count, per-server pattern overrides, transfer quota and warning, experimental Microsoft TLS 1.3 option |
| Save paths/categories | D143, D204 | Category directories/types/sites, last-path memory, general category rules, temp directory, server file creation date |
| Download dialogs/defaults | D291, D298, D367 | Start/complete dialogs, eager transfer, queue-only mode, queue selectors, duplicate policy, manual User-Agent, progress-window/tab appearance, resume timestamp option, virus checking |
| Proxy/SOCKS | D140, D369 | None/system/PAC/manual, per-protocol proxy, credentials, bypass patterns, browser-request proxy fallback, SOCKS4/5, proxy DNS, FTP passive mode |
| Site logins | D145/D146 | Server/path-scoped credentials; documented registered-product restriction |
| Dial-up/VPN | D184–D186 | Windows connection selection, credentials, apply/save password, redial count and delay |
| Sounds | D195 | Download-event sounds and playback test |
| Speed limiting | D337, D365 | Application setting described as a maximum for one file; per-download cap and persistence across stop/resume |
| Multimedia panels | D340/D341/D370/D488 | Full/mini panel, allowed file types, minimum media size, site exclusions, automatic player capture toggle, optional protected-content notification |
| Selected-link panels | D348/D349 | Always/never/site-specific panel policy, full/mini display |

Do not equate the application's speed-limiter preference with an aggregate global token bucket: D337 explicitly describes a per-file maximum. A true aggregate cap in the new implementation would need to be specified separately after this investigation.

## Queues, scheduler, and completion actions

Resources: **D306** schedule, **D307** queue contents, **D308** scheduler shell, **D309** usage limits, **D310** queue naming, **D332** scheduler-running reminder.

The shipped scheduler help establishes two default queues—ordinary downloads and synchronization—and additional user-defined queues. Completed ordinary downloads leave their queue. Synchronization entries remain and periodically check whether server content changed. Custom queues can switch modes; help describes the default queues as fixed in their respective modes.

| Capability | Evidence | Detail |
|---|---|---|
| Queue ordering | D307 and scheduler help | Move selected files with arrows or drag; remove from queue; drag between queues |
| Per-queue concurrency | D307 | Number of simultaneous files per queue; separate from connections per file/server |
| Manual controls | D308 and menu/help | Start now and stop through scheduler, toolbar, main menu, queue context menu |
| Calendar schedule | D306 | Start time, once/date or daily/weekdays, independent stop time |
| Periodic synchronization | D306 and scheduler help | Repeat interval in hours/minutes; update changed remote files |
| Start on application startup | D306 | Separate scheduling option |
| Retry policy | D306 and scheduler help | Failed file rotation, optional retry limit; help describes infinite final-file retries at 30-second intervals |
| Usage quota | D309/D141 and help | MB per hour-window, warning, later resumption; algorithm/window boundaries untested |
| Queue membership | Main/scheduler help | Default assignment, queue markers, drag/drop; documented restriction on completed files entering ordinary queues |
| Persistence | Scheduler help | Order/content saved immediately; other settings saved on apply, queue change, or close |
| Completion program | D306 and help | Open a configured file/program after queue processing |
| Modem/app/power actions | D306/D364/D150 and catalog | Disconnect, exit, shutdown; catalog also includes restart, sleep, hibernate and countdown messages |
| Cancel completion action | D150/D292 and catalog | Countdown and cancel/stop UI; no power action has been executed during investigation |

The help says manually stopping queue processing suppresses queue-completion actions. Exact behavior with failures, concurrent queues, and periodic synchronization remains untested. Clock changes, DST, sleep/wake, and missed scheduled starts also require experiments.

## Site grabber: separate feature, not merely a batch downloader

Resources: wizard shell **D239**, result window **D241**, start/auth **D257**, save rules **D259**, exploration rules **D264**, file filters **D240**, settings **D262**, filter editors **D244/D245**, plus M256 and grabber help.

1. **Project/start page:** project name, initial URL, templates, credentials, optional browser-assisted manual login, logout-page exclusions.
2. **Destination:** category-based folders, one category, or one directory; original relative subfolders; local-link rewriting; overwrite policy; add checked results to the main queue on close.
3. **Exploration:** whole site or depth limits, separate internal/external levels, parent-directory restriction, same-domain scope, popup exclusion, JavaScript processing, include/exclude paths.
4. **Download filters:** included/excluded file masks, same-site-only scope, duplicate hiding, automatic matched-file download, path/domain rules, min/max size.

Result window: URL/site hierarchy, file table, check/uncheck selections, explore/download start-stop controls, add results to the main task list, statistics, file properties/open actions, and return to prior wizard stages without losing project state.

Project persistence: save/load/recent projects, named templates, saved-project nodes in the main category tree. Each project also has an independent schedule, one-off exploration/download modes, periodic synchronization, stop time, completion program, and power actions.

The shipped help describes grabber retries differently from ordinary queue retries: two attempts in the grabber versus repeated retries in the main queue. It also documents historical IE cache reuse. Current applicability of those details needs runtime verification.

## Browser extension protocol and interception

### Manifest and platforms

The supplied CRX manifest declares MV3, `background.js` as service worker, `content.js` at document start in all HTTP(S) frames, and web-accessible `document.js`, `debug.js`, `captured.js`, and `captured.html`.

Declared permissions include native messaging, downloads, cookies, context menus, webRequest, declarativeNetRequest, scripting, navigation, tabs, proxy, storage, management, and download-UI controls. It requests all hosts. Optional permissions include notifications and display information.

The background initialization returns early on non-Windows platforms; the content script is also gated on a Windows platform string. Cross-platform support is not present in this supplied IDM implementation.

### Two transport layers

Browser Native Messaging serializes a JSON value as UTF-8, prefixed by an unsigned 32-bit byte count in native byte order. The prefix counts encoded bytes, not characters. Browser-generated framing is outside extension JavaScript. This is the [browser protocol](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging), not IDM's application schema.

`background.js` tries a WebSocket at `127.0.0.1:1001`, then `0.1.0.1:1001`, using subprotocol `plugin.v3.internetdownloadmanager.com`; another attempt uses `runtime.connectNative("com.tonec.idm")`. Do not treat the second IP literal as proof of a portable localhost convention.

The native-host manifests recovered from the installer identify `IDMMsgHost.exe`, stdio transport, two Chromium extension origins, and Firefox ID `mozilla_cc3@internetdownloadmanager.com`.

Targeted `IDMMsgHost.exe` disassembly corroborates the desktop bridge: the UTF-16 pipe-name prefix `\\.\pipe\IDMNetworkMonitor.` is at file offset `0x351c` (VA `0x40471c`), standard-input/output handles are obtained near VA `0x402b94`, and `CreateFileW` opens the constructed pipe near VA `0x402bd1`. Two worker threads use the I/O routine at VA `0x4034a0`; a `CoCreateInstance` call is visible near VA `0x403780`. These addresses refer to this recovered host binary only. The later native decompilation traced the session suffix and JSON/pipe translation; complete pipe framing and peer authentication remain unresolved.

IDM serializes an inner record of the form `MSG#sequence#type#variant#flags`, optional colon-delimited numeric arguments, comma-delimited numbered fields, and a semicolon terminator. Strings are byte-length-prefixed inside that inner grammar; binary handling varies by transport/protocol version. The native port posts this as a **string**, so the outer Native Messaging payload is a JSON string, not an object with friendly property names. `qb`, `S`, and `zb` in the original background source establish this distinction.

### Observed message families

| Inner type | Observed use | Evidence |
|---|---|---|
| 0 | Keepalive path | `Ma` |
| 1 | Reply/acknowledgment family | `$a`, `mc` |
| 2 | Initialization/handshake request | `Na` |
| 3/4 | Initialization response and configuration | `$a` dispatch |
| 8/9 | Media/panel visibility and geometry | `ba`, `Fb`, `nc` |
| 13 | Observed network/download handoff and related replies | `X`, `lc`, `$a` |
| 14 | User-requested link download, also link-list entries | `pa`, `ec` |
| 15 | Multi-link batch initialization | `ec` |
| 16 | Captured response content transfer | `Bb`, `Yb`, `ub` |
| 18 | Host-supplied site matching configuration | `$a` |
| 19 | Site-specific extracted metadata | `Kb` |
| 21/23 | Page/media metadata and identifiers | `ec`, `Hb` |

This is a bounded observed subset, not a complete formally recovered protocol. Numeric flags and every family variant remain partially unknown.

### Observed handoff fields

| Numbered field | Observed contents | Evidence |
|---|---|---|
| 6 | Download/resource URL | `pa`, `X`, `lc` |
| 7 | Parent/origin page URL | `pa`, `X` |
| 17 | Original/previous resource URL in redirect/correlation paths | `X`, `lc` |
| 50 | Referer / source page context | `pa`, `X`, `lc` |
| 51 | Serialized cookies | `pa`, `ec`, `X`, `lc` |
| 54 | Browser User-Agent string | `pa`, `ec`, `X`, `lc` |
| 56 | Cookies for original URL when final URL differs | `lc` |
| 11 / 13 / 18 | Current request, response, and earlier request headers | `X`, header serializer `Kc` |
| 14 / 19 | Request body and earlier body | `X`, body serializer `Bc` |
| 100 | Filename/label in link/download paths | `pa`, `lc` |
| 103 | Browser-reported total bytes | `lc` |
| 4 / 127 | File classification/extension and MIME metadata | `X`, `lc` |
| 121 / 122 | Proxy configuration/context in different paths | `pa`, `ec`, `X`, `lc` |

These meanings are specific to observed producers; they are not universal declarations for every occurrence of the field number. Cookies are obtained for individual URLs and serialized as name/value pairs. Captured headers are serialized as CRLF-separated lines. POST bodies have raw/form/multipart handling. Replay safety and partitioned-cookie fidelity are not established.

### Browser download handoff sequence

The Chromium path registers `downloads.onCreated` and `downloads.onDeterminingFilename`. Eligible creation events are correlated with recent request records where possible and paused. Filename determination checks integration/filter rules, collects request context or cookie/referrer fallbacks, and sends type 13. The reply path calls `kc`: cancel the browser task when accepted, or invoke the deferred filename callback and resume on fallback.

The `onDeterminingFilename` hook does not itself cancel downloads. The original code explicitly uses pause/cancel/resume calls. Current [Chrome API requirements](https://developer.chrome.com/docs/extensions/reference/api/downloads) require each filename listener to call its suggestion callback once; deferment returns true.

Request observation uses before-request, before-send-headers, headers-received, response-started, error/completion, and navigation hooks. Cookies/request headers use the extra-headers option where selected by the original code. Session declarativeNetRequest rules redirect configured main-frame matches to `captured.html`.

The source contains Firefox-oriented cancellation branches, but a normal Chromium MV3 build cannot rely on unrestricted blocking webRequest listeners. [Chrome documents this limitation](https://developer.chrome.com/docs/extensions/reference/api/webRequest). The supplied manifest's presence does not establish what a separate Firefox build supports.

## Media discovery, player panels, and video/audio assembly

Media classification uses URL/site patterns, MIME/extension tables, response headers, and request types including media, XHR, objects, and other resources. HLS/DASH related classifications include M3U8, MPD, M4S, MPEG-TS, MP4, WebM, AAC/M4A and MP3. Host-provided site patterns supplement the shipped code.

`document.js` runs in page context and hooks configured XHR/fetch/Response methods. `window.postMessage` carries captured responses or page-derived metadata back to the isolated content script, which forwards them to the background/native side. This is not just an extension scan of `<video src>`.

Content scripts inspect `video`, `audio`, `object`, `embed`, nested frames, and source children; use mutation/resize/intersection observation; and calculate visible player bounds. Media element IDs, page titles, source URLs, frame geometry, and visibility are reported. The background sends media-panel geometry to the desktop.

**Derived:** the floating IDM panel appears to involve desktop-rendered UI coordinated with page geometry. No DOM implementation of the entire IDM panel has been established from the supplied content script. Do not claim it is a simple injected button without runtime/window tracing.

`idmvconv.dll` exports include TS+AAC→MP4 conversion, MP4/MKV and MKV/MKV muxing, TS stream combinations, FLV→MP4 conversion, and WebM audio normalization. This directly supports separate video/audio processing as a subsystem; it does not prove support for arbitrary output codecs or MP3 transcoding.

The language catalog explicitly contains an audio-only track choice and protected-stream refusal messages. Thus the target does not support every video. IDM also states that some protected protocols are unsupported in its [video FAQ](https://www.internetdownloadmanager.com/register/new_faq/embed-link.html).

Outstanding media experiments: enumerate all available qualities versus only played representations, identify separate audio/video pairing, adaptive track switching, audio languages, codecs/containers, expired URLs, HLS master and media playlists, DASH representations, discontinuities, live streams, login-required pages, MSE/blob attribution, overlay positioning/fullscreen, and protected-content notifications. DRM decryption is outside this project's scope.

## Runtime verification matrix

No entry in this matrix is native-Windows verified. The Wine experiments below establish application behavior under Wine. Windows drivers and actual browser/process integration remain separately unresolved.

### Completed Linux/Wine experiments

| Experiment | Observed result | Evidence |
|---|---|---|
| Installation and launch | Installer completed; main 6.43 window, category tree, Neon toolbar, drop target and startup dialogs appeared | `runtime/idman-start.png`, installed payload hash check |
| Settings inspection | All nine settings families were reached; actual defaults included eight connections per file, four simultaneous queue files, and disabled usage quota | `runtime/verified-options-*.png`, `runtime/scheduler-files.png` |
| Integration settings | Reached force/bypass key settings, multimedia panel settings, and selected-link panel settings. This does not prove actual browser capture | `runtime/integration-*.png` |
| Accelerated file download | Downloaded deterministic 64 MiB content; exact output hash matched expected content | `runtime/index.json`, HTTP capture |
| Probing and splitting | First experiment used an initial GET, then eight Range GETs; no HEAD occurred. Requests increased as the transfer progressed and partitions were subdivided | HTTP capture for `/range.bin` |
| Eager download | Transfer began while the Download File Info dialog was still open, before Start Download was pressed | `runtime/download-info.png` and earlier requests |
| Destination choice | Missing destination triggered a create-folder prompt; selected path became the general-category remembered path | Download confirmation/complete captures |
| Pause and resume | Paused during redownload; workers disconnected; resume issued requests starting beyond byte zero | `runtime/range-paused.png`, `runtime/range-resumed.png`, HTTP capture |
| Partial-state storage | Four separate segment files plus a task `.log` were present after pause under `DwnlData`; this observed run did not use a single shared sparse output file | `runtime/paused-file-inventory.json` |
| Forced process termination | Terminated only the owned IDM test process, relaunched it, recovered the task and resumed remaining ranges. Final 64 MiB hash matched | `runtime/crash-experiment.json`, `runtime/recovered-task-list.png`, `runtime/range-recovery-result.json` |
| Server without usable ranges | Initially used a single full GET. After a deliberate server interruption, IDM attempted range resumption; full 200 responses did not satisfy it, and it displayed an explicit non-resumable-transfer error | `runtime/no-range-progress.png`, `runtime/no-range-after-disconnect.png`, HTTP capture |
| Queue restart of non-resumable task | Starting the main queue restarted the failed task and completed it with exact content hash. Resume rejection did not silently corrupt the output | `runtime/no-range-queue-retry.png`, `runtime/index.json` |
| Task context menus | Completed and incomplete tasks exposed different enabled actions: open, move/rename, redownload/resume, refresh address, queue membership, configurable double-click action and properties | `runtime/task-context-menu.png`, `runtime/failed-task-context-menu.png` |
| Grabber wizard/crawl | Reached all four wizard stages; explored the local HTML fixture and listed the HTML page plus both linked binary files | `runtime/grabber-step*.png`, `runtime/grabber-results-managed.png` |
| Grabber settings/schedule | Reached settings with exploration/download concurrency four; schedule requested saving a named project first, then exposed project schedule/action selection | `runtime/grabber-settings.png`, `runtime/grabber-save-project.png`, `runtime/grabber-scheduler-saved.png` |
| Direct media | Original CLI downloaded the synthetic MP4 exactly; FFprobe identified video and audio | `browser_media/manual_media_downloads.json` |
| Manual streaming-manifest URL | Ordinary CLI path saved HLS/DASH manifest bytes rather than fetching and merging segments. Extension-driven streaming remains untested | `browser_media/manual_media_downloads.json` |
| Native host execution | Controlled framing client under Wine received no reply; actual browser-to-desktop integration was not established | `browser_media/native_host_result.json` |

Separately, the original extension background source passed 15 assertions with mocked browser APIs and simulated host acknowledgments. One pending-disconnect case remains a bounded observation. These are component experiments, not entries proving runtime browser integration; see [the follow-up](idm_browser_media_analysis.md).

Both verified binary outputs were 67,108,864 bytes, SHA-256 `281e519df3077b557c6b03f5da83c4e8d397219259615dd7c3308f89cae8f2a6`. This verifies these particular fixture experiments; it does not establish correctness for arbitrary servers or power-loss durability.

The observed Range requests include small boundary overlaps. Their presence should not be replaced by an assertion of mathematically disjoint requests; the exact overlap/validation purpose is not yet traced. No `If-Range` guarantee should be inferred from the earlier static string evidence alone.

The default Wine identity caused IDM to display an administrator warning and disable advanced integration. This is an environment-specific restriction on this run, not evidence that normal Windows integration fails. Initial captures were taken without a window manager and contain incomplete redraws; later managed captures improved this. No shutdown, restart, hibernate, or sleep action was executed.

### Final controlled-runtime pass

These later observations supersede the earlier "untested" notes for the specific fixtures below. The results and selected screenshots are indexed in [runtime_final/index.json](analysis/runtime_final/index.json).

| Experiment | Original behavior under Wine | Limit |
|---|---|---|
| Named queue and concurrency | Created an Audit queue; changed Main queue concurrency from four to one. Three deferred 8 MiB files made their first requests strictly after the preceding task's last response, and all three hashes matched. | One local server and one queue setting; priority or cross-queue scheduling was not exercised. |
| Invalid `Content-Range` | The deliberately inconsistent partial response caused IDM to abandon that path, alert about connection corruption, and finish through a full GET; final hash matched. | One malformed-range variant, not exhaustive validation. |
| HTTP 503 then recovery | Two initial 503 responses were followed by successful 200/206 requests and an exact output hash. | A GUI notice was pending during the interval, so no precise retry-backoff duration is inferred. |
| HTTP 403 | Two 403 responses produced Error state and no completed output. | Other authentication challenges were not exercised. |
| Changed remote object during pause | Paused a version-one task at 9.37%; after ETag and Last-Modified changed, resume prompted that the file date changed and offered restart. Accepting restart yielded exactly version-two bytes. | Both validators changed, so ETag-only behavior is unknown. No `If-Range` was seen in this trace. |
| Text import and exports | Text import previewed two URLs, sizes, file types and destination and added both as deferred tasks. Text export contained one URL per line. Tested `.ef2` export used plain text URL blocks with optional referrer/User-Agent lines. | No full round-trip or all metadata fields established. |
| Category/search/batch/limiter | Video tree filtered the list; Find File exposed search scopes; generated batch exposed `*` substitution; speed-limit settings showed 10 KB/s default and startup toggle unchecked. | Search matching, batch expansion, and actual throttle rate were not measured. |
| Routine restart | All 13 task rows, named Audit queue, and Main queue concurrency value one remained after graceful exit/relaunch. | Separately tested forced termination/recovery covers one partial download, not every setting. |
| Native media DLL exports | Standalone conversion call crashed under Wine; standalone TS+TS mux emitted an MP4 that FFprobe listed but FFmpeg did not cleanly decode. | ABI/context and input expectations are unvalidated. This cannot establish integrated IDM mux quality. |

The installed payload also contains separate Chromium MV3, Edge MV2, and Firefox MV2 packages. Their exact manifests and package hashes are in [browser_packages.json](analysis/runtime_final/browser_packages.json). The Edge and Firefox packages share the native-host name `com.tonec.idm` but have different background scripts and API capabilities; neither was run in a real browser here.

### Platform and edge-case validation still required

| Experiment group | Required observations |
|---|---|
| First launch | Startup dialogs, license/trial UI, tips, tray behavior, integration prompts, defaults |
| Main window | Menus and dynamic popups, toolbar states, sorting, multiple selection, column configuration, categories, search, dark/light rendering |
| Download lifecycle | Direct file → info dialog → progress → pause/resume → completion; close versus cancel versus stop; byte correctness |
| Retry/ranges | Controlled HTTP server faults, range validation, content change, timeout, stalled worker behavior |
| Persistence | Pause/restart and forced termination/restart, partial-file ownership, checkpoint timing |
| Paths/files | Category routing, duplicates, overwrite, missing destination, no space, moving partial/completed files |
| Queues | Multiple named queues, ordering, concurrency, startup schedule, retries, synchronization, manual stop |
| Scheduler | One-shot/day schedules, recurrence, stop cutoff, missed starts, sleep/wake; power actions examined without executing them on the user's machine |
| Integration | Extension available/unavailable, native host activation, browser fallback, context-menu transfer, authenticated redirects, POST handling |
| Media | Clear HLS/DASH/direct media, separate audio/video, quality selection, muxing, failures, overlays |
| Grabber | Four wizard stages, authenticated start, include/exclude/depth rules, project save/load, offline links, project schedules |
| Settings | Each tab and nested dialog, apply/cancel semantics, persistence, proxies, credentials, sounds, scanner hook |

## Residual unknowns after the supplied-artifact audit

1. Runtime reachability of all resources and dynamic menus; full screenshots of the current build rather than historical help images.
2. Exact queue/task state transitions, defaults, retry limits, bandwidth semantics, and behavior under failure.
3. Core-function traces for range scheduling, recovery metadata layout, and file assembly.
4. Full native protocol grammar/flags and the complete authenticated host-to-desktop contract.
5. Media quality enumeration and video/audio pairing rules across supported sites.
6. Windows-driver integration behavior and its cross-platform replacement boundary.
7. Import/export and saved-grabber-project file formats.

The bounded audit and architecture/directory plan are in [the completion report](idm_audit_completion.md) and [architecture specification](architecture_spec.md). The subsequent independent Rust implementation is tracked in [implementation status](implementation_status.md). The [feature coverage ledger](feature_coverage.md) maps the complete parsed UI inventory, and the [dynamic behavior review](idm_behavior_review.md) maps all language entries to reviewed semantic sections while retaining caller and runtime unknowns.
