# XDM Parity Ledger

## Summary Counts per Document

| Document | Implemented | Partial | Different | Missing | Unknown | Total |
|---|---:|---:|---:|---:|---:|---:|
| idm_core_queue_contracts.md | 11 | 42 | 5 | 11 | 0 | 69 |
| idm_browser_behavior_contracts.md | 37 | 14 | 8 | 3 | 0 | 62 |
| idm_behavior_review.md | 0 | 27 | 0 | 12 | 0 | 39 |
| **Total** | **48** | **83** | **13** | **26** | **0** | **170** |

## Contract Details

| ID | Title | Status | Current XDM Behavior | Tests |
|---|---|---|---|---|
| N01 | GET/HEAD discovery | implemented | Implemented (2026-10-13): no HEAD. One GET with `Range: bytes=OFFSET-` both veri… | tests/integration/http_engine.py |
| N02 | HTTP methods and request bodies | partial | Partial: `Options.method/body` and CLI `--method/--data-file` replay the method … | tests/integration/request_context.py |
| N03 | One-shot and expiring URLs | partial | Partial: request adoption means a ranged or full one-shot link is fetched by the… | tests/integration/http_engine.py |
| N04 | Range capability and boundaries | implemented | Implemented static path: strict Content-Range parser, bounded segments, identity… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N05 | Ignored or malformed partial response | partial | Partial: Malformed probe falls back to full transfer; a server that stops honori… | tests/integration/http_engine.py, tests/integration/restart_required.py, tests/integration/http_fixture.py, crates/xdm-http/src/lib.rs |
| N06 | Representation compression | different | Restricted: sends Accept-Encoding identity and rejects nonidentity final/range r… | — |
| N07 | Known and unknown response length | partial | Partial: unknown size streams the adopted full response until EOF with total/ETA… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N08 | Redirect chains and credentials | partial | Partial: maximum ten hops; Referer is not rewritten on redirect. Cookie/Authoriz… | tests/integration/http_engine.py |
| N09 | Cookie/User-Agent/Referer/header forwarding | partial | Partial: HTTP header list is persisted and shared among workers; engine owns Ran… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N10 | Origin/path login credentials | implemented | Implemented on Linux (2026-10-13): site/path credential store with pattern match… | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs, crates/xdm-daemon/src/main.rs |
| N11 | Authentication retries and expiry | partial | Partial (2026-10-13): 401 challenge produces `auth_required` status with realm, … | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs |
| N12 | Remote-object validators | partial | Partial: checks size/final URL/header-context hash and available strong ETag/dat… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N13 | Ignore modification-time preference | implemented | Implemented: `ResumePolicy { ignore_modification_date: true }` (set per task thr… | tests/integration/naming_duplicates.py, crates/xdm-http/src/lib.rs, crates/xdm-daemon/src/main.rs, crates/xdm-ipc/src/lib.rs |
| N14 | User-visible filename/type changes | partial | Partial: Resolves server-chosen filenames from `Content-Disposition` (RFC 6266 p… | tests/integration/naming_duplicates.py, crates/xdm-http/src/lib.rs, crates/xdm-daemon/src/main.rs, crates/xdm-ipc/src/lib.rs |
| N15 | Permission/permanent HTTP errors | partial | Partial:403/404 nonretryable; daemon records failed. Error is text rather than t… | tests/integration/http_engine.py, tests/integration/cli_http.py, tests/integration/contracts_core.py |
| N16 | Transient server/network retries | partial | Partial: probe/full requests retry up to four times on network/408/429/5xx honor… | tests/integration/http_engine.py, tests/integration/cli_http.py, tests/integration/contracts_core.py |
| N17 | Timeouts, DNS and diagnostics | partial | Partial: blocking client has30-second request timeout; network errors propagate.… | tests/integration/contracts_core.py |
| N18 | TLS/certificates and Microsoft TLS1.3 preference | different | Different backend: reqwest rustls-tls. No Microsoft TLS toggle, custom trust/cli… | — |
| N19 | FTP authentication/control replies | partial | Partial (2026-10-13): independent FTP client (`xdm-ftp` crate) implements contro… | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs |
| N20 | FTP passive and active data modes | partial | Partial (2026-10-13): passive mode implemented via EPSV (preferred) with PASV fa… | crates/xdm-ftp/tests/ftp.rs |
| N21 | FTP REST/SIZE and resume | partial | Partial (2026-10-13): SIZE, MDTM, REST implemented. Resume guarded by SIZE+MDTM … | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs |
| N22 | FTP optional connections/pipelining | missing | Missing: single serialized control session per task; no concurrent data connecti… | — |
| N23 | Manual per-protocol proxy | partial | Partial (2026-10-13): engine `Proxy::Manual` takes separate proxy URLs for http … | tests/integration/http_engine.py, tests/integration/request_context.py |
| N24 | SOCKS4/SOCKS5 and remote DNS | partial | Partial (2026-10-13): reqwest `socks` feature enabled; `socks4://`, `socks5://` … | tests/integration/http_engine.py |
| N25 | Proxy exclusions and host patterns | partial | Partial (2026-10-13): manual proxies take a bypass list of host names (subdomain… | tests/integration/http_engine.py |
| N26 | System proxy/PAC/WPAD | partial | Partial: `Proxy::System` uses proxy environment variables only (fixture verifies… | — |
| N27 | Browser proxy/VPN context fallback | missing | Missing: captured task has no resolved browser proxy route. | — |
| N28 | Windows dial-up/VPN and redial | missing | Missing. | — |
| N29 | Static segment allocation | implemented | Implemented independent static1–16 partitioning; one worker thread per segment,6… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N30 | Dynamic splitting and worker reuse | partial | Partial (2026-10-13): workers form a pool of the requested size that claims idle… | tests/integration/http_engine.py |
| N31 | Stall recovery and adaptive connections | implemented | Implemented: Configurable stall timeout (default 15 s, valid 1–600 s) aborts a c… | tests/integration/http_engine.py, tests/integration/restart_required.py, tests/integration/http_fixture.py, tests/integration/contracts_core.py, crates/xdm-http/src/lib.rs, crates/xdm-daemon/src/main.rs |
| N32 | Overlap/data-corruption validation | partial | Partial safety: strict headers/body lengths/validators; no persisted interval ch… | — |
| N33 | Preallocation and offset writes | different | Independent implementation: set_len plus positional write_at; not explicit Linux… | — |
| N34 | Checkpoint schema and write ordering | partial | Partial: version2 JSON .part.meta, per-segment cursor, URLs/context hash/validat… | tests/integration/contracts_core.py |
| N35 | Pause/resume with one connection | implemented | Implemented (2026-10-13): connections=1 uses a single range worker with a `.part… | tests/integration/http_engine.py |
| N36 | Non-range pause/restart | partial | Partial: Single-stream non-range pause retains .part without metadata; next atte… | tests/integration/http_engine.py, tests/integration/restart_required.py |
| N37 | Crash recovery and stale running states | partial | Partial: daemon maps running/pausing→queued at open; segmented checkpoint valida… | tests/integration/contracts_core.py |
| N38 | Separate temporary and final directories | missing | Missing: .part and.meta sit beside destination; no distinct temp-root setting or… | — |
| N39 | Disk-full and access-denied assembly | partial | Partial (2026-10-13): disk-space preflight via `fstatvfs` and rollback of failed… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N40 | Large files and filesystem limits | implemented | Implemented (2026-10-13): u64 offsets/lengths and set_len; >4 GiB crossing verif… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N41 | File timestamps | missing | Missing: no preserve-server-date option or timestamp write. | — |
| N42 | Finalize, rename and output conflicts | partial | Partial (2026-10-13): publication uses a hard link plus removal of `.part` (chec… | tests/integration/http_engine.py, tests/integration/contracts_core.py |
| N43 | Orphan partial cleanup | missing | Missing: stopped-task remove retains files by design; no orphan inventory/cleanu… | — |
| N44 | Default/per-server connection caps | partial | Partial: per-task1–16 only; daemon job cap1–16. No shared host connection budget… | — |
| N45 | Per-file limiter and remembered exceptions | partial | Partial: shared token bucket across a task workers, persisted task cap, adjustab… | tests/integration/task_rate.py |
| N46 | Aggregate/per-host bandwidth limits | missing | Missing: no aggregate/per-host bucket. | — |
| N47 | Usage quota and automatic resumption | missing | Missing: live counters exist but no durable quota ledger/window scheduler. | — |
| N48 | Resource efficiency contract | partial | Partial: bounded 64 KiB buffers, task-limit bounded threads; per-MiB sync checkp… | — |
| Q01 | Queue types and default queues | partial | Partial: only Main + named ordinary queues; no fixed Synchronization queue or mo… | tests/integration/daemon_ipc.py, tests/integration/sync_queues.py, crates/xdm-daemon/src/sync.rs, crates/xdm-ipc/src/lib.rs |
| Q02 | Named queues and membership | partial | Partial: create/rename/empty-remove/atomic stopped-task assign; Main fixed, max3… | tests/integration/named_queues.py |
| Q03 | Independent start/stop and global controls | partial | Partial: `queue start-one ID` sets a per-queue independent-start flag so the que… | tests/integration/scheduler.py, tests/integration/queue_policies.py |
| Q04 | Per-queue simultaneous files | implemented | Implemented bounded per-queue1–16 plus global1–16; Main initial queue limit16/gl… | tests/integration/named_queues.py |
| Q05 | Manual queue order and priority | partial | Partial: `position` column (migrated; new tasks append via trigger), `daemon mov… | tests/integration/queue_policies.py |
| Q06 | Completion leaves ordinary queue | partial | Partial: when a task completes, `queue_id` becomes 0 (no live queue) and the ori… | tests/integration/queue_policies.py |
| Q07 | Queue retry count and rotation | partial | Partial: per-queue `queue retry ID COUNT | tests/integration/queue_policies.py |
| Q08 | One-time start/stop | partial | Partial: global one-shot as before, plus per-queue one-time schedule (`queue sch… | tests/integration/scheduler.py, tests/integration/queue_policies.py |
| Q09 | Daily/weekday recurrence | partial | Partial: per-queue daily schedule (`queue schedule ID daily HH:MM HH:MM | tests/integration/queue_policies.py |
| Q10 | Synchronization repeat/check changed objects | missing | Missing: no conditional refresh/replacement or recurring validator watch. | tests/integration/sync_queues.py, crates/xdm-daemon/src/main.rs, crates/xdm-daemon/src/sync.rs, crates/xdm-ipc/src/lib.rs |
| Q11 | Start queue on application startup | different | Different: running flags restored; crashed active tasks requeued and eligible ta… | — |
| Q12 | Missed runs and daemon availability | partial | Partial: per-queue missed-run policy is implemented (catch up once if the window… | — |
| Q13 | Sleep wake timers | missing | Missing: no wake registration or capability warning. | — |
| Q14 | Completion success/failure/manual-stop rules | partial | Partial (2026-10-13): per-queue cycle state (`idle/running/completed/completed_w… | tests/integration/completion_actions.py, tests/integration/queue_policies.py |
| Q15 | Completion program/file launch | implemented | Implemented (2026-10-13): `queue completion ID program PATH [ARGS...]` stores th… | tests/integration/completion_actions.py |
| Q16 | Disconnect modem/VPN and app exit | partial | Partial: the modem/VPN disconnect action remains missing (no dial-up or VPN adap… | — |
| Q17 | Shutdown/restart/sleep/hibernate | implemented | Implemented (2026-10-13): sleep, hibernate, shutdown and restart are settable pe… | tests/integration/completion_actions.py |
| Q18 | Force process termination setting | missing | Not implemented: there is no Windows "force close applications" setting and no f… | — |
| Q19 | Schedule/order/settings durability | partial | Partial: SQLite WAL immediate command persistence and queue flags/limits; no sch… | — |
| Q20 | Pause/resume/redownload state semantics | partial | Partial: pause queued/running; resume paused/failed; redownload fresh destinatio… | — |
| Q21 | Queue exports and durable submission | different | Independent JSON/URL list import atomic/idempotent receipts; exports omit header… | — |
| B01 | B initialization (134–136) rejects non-Windows; C initial gu… | implemented | Implemented: Extension initialization is cross-platform without Windows-only pla… | tests/integration/contracts_browser.mjs |
| B02 | Chromium MV3 worker, all HTTP(S) frames at document start; h… | implemented | Implemented: Content scripts run at document_start with all_frames enabled, decl… | tests/integration/contracts_browser.mjs |
| B03 | B `yb/vc` stores client/version/update/silent/consent state;… | implemented | Implemented: Persists settings (interception toggle, site exclusions, private mo… | tests/integration/contracts_browser.mjs |
| B04 | B `Ba/Na/oa`: cycle two WebSocket endpoints then native host… | different | Different: Uses per-request native messaging and resumes browser downloads on da… | tests/integration/contracts_browser.mjs |
| B05 | B `Ma`: 15 s keepalive for older negotiated versions; listen… | different | Different: Employs stateless per-request messaging avoiding persistent keepalive… | tests/integration/contracts_browser.mjs |
| B06 | B `R/Gb/pb`: global/per-tab integration enablement, disabled… | implemented | Implemented: Provides global interception toggle in storage, per-tab toggle via … | tests/integration/contracts_browser.mjs |
| B07 | B `Bb/Nb`: host settings enable menus and listeners; optiona… | missing | Missing: Observes HTTP(S) and uses storage toggles, but lacks browser download s… | tests/integration/contracts_browser.mjs |
| B08 | W startup/update/error/private-access/Firefox-consent panels… | different | Different: Provides a standard options page (options_ui) documenting settings an… | tests/integration/contracts_browser.mjs |
| B09 | B `Oc` checks competing packaged extension IDs and can disab… | partial | Partial: Manifest declares management permission, but background script does not… | tests/integration/contracts_browser.mjs |
| B10 | R registration redirect forwards hash command44, strips hash… | missing | Missing: Does not implement proprietary registration URL redirects (#command44) … | tests/integration/contracts_browser.mjs |
| B11 | B `Nb/cb`: before-request observes GET/POST, request body, s… | implemented | Implemented: Observes request bodies via webRequest.onBeforeRequest; file upload… | tests/integration/contracts_browser.mjs |
| B12 | B `Rb`: before-send-headers captures headers, correlates pag… | implemented | Implemented: Request context is keyed by tab ID and URL; Referer and User-Agent … | tests/integration/contracts_browser.mjs |
| B13 | B `Ub/Hc`: response status, MIME, Content-Disposition, chars… | implemented | Implemented: Captures Content-Type from response headers and preserves UTF-8 fil… | tests/integration/contracts_browser.mjs |
| B14 | B `Qb/cb/X`: maintains earlier URL/headers/body across redir… | partial | Partial: Tracks redirects via webRequest.onBeforeRedirect to propagate request c… | tests/integration/contracts_browser.mjs |
| B15 | B `Fc/Gc/Hc/Y/yc`: host-provided extension/MIME/site/path ex… | partial | Partial: Implements wildcard site exclusion rules (*.domain) and storage setting… | tests/integration/contracts_browser.mjs |
| B16 | B `qb`: onCreated correlates recent request by URL/time (obs… | implemented | Implemented: Correlates downloads using composite tab ID and URL keys with a 60s… | tests/integration/contracts_browser.mjs |
| B17 | B `rb/lc/X`: filename listener filters then submits type13, … | implemented | Implemented: Defers filename determination via asynchronous handoff, releases th… | tests/integration/contracts_browser.mjs |
| B18 | B `$a` reply13 maps action and status3 to `kc/O`; accepted C… | implemented | Implemented: Explicitly cancels the browser download upon durable daemon accepta… | tests/integration/contracts_browser.mjs |
| B19 | B `oa` removes integration listeners; P disconnect-after-sen… | implemented | Implemented: A 10-second timeout resumes browser downloads and releases the sugg… | tests/integration/contracts_browser.mjs |
| B20 | B `Sb/Tb/O`: completion/error release correlated request; he… | implemented | Implemented: Enforces a 60-second TTL (REQUEST_TTL_MS) and a bounded cache (MAX_… | tests/integration/contracts_browser.mjs |
| B21 | B `U/V/Dc`: cookies per requested URL, original URL cookies … | implemented | Implemented: Queries URL cookies matching the request context; Firefox queries c… | tests/integration/contracts_browser.mjs |
| B22 | B `Kc/Bc`: CRLF request/response headers; raw bytes, URL-enc… | implemented | Implemented: Captures URL-encoded form POST bodies up to 1 MiB with repeated fie… | tests/integration/contracts_browser.mjs |
| B23 | B `Lc`: walk frame ancestry past about:blank for referrer; p… | partial | Partial: Declares webNavigation permission and collects initiator/documentUrl ac… | tests/integration/contracts_browser.mjs |
| B24 | B `bb/Rb/Sb`, host requests20: direct/system/auto/PAC/fixed/… | partial | Partial: Declares proxy permission and provides a basic proxy diagnostic handsha… | tests/integration/contracts_browser.mjs |
| B25 | B `rc/Cb/Fb/wb`, C key/mouse31/32/33: host-defined force/byp… | partial | Partial: Content script tracks Alt, Ctrl, Shift and Meta state in `modifierState… | tests/integration/contracts_browser.mjs |
| B26 | B `Nb/Mb`, R: configured main-frame registration redirect us… | different | Different: Manifest declares declarativeNetRequest permission, but XDM intention… | tests/integration/contracts_browser.mjs |
| B27 | B `T/dc`: HTTP/HTTPS/FTP/FTPS plus proprietary registration/… | implemented | Implemented: Restricts interception strictly to valid HTTP(S) protocols; unsuppo… | tests/integration/contracts_browser.mjs |
| B28 | B `Cb/Ea/pa`: link,image,video context download; page/frame/… | implemented | Implemented: Registers context menus for link, image, media (video/audio), and p… | tests/integration/contracts_browser.mjs |
| B29 | B `Ea/Eb`, C `Zb`: media/image source can ask DOM anchor loo… | implemented | Implemented: Context menu link downloads query the DOM content script via page.g… | tests/integration/contracts_browser.mjs |
| B30 | B `Qa/Db/Zb/ec`, C `Xb/nb/N`: selected links and all links c… | implemented | Implemented: Content script `extractLinks` collects http(s) URLs from anchor and… | tests/integration/contracts_browser.mjs |
| B31 | B `ec`: multi-link type15 initialization, count and batch id… | different | Different: Implements batch.submit native messaging with UUID request IDs and it… | tests/integration/contracts_browser.mjs |
| B32 | B `Db/Zb` + C mouse26: selected text can create desktop sele… | partial | Partial: Content script `selectionHandler`, run on selectionchange and mouseup, … | tests/integration/contracts_browser.mjs |
| B33 | B `nb/Ea` menu5: browser downloads-page link search; erases … | partial | Partial: Registers a context menu entry for page downloads (xdm-downloads), but … | tests/integration/contracts_browser.mjs |
| B34 | B `fc`: no host/no links produces browser-side error notific… | implemented | Implemented: Displays a browser notification via chrome.notifications.create whe… | tests/integration/contracts_browser.mjs |
| B35 | B `Aa/Hc`: direct audio/video, MPEGTS, M4S, HLS/DASH/F4M, su… | implemented | Implemented: Classifies direct media (MP4, WebM, MP3, etc.) and streaming manife… | tests/integration/contracts_browser.mjs |
| B36 | B `X`: media detection type13 action discriminator differs p… | implemented | Implemented: Distinguishes streaming manifests (HLS/DASH) from plain file downlo… | tests/integration/contracts_browser.mjs |
| B37 | B reply13→content19; C `bc`: browser-context fetch with URL/… | implemented | Implemented: Supports in-browser manifest fetching via manifest.fetch message ha… | tests/integration/contracts_browser.mjs |
| B38 | D `ua/Ka/La/Ja`: configured XHR open/header/send/loadend wra… | implemented | Implemented: Injects page_xhr.js at document_start to intercept streaming media … | tests/integration/contracts_browser.mjs |
| B39 | D `la/Ma/P/Na/ya`: fetch wrapper and Response text/json/arra… | implemented | Implemented: Injects page_fetch.js at document_start to monitor window.fetch for… | tests/integration/contracts_browser.mjs |
| B40 | D message1229212978 config dynamically installs/restores wra… | implemented | Implemented: Reports installed hook capabilities and status via hook.status runt… | tests/integration/contracts_browser.mjs |
| B41 | D message1229212981 property/selector extraction→1229212982;… | implemented | Implemented: Content script `queryElementMetadata` returns declared element prop… | tests/integration/contracts_browser.mjs |
| B42 | C `Gc/qc/Jc/xb/Ic/lc/mc/oc/nc`: host-version-gated site rule… | partial | Partial: Content script defines two versioned site adapters, generic-html5 and e… | tests/integration/contracts_browser.mjs |
| B43 | C `Tb/H/ib`: video/audio/object/embed/source-child associati… | implemented | Implemented: Scans video, audio, and source elements on initial load and observe… | tests/integration/contracts_browser.mjs |
| B44 | C `R/ib/jb/wb/O`: stable element/site IDs, replacement-playe… | partial | Partial: Content script `playerRegistry` gives each video or audio element a sta… | tests/integration/contracts_browser.mjs |
| B45 | C `P/Lb/fa`: visibility/opacity/dimensions/clipped ancestors… | partial | Partial: Suppresses 1x1 tracking pixels using getBoundingClientRect, but lacks f… | tests/integration/contracts_browser.mjs |
| B46 | C `Sb/cc`, B `hc/ic/nc`: nested frame IDs/rectangles transla… | partial | Partial: Runs content scripts in all frames (all_frames: true), but does not tra… | tests/integration/contracts_browser.mjs |
| B47 | C mutation/resize/intersection observers, 200 ms debounce `W… | partial | Partial: Content script runs DOM inspection behind a 200 ms debounce timer for c… | tests/integration/contracts_browser.mjs |
| B48 | B `ba/Fb/oc/ga/fc`: native panel states hidden/visible/backg… | partial | Partial: Content script `playerPanel` builds a per-player overlay element with a… | tests/integration/contracts_browser.mjs |
| B49 | B window listeners and `U/sc/pc/Oa`: window registration6, c… | missing | Missing: Does not register browser window listeners for geometry, fullscreen, DP… | tests/integration/contracts_browser.mjs |
| B50 | B `ab/xb/Ob/xa`: navigation and history changes refresh site… | implemented | Implemented: Clears tab media catalogs and resets badge counts on navigation via… | tests/integration/contracts_browser.mjs |
| B51 | C `C/$b/ob` + B `ec`: title from top page/og:title, source c… | implemented | Implemented: Derives human-readable media titles from player element titles, doc… | tests/integration/contracts_browser.mjs |
| B52 | L21130 instructs selecting desired quality in player before … | implemented | Implemented: Manifests are inspected via media.inspect native messaging, enumera… | tests/integration/contracts_browser.mjs |
| B53 | L21005/21006 audio-only/video-only; mux exports establish se… | implemented | Implemented: Supports audio-only selection via media.submit with videoIndex: -1 … | tests/integration/contracts_browser.mjs |
| B54 | L21192 incompatible audio/video can leave audio separately; … | implemented | Implemented: Forwards container format choices (e.g. MKV) via media.submit paylo… | tests/integration/contracts_browser.mjs |
| B55 | L21000/21001 live stop/save and no same-position resume; sub… | implemented | Implemented: Submits stream capture requests via media.submit with task tracking… | tests/integration/contracts_browser.mjs |
| B56 | B `$a`12/`Ac/mc`: host resource asks browser to locate assoc… | implemented | Implemented: Handles host queries via handleHostPlayerQuery to locate stored med… | tests/integration/contracts_browser.mjs |
| B57 | B `$a`24/`Dc`: host explicitly requests cookies for URL; res… | implemented | Implemented: Handles host cookie queries via handleHostCookieQuery, returning do… | tests/integration/contracts_browser.mjs |
| B58 | B `$a`20: host asks proxy comparison/context; reply121/122 a… | implemented | Implemented: Implements handleProxyHandshake to exchange platform and browser id… | tests/integration/contracts_browser.mjs |
| B59 | B `$a`22/`Ec`: optional notification permission, desktop-sup… | implemented | Implemented: Declares the notifications permission and provides desktop notifica… | tests/integration/contracts_browser.mjs |
| B60 | B `$a`100: echo family when acknowledgment requested; defaul… | different | Different: Uses versioned, allowlisted JSON operations (version: 1, type: 'downl… | tests/integration/contracts_browser.mjs |
| B61 | Native host `00402830` startup discovers Windows session/pro… | different | Different: Independent native host registered as org.xdm.browser communicating o… | tests/integration/contracts_browser.mjs |
| B62 | Standard four-byte native-endian UTF-8 JSON length, outer JS… | different | Different: Uses standard native-messaging JSON framing with a 1 MiB bound and sc… | tests/integration/contracts_browser.mjs |
| address-refresh | Expired task enters a wait-for-new-session/address flow; one… | partial | task.properties: partial; task.resume: partial; task.wait: partial | tests/integration/contracts_behavior.py |
| all-links-and-clipboard | All-link collection can hide HTML/images/duplicates and use … | partial | browser.all_links: pending; settings.general: partial | tests/integration/contracts_behavior.py |
| appearance-and-progress | System dark mode, font choice/reset and progress tab/start-v… | partial | gui.appearance: pending; task.progress: partial | tests/integration/contracts_behavior.py |
| browser-platform-health | Original health depends on elevation, BFE, drivers/helpers a… | partial | browser.integration: partial; platform.privilege: platform-pending | tests/integration/contracts_behavior.py |
| category-retroactive | Extension/site rules and last-used paths classify future fil… | partial | settings.categories: partial; settings.paths: partial | tests/integration/contracts_behavior.py |
| completion-power | Task, queue and grabber completion can request shutdown/slee… | missing | power.shutdown: pending; power.restart: pending; task.completion: pending; grabb… | tests/integration/contracts_behavior.py |
| completion-precedence | Per-file settings can be unavailable during queue processing… | partial | task.completion: pending; app.exit: partial; queue.schedule: partial; task.progr… | tests/integration/contracts_behavior.py |
| completion-program-and-scanner | Completion can open a configured file; scanner arguments acc… | missing | security.scanner: pending; grabber.schedule: pending | tests/integration/contracts_behavior.py |
| connection-profiles-and-host-caps | Default profile/connection count and per-server exceptions a… | partial | settings.connections: partial; settings.host_connections: pending | tests/integration/contracts_behavior.py |
| content-identity-recovery | Changed date/size can restart a task; date checking can be d… | partial | task.resume: partial; settings.downloads: partial | tests/integration/contracts_behavior.py |
| duplicate-policy | Remembered duplicate policies are ask, numbered duplicate, o… | partial | task.duplicate: partial; settings.downloads: partial | tests/integration/contracts_behavior.py |
| eager-confirmation | Transfer may start while confirmation is visible, so Downloa… | partial | task.add: partial; settings.downloads: partial; queue.assignment: partial | tests/integration/contracts_behavior.py |
| file-actions | Move/rename, open-with, configurable double-click and comple… | partial | task.properties: partial; task.completion: pending | tests/integration/contracts_behavior.py |
| force-bypass-capture | Force and bypass modifier sets must not conflict; force capt… | missing | browser.shortcuts: pending | tests/integration/contracts_behavior.py |
| generated-batch | A URL wildcard expands numeric/alphabetic sequences with wid… | missing | batch.generated: pending | tests/integration/contracts_behavior.py |
| grabber-auth-and-offline | Crawler supports manual login, excludes logout patterns, use… | missing | grabber.crawl: pending; grabber.destination: pending; grabber.project: pending | tests/integration/contracts_behavior.py |
| grabber-project-recovery | Corrupt/incompatible projects/templates, partial loading and… | missing | grabber.load: pending; grabber.save: pending; grabber.schedule: pending | tests/integration/contracts_behavior.py |
| limiter-precedence | Startup limiter default and remembered per-file exceptions/p… | partial | speed.task: partial | tests/integration/contracts_behavior.py |
| live-stop-save | Stopping live recording can finalize captured content, with … | partial | media.overlay: pending; task.resume: partial | tests/integration/contracts_behavior.py |
| minimum-media-size | Separate audio/video panel thresholds only apply when total … | missing | media.minimum: pending | tests/integration/contracts_behavior.py |
| miscapture-exceptions | Two cancellations or similar mistaken captures can prompt fo… | missing | browser.exceptions: pending | tests/integration/contracts_behavior.py |
| player-panel-policy | Full/mini media panel, site exclusions and protected-content… | partial | media.overlay: pending; browser.types: partial; browser.exceptions: pending | tests/integration/contracts_behavior.py |
| playlist-link-expansion | Downloaded M3U can offer its MP3 links as one download or a … | missing | media.overlay: pending; browser.all_links: pending | tests/integration/contracts_behavior.py |
| proxy-selection | Manual, auto-detected, system and none modes; PAC, protocol-… | missing | settings.proxy: pending | tests/integration/contracts_behavior.py |
| quality-and-track-catalog | Other quality may appear only after player selection; separa… | missing | media.overlay: pending | tests/integration/contracts_behavior.py |
| queue-independent-start | Named queues have individual start/stop commands; toolbar st… | partial | queue.manage: partial; queue.master: partial | tests/integration/contracts_behavior.py |
| queue-order-and-retry | Manual queue ordering and a scheduler retry count per failed… | partial | queue.files: partial; queue.schedule: partial | tests/integration/contracts_behavior.py |
| quota-window | A megabyte-per-hour-window budget can warn, stop downloads a… | missing | settings.quota: pending | tests/integration/contracts_behavior.py |
| recurrence-and-wake | Daily/weekday and periodic jobs require the application runn… | partial | queue.schedule: partial; grabber.schedule: pending | tests/integration/contracts_behavior.py |
| remove-versus-delete | Removing rows is separate from deleting completed files; inc… | partial | task.remove: partial; task.remove_completed: pending | tests/integration/contracts_behavior.py |
| search-scopes | Name/description/source/site/parent/referrer search supports… | partial | gui.search: partial; gui.find_next: pending | tests/integration/contracts_behavior.py |
| server-name-reconciliation | Redirect/response filename and MIME-derived extension can pr… | partial | task.properties: partial; task.add: partial | tests/integration/contracts_behavior.py |
| single-use-request | A second request may return HTML instead of the file; origin… | partial | browser.fallback: verified-subset; browser.shortcuts: pending; task.add: partial | tests/integration/contracts_behavior.py |
| stream-assembly-fallback | Media part processing can seek a matching frame; muxing has … | partial | media.overlay: pending; task.progress: partial | tests/integration/contracts_behavior.py |
| synchronization | Synchronization and grabber updates revisit sources, update … | partial | queue.files: partial; grabber.schedule: pending | tests/integration/contracts_behavior.py, tests/integration/sync_queues.py, crates/xdm-daemon/src/main.rs, crates/xdm-daemon/src/queues.rs, crates/xdm-daemon/src/sync.rs, crates/xdm-ipc/src/lib.rs |
| temporary-storage-cleanup | Final and temporary volumes have separate capacity/file-size… | partial | settings.paths: partial; task.resume: partial | tests/integration/contracts_behavior.py |
| tls-substitution | Experimental original TLS mode changes a global Windows sett… | partial | settings.connections: partial | tests/integration/contracts_behavior.py |
| unknown-length-completion | EOF completion includes a notice that a file of unknown size… | partial | task.progress: partial; task.completion: pending | tests/integration/contracts_behavior.py |
| updater-integrity | Update metadata/signature/download validation and site-rule … | partial | app.update: pending; browser.integration: partial | tests/integration/contracts_behavior.py |

## Work Queue

### Missing/Partial contracts with NO referenced test (highest priority)

| ID | Title | Status | Source Document |
|---|---|---|---|
| N22 | FTP optional connections/pipelining | missing | idm_core_queue_contracts.md |
| N26 | System proxy/PAC/WPAD | partial | idm_core_queue_contracts.md |
| N27 | Browser proxy/VPN context fallback | missing | idm_core_queue_contracts.md |
| N28 | Windows dial-up/VPN and redial | missing | idm_core_queue_contracts.md |
| N32 | Overlap/data-corruption validation | partial | idm_core_queue_contracts.md |
| N38 | Separate temporary and final directories | missing | idm_core_queue_contracts.md |
| N41 | File timestamps | missing | idm_core_queue_contracts.md |
| N43 | Orphan partial cleanup | missing | idm_core_queue_contracts.md |
| N44 | Default/per-server connection caps | partial | idm_core_queue_contracts.md |
| N46 | Aggregate/per-host bandwidth limits | missing | idm_core_queue_contracts.md |
| N47 | Usage quota and automatic resumption | missing | idm_core_queue_contracts.md |
| N48 | Resource efficiency contract | partial | idm_core_queue_contracts.md |
| Q12 | Missed runs and daemon availability | partial | idm_core_queue_contracts.md |
| Q13 | Sleep wake timers | missing | idm_core_queue_contracts.md |
| Q16 | Disconnect modem/VPN and app exit | partial | idm_core_queue_contracts.md |
| Q18 | Force process termination setting | missing | idm_core_queue_contracts.md |
| Q19 | Schedule/order/settings durability | partial | idm_core_queue_contracts.md |
| Q20 | Pause/resume/redownload state semantics | partial | idm_core_queue_contracts.md |

**Count: 18**

### Partial contracts WITH tests (next priority)

| ID | Title | Status | Tests | Source Document |
|---|---|---|---|---|
| N02 | HTTP methods and request bodies | partial | tests/integration/request_context.py | idm_core_queue_contracts.md |
| N03 | One-shot and expiring URLs | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N05 | Ignored or malformed partial response | partial | tests/integration/http_engine.py, tests/integration/restart_required.py, tests/integration/http_fixture.py, crates/xdm-http/src/lib.rs | idm_core_queue_contracts.md |
| N07 | Known and unknown response length | partial | tests/integration/http_engine.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N08 | Redirect chains and credentials | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N09 | Cookie/User-Agent/Referer/header forwarding | partial | tests/integration/http_engine.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N11 | Authentication retries and expiry | partial | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs | idm_core_queue_contracts.md |
| N12 | Remote-object validators | partial | tests/integration/http_engine.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N14 | User-visible filename/type changes | partial | tests/integration/naming_duplicates.py, crates/xdm-http/src/lib.rs, crates/xdm-daemon/src/main.rs, crates/xdm-ipc/src/lib.rs | idm_core_queue_contracts.md |
| N15 | Permission/permanent HTTP errors | partial | tests/integration/http_engine.py, tests/integration/cli_http.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N16 | Transient server/network retries | partial | tests/integration/http_engine.py, tests/integration/cli_http.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N17 | Timeouts, DNS and diagnostics | partial | tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N19 | FTP authentication/control replies | partial | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N20 | FTP passive and active data modes | partial | crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N21 | FTP REST/SIZE and resume | partial | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N23 | Manual per-protocol proxy | partial | tests/integration/http_engine.py, tests/integration/request_context.py | idm_core_queue_contracts.md |
| N24 | SOCKS4/SOCKS5 and remote DNS | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N25 | Proxy exclusions and host patterns | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N30 | Dynamic splitting and worker reuse | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N34 | Checkpoint schema and write ordering | partial | tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N36 | Non-range pause/restart | partial | tests/integration/http_engine.py, tests/integration/restart_required.py | idm_core_queue_contracts.md |
| N37 | Crash recovery and stale running states | partial | tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N39 | Disk-full and access-denied assembly | partial | tests/integration/http_engine.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N42 | Finalize, rename and output conflicts | partial | tests/integration/http_engine.py, tests/integration/contracts_core.py | idm_core_queue_contracts.md |
| N45 | Per-file limiter and remembered exceptions | partial | tests/integration/task_rate.py | idm_core_queue_contracts.md |
| Q01 | Queue types and default queues | partial | tests/integration/daemon_ipc.py, tests/integration/sync_queues.py, crates/xdm-daemon/src/sync.rs, crates/xdm-ipc/src/lib.rs | idm_core_queue_contracts.md |
| Q02 | Named queues and membership | partial | tests/integration/named_queues.py | idm_core_queue_contracts.md |
| Q03 | Independent start/stop and global controls | partial | tests/integration/scheduler.py, tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q05 | Manual queue order and priority | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q06 | Completion leaves ordinary queue | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q07 | Queue retry count and rotation | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q08 | One-time start/stop | partial | tests/integration/scheduler.py, tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q09 | Daily/weekday recurrence | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q14 | Completion success/failure/manual-stop rules | partial | tests/integration/completion_actions.py, tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| B09 | B `Oc` checks competing packaged extension IDs and can disab… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B14 | B `Qb/cb/X`: maintains earlier URL/headers/body across redir… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B15 | B `Fc/Gc/Hc/Y/yc`: host-provided extension/MIME/site/path ex… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B23 | B `Lc`: walk frame ancestry past about:blank for referrer; p… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B24 | B `bb/Rb/Sb`, host requests20: direct/system/auto/PAC/fixed/… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B25 | B `rc/Cb/Fb/wb`, C key/mouse31/32/33: host-defined force/byp… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B32 | B `Db/Zb` + C mouse26: selected text can create desktop sele… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B33 | B `nb/Ea` menu5: browser downloads-page link search; erases … | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B42 | C `Gc/qc/Jc/xb/Ic/lc/mc/oc/nc`: host-version-gated site rule… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B44 | C `R/ib/jb/wb/O`: stable element/site IDs, replacement-playe… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B45 | C `P/Lb/fa`: visibility/opacity/dimensions/clipped ancestors… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B46 | C `Sb/cc`, B `hc/ic/nc`: nested frame IDs/rectangles transla… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B47 | C mutation/resize/intersection observers, 200 ms debounce `W… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| B48 | B `ba/Fb/oc/ga/fc`: native panel states hidden/visible/backg… | partial | tests/integration/contracts_browser.mjs | idm_browser_behavior_contracts.md |
| address-refresh | Expired task enters a wait-for-new-session/address flow; one… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| all-links-and-clipboard | All-link collection can hide HTML/images/duplicates and use … | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| appearance-and-progress | System dark mode, font choice/reset and progress tab/start-v… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| browser-platform-health | Original health depends on elevation, BFE, drivers/helpers a… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| category-retroactive | Extension/site rules and last-used paths classify future fil… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| completion-precedence | Per-file settings can be unavailable during queue processing… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| connection-profiles-and-host-caps | Default profile/connection count and per-server exceptions a… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| content-identity-recovery | Changed date/size can restart a task; date checking can be d… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| duplicate-policy | Remembered duplicate policies are ask, numbered duplicate, o… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| eager-confirmation | Transfer may start while confirmation is visible, so Downloa… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| file-actions | Move/rename, open-with, configurable double-click and comple… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| limiter-precedence | Startup limiter default and remembered per-file exceptions/p… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| live-stop-save | Stopping live recording can finalize captured content, with … | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| player-panel-policy | Full/mini media panel, site exclusions and protected-content… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| queue-independent-start | Named queues have individual start/stop commands; toolbar st… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| queue-order-and-retry | Manual queue ordering and a scheduler retry count per failed… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| recurrence-and-wake | Daily/weekday and periodic jobs require the application runn… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| remove-versus-delete | Removing rows is separate from deleting completed files; inc… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| search-scopes | Name/description/source/site/parent/referrer search supports… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| server-name-reconciliation | Redirect/response filename and MIME-derived extension can pr… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| single-use-request | A second request may return HTML instead of the file; origin… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| stream-assembly-fallback | Media part processing can seek a matching frame; muxing has … | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| synchronization | Synchronization and grabber updates revisit sources, update … | partial | tests/integration/contracts_behavior.py, tests/integration/sync_queues.py, crates/xdm-daemon/src/main.rs, crates/xdm-daemon/src/queues.rs, crates/xdm-daemon/src/sync.rs, crates/xdm-ipc/src/lib.rs | idm_behavior_review.md |
| temporary-storage-cleanup | Final and temporary volumes have separate capacity/file-size… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| tls-substitution | Experimental original TLS mode changes a global Windows sett… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| unknown-length-completion | EOF completion includes a notice that a file of unknown size… | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |
| updater-integrity | Update metadata/signature/download validation and site-rule … | partial | tests/integration/contracts_behavior.py | idm_behavior_review.md |

**Count: 75**
