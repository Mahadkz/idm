# IDM feature-to-XDM coverage ledger

Mapped identities: **105 dialog templates, 1055 controls, 2 menu trees, 75 menu command occurrences**. Every menu container/separator is also retained. There are 84 target feature families and 26 additional non-UI functional paths.

This is complete coverage of the supplied parsed resource inventory, not a claim that every hidden IDM behavior is known or that XDM has reached parity. The machine-readable [ledger](feature_coverage.json) includes one source key for every control and menu node. IDs disambiguate repeated captions and generic controls. Empty/framework templates, unknown fields/callers, and native Windows behavior remain explicit rather than being labeled implemented.

`verified-subset` means local behavior has evidence; `partial` lists remaining work; `pending` has no implementation; `unresolved` requires more behavior evidence; `platform-pending` requires native platform work; `intentional-substitute` identifies proprietary/commercial/legacy surfaces replaced by an independent open-source design. A family's state does not verify each child control. Individual functional controls still need control-level checks; static/display references are recorded as UI evidence.

Original assets/code are investigation inputs only. XDM has original source and UI assets. No DRM decryption is planned. Native Windows runtime validation and real browser/media edge cases remain necessary.

Dynamic text coverage: all 1604 catalog entries and 156 PE string entries have stable reference keys. Caption/command matches and non-unique control-ID candidates are retained as associations, not proof of the triggering behavior. **1136 dynamic references have no such resource association** and remain explicit investigation work. This prevents the static dialog count from hiding dynamically assembled functionality.

## Dialog families

| Original dialog(s) | XDM feature | State | Behavior and remaining acceptance |
|---|---|---|---|
| D100 | `app.about` — About and license information | pending | Show XDM version, GPL license, dependencies and project links. |
| D102 | `gui.shell` — Main shell and category/queue navigation | partial | Task list and filters exist; full table/tree, menus, multi-selection and keyboard navigation remain. |
| D103, D363, D367 | `task.progress` — Detailed progress and segment view | partial | Live HTTP totals/speed/ETA exist; connection visualization, resume capability and configurable detail tabs remain. |
| D105, D236 | `app.tips` — Tips and integration guidance | pending | Original help and tips must be replaced with independently written guidance and a dismiss/startup preference. |
| D130, D148 | `task.add` — Add URL and download confirmation | partial | URL/path and queue import exist; category/description/auth fields, destination picker, history and download-later confirmation remain. |
| D135 | `task.properties` — Download properties | partial | Details exist; editable destination/description, parent page/referrer, last-attempt metadata and address refresh remain. |
| D138 | `app.commercial` — Commercial registration and purchase | intentional-substitute | XDM is GPL and has no paid activation. Map the information role to open-source license/project information; do not reproduce IDM license checks. |
| D139, D231 | `settings.general` — General settings and integration | partial | Browser opt-in exists; startup, clipboard monitoring, browser detection and integration preferences remain. D231 is an additional resource variant. |
| D140, D369 | `settings.proxy` — Proxy, SOCKS and advanced proxy settings | pending | Implement explicit none/system/PAC/manual choices, per-protocol credentials/bypass, SOCKS DNS and safe browser proxy context; native platform tests required. |
| D141 | `settings.connections` — Connection defaults | partial | Task connections and job limits exist; persistent default profiles and related settings UI remain. |
| D143 | `settings.paths` — Save paths and file dates | partial | Built-in routing exists; remembered/category/temp paths, creation-date policy and destination rules remain. |
| D144, D182, D190, D290 | `app.update` — Updates and optional components | pending | Provide independent release checks and optional component handling with version/integrity validation; no IDM update endpoints. |
| D145, D146 | `auth.store` — Site/path credential profiles | pending | Origin/path-scoped credentials, protected storage and explicit edit/remove workflows are required. |
| D147, D300 | `task.confirmation` — Generic task confirmations | unresolved | Resource captions establish choices but not every caller. Trace triggering conditions before specifying yes-all/no-all or continue behavior. |
| D150 | `power.shutdown` — Queue completion shutdown | pending | Explicit per-queue success/failure policy, cancelable countdown and OS adapter; fixture tests must never shut down the host. |
| D184, D185, D186 | `platform.dialup` — Dial-up/VPN and redial | platform-pending | Windows connection selection, credential handling and reconnect policy require native Windows validation; Linux equivalents need separate capability design. |
| D187 | `gui.drop` — Floating URL drop target | pending | Original visual design, drag/drop URL validation, show/hide and queue submission. |
| D191 | `lists.export` — Download list export | partial | Portable JSON/text exports exist; selected/queue scope and original-format compatibility remain. JSON is an intentional new format. |
| D193, D294, D341 | `browser.types` — Capture file types | partial | Hard-coded media detection exists; configurable extension types, defaults and site exclusions remain. |
| D194, D309 | `settings.quota` — Usage quota and warning | pending | Persist byte budgets/window rules and warning/resume behavior, separately from speed caps. |
| D195 | `settings.sounds` — Event sounds | pending | Per-event sound selection, preview playback and enable/disable preferences. |
| D196 | `task.archive` — ZIP member preview | pending | Bounded range-aware archive inspection and a member list; unsupported/large/malformed archives need explicit handling. |
| D204 | `settings.categories` — Custom categories and rules | partial | Built-in extension/MIME categories exist; create/edit/delete, site patterns, custom folders and user overrides remain. |
| D208 | `gui.language` — Language selection | pending | Independent translations, persistent locale and runtime switching; no copied IDM translation catalog. |
| D209 | `gui.columns` — Task columns | pending | Show/hide/reorder columns and persist widths/order; a row label is not column parity. |
| D226 | `settings.host_connections` — Per-server connection caps | pending | Host pattern limits shared across simultaneous tasks, distinct from per-task segment count. |
| D228 | `browser.all_links` — Capture and select all page links | pending | Collect eligible links with selection, category/destination rules and durable batch handoff. |
| D229 | `browser.shortcuts` — Force/bypass modifier keys | pending | Per-browser tested key combinations and transient capture override; no accidental default browser cancellation. |
| D230 | `network.check` — Connection diagnostics | pending | Bounded connectivity/integration checks and useful status without leaking authentication. |
| D232, D336 | `app.report` — Problem reports | pending | Create a redacted, reviewable local report; sending must be an explicit user action. |
| D239, D241 | `grabber.wizard` — Site grabber wizard | pending | Independent project wizard with crawl plan, destination, results and start/stop steps. |
| D240 | `grabber.files` — Grabber results and file filters | pending | Selection tree/table, duplicate rules, same-site restriction and filters. |
| D242, D504, D30734 | `legacy.empty` — Empty resource templates | reference-only | Retain identities; empty templates do not establish a separately reachable product feature. |
| D244, D245 | `grabber.filters` — Grabber filter editor | pending | Named wildcard include/exclude masks with add/edit/delete and validation. |
| D257 | `grabber.project` — Grabber project creation | pending | Project name, seed URL, templates and durable project state. |
| D259 | `grabber.destination` — Grabber destination policy | pending | Category folder/single-folder rules, offline path mapping and conflict handling. |
| D262 | `grabber.settings` — Grabber concurrency and cache settings | pending | Separate explore/download budgets; replace legacy IE cache behavior with a supported cache policy. |
| D264 | `grabber.crawl` — Crawl scope and depth | pending | Depth, base-site/outside-site limits, navigation filters, robots/rate policy and cycle bounds. |
| D268 | `grabber.templates` — Grabber templates | pending | Named validated templates and independent settings schema. |
| D269 | `grabber.save` — Save grabber projects | pending | Atomic project save, name conflicts and optional prompt preference. |
| D270 | `grabber.load` — Load grabber projects | pending | Project listing, schema migration and damaged-project recovery. |
| D271 | `grabber.statistics` — Grabber statistics | pending | Processed pages/files, discovered links, filtered/skipped/error counts and progress events. |
| D279 | `auth.challenge` — HTTP authentication challenge | pending | Realm/origin prompt, protected credentials, cancellation and retry; do not blindly forward credentials across origins. |
| D280 | `batch.generated` — Generated URL batches | pending | Numeric/alphabetic sequence ranges, padding, preview and bounded expansion. URL-list import is not generated-batch parity. |
| D281 | `grabber.cleanup` — Grabber temporary-file cleanup | pending | Cancelable cleanup progress and interrupted-project recovery without deleting user outputs. |
| D284 | `grabber.schedule` — Grabber project schedule | pending | One-shot/recurring crawl/download phases, time limits and completion rules. |
| D286, D364 | `task.completion` — Completion dialog and actions | pending | Open/open-with/folder, dialog preference, destination action, app exit and explicit power policy. |
| D291 | `settings.downloads` — Download defaults | partial | Explicit queue/paused import exists; start/complete prompts, eager transfer, duplicate defaults, User-Agent and completion options remain. |
| D292 | `power.restart` — Queue completion restart | pending | Opt-in completion policy and cancelable restart countdown; native Windows/Linux tests required. |
| D296, D297, D334, D381, D489 | `browser.integration` — Chromium-family integration | partial | Linux Chrome for Testing handoff/fallback passes; installed Chrome/Edge/Brave profiles and native Windows registration/GUI validation remain. |
| D298 | `task.duplicate` — Duplicate download choices | partial | Conflicts reject safely and explicit fresh-destination redownload exists; numbered duplicate/overwrite/resume-complete dialog policies remain. |
| D302, D303, D304 | `browser.exceptions` — Capture address/site exceptions | pending | Persistent site/address masks, cancellation prompts and once-only fallback behavior. |
| D305 | `browser.fallback` — Handoff failure and browser fallback | verified-subset | Real Linux acceptance/cancel and daemon-unavailable browser completion pass; single-use GET/POST, HTML miscapture, other platforms and disconnect/late-ack races remain. |
| D306, D332 | `queue.schedule` — Queue schedule | partial | Persistent global one-shot start/stop exists; per-queue times, recurring weekday/daily, time zones and missed-run policy remain. |
| D307 | `queue.files` — Queue membership, order and concurrency | partial | Atomic stopped-task assignment, priority/FIFO and per-queue caps exist; manual ordering and sync retention remain. |
| D308, D310 | `queue.manage` — Named queue management | partial | Create/rename/empty-delete/start/stop/limits and GUI filtering pass local tests; enabling a queue does not open the global gate, so independent original queue start semantics remain a parity gap. Stopping a queue interrupts only its workers. |
| D335 | `browser.firefox` — Firefox browser adapter | pending | Independent event adapter, host registration and real browser tests; Chromium hooks cannot be assumed equivalent. |
| D337, D365 | `speed.task` — Speed limiter | partial | Persisted live HTTP task caps exist; application startup defaults, media caps and limiter-tab customization remain. |
| D338, D339 | `task.remove` — Task and optional file deletion | partial | Stopped-task removal preserves outputs/checkpoints and replay receipts; optional completed-file deletion and multi-task delete remain. |
| D340, D370 | `media.overlay` — Floating player panel | pending | Full/mini player overlay, position/visibility, site exclusions and protected-content notification; toolbar popup is not overlay parity. |
| D342 | `platform.privilege` — Privilege/integration compatibility | platform-pending | Per-user IPC exists; Windows integrity levels, broker behavior and browser integration checks need native validation. |
| D344 | `task.wait` — Stop waiting and reconnect UI | partial | Bounded retries exist; user-facing stop-waiting state and interrupted-read responsiveness need further tests. |
| D345 | `queue.assignment` — Deferred tasks and queue choice | partial | Import paused/enqueue and queue assignment exist; per-download later/start queue prompt and remembered choices remain. |
| D348, D349 | `browser.selection_panel` — Selected-link download panel | pending | Selected URL detection, full/mini panel and always/never/site-specific preferences. |
| D352 | `app.exit` — Exit and state saving | partial | SQLite persists commands and GUI can close independently; explicit whole-app stop/exit policy and graceful daemon shutdown remain. |
| D487, D505, D506 | `gui.search` — Find file | partial | Case-insensitive URL/path/status/category search exists; description/page/referrer scopes, case/whole-string and find-next remain. |
| D488 | `media.minimum` — Minimum detected media size | pending | Configurable minimum size and unavailable-size behavior; do not discard manifests by segment size. |
| D494 | `security.scanner` — Antivirus hook | pending | Explicit scanner executable/arguments, completion timing and result/error handling. |
| D30721 | `legacy.generic` — Generic native New template | unresolved | Retain this framework-looking resource; caller and reachable product behavior have not been established. |

## Every static menu command

| Source | Original command path | XDM feature | State |
|---|---|---|---|
| M129/32771 | Tasks → Add new download | `task.add` | partial |
| M129/34024 | Tasks → Add batch download | `batch.generated` | pending |
| M129/34036 | Tasks → Add batch download from clipboard | `browser.all_links` | pending |
| M129/33010 | Tasks → Run site grabber | `grabber.wizard` | pending |
| M129/32809 | Tasks → Show drop target | `gui.drop` | pending |
| M129/32821 | Tasks → Export → To IDM export file | `lists.export` | partial |
| M129/32823 | Tasks → Export → To text file | `lists.export` | partial |
| M129/32816 | Tasks → Import → From IDM export file | `lists.import` | verified-subset |
| M129/32818 | Tasks → Import → From text file | `lists.import` | verified-subset |
| M129/32772 | Tasks → Exit | `app.exit` | partial |
| M129/32774 | File → Stop Download | `task.resume` | partial |
| M129/32775 | File → Remove | `task.remove` | partial |
| M129/32776 | File → Download Now | `task.resume` | partial |
| M129/33009 | File → Redownload | `task.redownload` | partial |
| M129/32778 | Downloads → Pause All | `queue.master` | partial |
| M129/32779 | Downloads → Stop All | `queue.master` | partial |
| M129/32794 | Downloads → Delete All Completed | `task.remove_completed` | pending |
| M129/34040 | Downloads → Find (Ctrl-F) | `gui.search` | partial |
| M129/34041 | Downloads → Find Next (F3) | `gui.find_next` | pending |
| M129/32801 | Downloads → Scheduler | `queue.schedule` | partial |
| M129/34026 | Downloads → Start queue → Start | `queue.manage` | partial |
| M129/34028 | Downloads → Stop queue → Stop | `queue.manage` | partial |
| M129/34033 | Downloads → Speed Limiter → Turn on | `speed.task` | partial |
| M129/34034 | Downloads → Speed Limiter → Turn off | `speed.task` | partial |
| M129/34035 | Downloads → Speed Limiter → Settings... | `speed.task` | partial |
| M129/32781 | Downloads → Options | `settings.options` | partial |
| M129/32825 | View → Hide categories | `gui.category_visibility` | pending |
| M129/32828 | View → Arrange files → By Order Of Addition | `gui.sort` | pending |
| M129/32830 | View → Arrange files → By File Name | `gui.sort` | pending |
| M129/32832 | View → Arrange files → By Size | `gui.sort` | pending |
| M129/32834 | View → Arrange files → By Status | `gui.sort` | pending |
| M129/32836 | View → Arrange files → By Time Left | `gui.sort` | pending |
| M129/32838 | View → Arrange files → By Transfer Rate | `gui.sort` | pending |
| M129/33006 | View → Arrange files → By Last Try Date | `gui.sort` | pending |
| M129/32840 | View → Arrange files → By Description | `gui.sort` | pending |
| M129/33004 | View → Arrange files → By Save Path | `gui.sort` | pending |
| M129/33005 | View → Arrange files → By Referer | `gui.sort` | pending |
| M129/34043 | View → Arrange files → By Parent Web Page | `gui.sort` | pending |
| M129/33001 | View → Toolbar → Customize... | `gui.toolbar` | pending |
| M129/34011 | View → Toolbar → Large Buttons | `gui.toolbar` | pending |
| M129/34010 | View → Toolbar → Small Buttons | `gui.toolbar` | pending |
| M129/34012 | View → Toolbar → Look for new toolbars | `gui.toolbar` | pending |
| M129/33510 | View → Toolbar → Neon | `gui.toolbar` | pending |
| M129/34037 | View → IDM tray icon → 3D style | `gui.tray` | pending |
| M129/34038 | View → IDM tray icon → Classic style | `gui.tray` | pending |
| M129/34039 | View → IDM tray icon → Don't show | `gui.tray` | pending |
| M129/33002 | View → Customize URL List... | `gui.columns` | pending |
| M129/34042 | View → Dark Mode support | `gui.appearance` | pending |
| M129/34045 | View → Font → Choose Font... | `gui.appearance` | pending |
| M129/34046 | View → Font → Reset to default Font | `gui.appearance` | pending |
| M129/32841 | View → Language → English | `gui.language` | pending |
| M129/57670 | Help → Help contents | `app.help` | pending |
| M129/32811 | Help → Tutorials | `app.help` | pending |
| M129/34029 | Help → Scheduler and queues | `app.help` | pending |
| M129/34022 | Help → Grabber Help | `app.help` | pending |
| M129/106 | Help → Tip of the Day... | `app.tips` | pending |
| M129/32813 | Help → IDM Home Page | `app.help` | pending |
| M129/32784 | Help → Contact IDM Support | `app.help` | pending |
| M129/32787 | Help → Check for updates... | `app.update` | pending |
| M129/32786 | Help → About IDM | `app.about` | pending |
| M129/32788 | Help → Tell a friend | `app.help` | pending |
| M129/32785 | Registration → Order online | `app.commercial` | intentional-substitute |
| M129/32783 | Registration → Registration | `app.commercial` | intentional-substitute |
| M256/1458 | Project → New | `grabber.project` | pending |
| M256/1438 | Project → Load... | `grabber.load` | pending |
| M256/1442 | Project → Save | `grabber.save` | pending |
| M256/34020 | Project → Save current settings as a template | `grabber.templates` | pending |
| M256/34013 | Project → Recent Projects → Recent Projects list | `grabber.recent` | pending |
| M256/34014 | Project → Recent Projects → 2 | `grabber.recent` | pending |
| M256/34015 | Project → Recent Projects → 3 | `grabber.recent` | pending |
| M256/34016 | Project → Recent Projects → 4 | `grabber.recent` | pending |
| M256/1079 | Project → Close | `grabber.cleanup` | pending |
| M256/1457 | Options → Grabber Settings | `grabber.settings` | pending |
| M256/34021 | Options → Schedule the project | `grabber.schedule` | pending |
| M256/57670 | Help | `app.help` | pending |

## Non-UI behavior

| Feature | State | Behavior and remaining acceptance |
|---|---|---|
| `http.probe` — HEAD/GET and actual Range probing | verified-subset | CLI fixtures verify real byte ranges and safe single-stream fallback; unusual compressed/redirected/unknown-size cases still need coverage. |
| `http.partition` — Parallel range boundaries and response validation | verified-subset | Unit split/range parsing plus hash fixtures pass. Server validators, identity encoding and declared byte bounds are checked. |
| `http.dynamic` — Dynamic split and stalled-worker redistribution | pending | Lease-safe reassignment of the largest remaining segment, minimum split size and stall cancellation. |
| `http.recovery` — Checkpoint and crash recovery | partial | Segmented HTTP pause/resume and changed-validator refusal pass. The current single-stream path (including connections=1 on range-capable servers) restarts at zero and has no resume checkpoint; disk-full, forced crash, >4 GiB and filesystem edge cases remain. |
| `http.retry` — Retry and error classification | partial | Transient retries and 403/malformed ranges pass; Retry-After/jitter, expired URL workflows and user-driven reconnect remain. |
| `http.storage` — Sparse allocation, offset I/O and publication | partial | Pre-sized shared partial files and direct offset writes exist; native Windows sparse semantics and no-clobber completion races need more validation. |
| `http.auth` — Headers, cookies and origin-scoped redirects | partial | Direct browser/CLI headers exist; cross-origin authenticated redirects stop. Protected credential storage and authenticated media replay remain. |
| `http.protocols` — Protocol and proxy coverage | partial | HTTP(S) is implemented; FTP/PAC/SOCKS/profile fidelity from shipped settings requires independent work and native tests. |
| `http.global_cap` — Aggregate bandwidth cap | pending | Requested XDM feature; distinguish aggregate global/host caps from IDM's observed per-file limiter. |
| `ipc.native` — Native Messaging framing and daemon bridge | verified-subset | Partial framing/size tests, per-user local IPC and real Linux browser host handoff pass; native Windows ACL/integration validation remains. |
| `browser.intercept` — Browser handoff transaction | partial | Durable registration before browser cancel and bounded fallback exist; late acceptance/timeout ownership reconciliation remains. |
| `browser.context` — Link and media context menus | partial | Direct link/media actions exist; selected-link batches, modifiers and browser download-list context surfaces remain. |
| `media.discovery` — DOM and network media discovery | partial | HTTP media elements, source children, MIME and URL candidates exist; player overlay, navigation lifecycle and all site variants remain. |
| `media.body_capture` — Page-context XHR/fetch and manifest-body capture | pending | Original extension producers were mapped; fresh bounded page-context body/chunk/hash protocol is not implemented. |
| `media.catalog` — Available representation and audio catalog | partial | Public clear FFprobe tracks and popup selection work; compatibility/language/subtitle/audio-group details need broader fixtures. |
| `media.hls` — Clear HLS selection and assembly | partial | High/low video plus separate audio fixtures decode; authentication, live capture, discontinuities and expiring URLs remain. |
| `media.dash` — Clear DASH selection and assembly | partial | Selected clear video/audio MP4 fixtures decode; additional profiles, subtitles and authenticated segments remain. |
| `media.containers` — Mux and output format choices | partial | FFmpeg stream-copy MP4 exists when codecs fit; MKV/WebM choices and explicit conversion remain. |
| `media.audio` — Audio-only and MP3 extraction | pending | Explicit audio-only track workflow and optional MP3 transcode; MP4/MP3 are not interchangeable without conversion. |
| `media.protection` — Protected-content handling | verified-subset | Encrypted HLS and DASH ContentProtection refusal pass. DRM decryption is outside the requested clean-room scope; detection/notification fidelity remains. |
| `queue.sync` — Synchronization queues | pending | Retained source tasks, validator recheck intervals, changed-object replacement and sync scheduling. |
| `scheduler.recurring` — Daily/weekday and periodic schedules | pending | Time-zone/DST, clock changes, missed-run policy, duration/quota cutoffs and durable execution IDs. |
| `scheduler.actions` — Completion program, sleep, exit and power actions | pending | Explicit per-queue policy, cancelable countdown, process launching and platform capability tests. |
| `platform.install` — Browser host registration and packaging | partial | Linux/Windows registration scripts exist; native Windows browser profiles, installer, signing/update and file associations remain. |
| `platform.legacy` — Windows monitor drivers and legacy IE helpers | intentional-substitute | No proprietary driver clone. Map download handoff to supported browser/native-host APIs; legacy-only compatibility is explicitly unverified. |
| `performance.native` — CPU/RAM and native toolkit benchmark | pending | Bounded buffers and separate GUI/daemon exist; release PSS/private-byte, wakeup, 10k-row and Qt-versus-Slint measurements remain. |

## Evidence and maintenance

The [completed bounded audit](idm_audit_completion.md), [dialog catalog](analysis/idm_dialog_catalog.md), [browser/media audit](idm_browser_media_analysis.md), and [native-core audit](idm_native_core_analysis.md) support the mapping. [Implementation status](implementation_status.md) and the tests under `tests/integration` support the current XDM subset.

All language entries have also received a [semantic review](idm_behavior_review.md), with detailed dynamic state-transition contracts. Semantic association does not establish the original runtime caller, so the unresolved association count above is retained.

Regenerate with `python3 docs/analysis/build_feature_coverage.py`; validate coverage and checked-in outputs with `python3 docs/analysis/build_feature_coverage.py --check`. The generator rejects unmapped dialog/menu command identities and checks that every source control has exactly one stable mapping key. New implementation evidence must update the family/path state and acceptance text, not just the coverage count.
