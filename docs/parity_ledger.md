# XDM Parity Ledger

## Summary Counts per Document

| Document | Implemented | Partial | Different | Missing | Unknown | Total |
|---|---:|---:|---:|---:|---:|---:|
| idm_core_queue_contracts.md | 8 | 41 | 6 | 14 | 0 | 69 |
| idm_browser_behavior_contracts.md | 2 | 25 | 5 | 30 | 0 | 62 |
| idm_behavior_review.md | 0 | 27 | 0 | 12 | 0 | 39 |
| **Total** | **10** | **93** | **11** | **56** | **0** | **170** |

## Contract Details

| ID | Title | Status | Current XDM Behavior | Tests |
|---|---|---|---|---|
| N01 | GET/HEAD discovery | implemented | Implemented (2026-10-13): no HEAD. One GET with `Range: bytes=OFFSET-` both veri… | tests/integration/http_engine.py |
| N02 | HTTP methods and request bodies | partial | Partial: `Options.method/body` and CLI `--method/--data-file` replay the method … | tests/integration/request_context.py |
| N03 | One-shot and expiring URLs | partial | Partial: request adoption means a ranged or full one-shot link is fetched by the… | tests/integration/http_engine.py |
| N04 | Range capability and boundaries | implemented | Implemented static path: strict Content-Range parser, bounded segments, identity… | tests/integration/http_engine.py |
| N05 | Ignored or malformed partial response | different | Different: malformed probe can fall back to full transfer; malformed worker resp… | tests/integration/http_engine.py |
| N06 | Representation compression | different | Restricted: sends Accept-Encoding identity and rejects nonidentity final/range r… | — |
| N07 | Known and unknown response length | partial | Partial: unknown size streams the adopted full response until EOF with total/ETA… | tests/integration/http_engine.py |
| N08 | Redirect chains and credentials | partial | Partial: maximum ten hops; Referer is not rewritten on redirect. Cookie/Authoriz… | tests/integration/http_engine.py |
| N09 | Cookie/User-Agent/Referer/header forwarding | partial | Partial: HTTP header list is persisted and shared among workers; engine owns Ran… | tests/integration/http_engine.py |
| N10 | Origin/path login credentials | implemented | Implemented on Linux (2026-10-13): site/path credential store with pattern match… | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs, crates/xdm-daemon/src/main.rs |
| N11 | Authentication retries and expiry | partial | Partial (2026-10-13): 401 challenge produces `auth_required` status with realm, … | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs |
| N12 | Remote-object validators | partial | Partial: checks size/final URL/header-context hash and available strong ETag/dat… | tests/integration/http_engine.py |
| N13 | Ignore modification-time preference | missing | Gap: no configurable ignore-date preference; engine refuses incompatible saved i… | — |
| N14 | User-visible filename/type changes | missing | Gap in core: destination chosen up front; no Content-Disposition/MIME renegotiat… | — |
| N15 | Permission/permanent HTTP errors | partial | Partial:403/404 nonretryable; daemon records failed. Error is text rather than t… | tests/integration/http_engine.py, tests/integration/cli_http.py |
| N16 | Transient server/network retries | partial | Partial: probe/full requests retry up to four times on network/408/429/5xx honor… | tests/integration/http_engine.py, tests/integration/cli_http.py |
| N17 | Timeouts, DNS and diagnostics | partial | Partial: blocking client has30-second request timeout; network errors propagate.… | — |
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
| N29 | Static segment allocation | implemented | Implemented independent static1–16 partitioning; one worker thread per segment,6… | tests/integration/http_engine.py |
| N30 | Dynamic splitting and worker reuse | partial | Partial (2026-10-13): workers form a pool of the requested size that claims idle… | tests/integration/http_engine.py |
| N31 | Stall recovery and adaptive connections | missing | Missing: fixed workers and whole-request timeout/retry. Client clones share conn… | — |
| N32 | Overlap/data-corruption validation | partial | Partial safety: strict headers/body lengths/validators; no persisted interval ch… | — |
| N33 | Preallocation and offset writes | different | Independent implementation: set_len plus positional write_at; not explicit Linux… | — |
| N34 | Checkpoint schema and write ordering | partial | Partial: version2 JSON .part.meta, per-segment cursor, URLs/context hash/validat… | — |
| N35 | Pause/resume with one connection | implemented | Implemented (2026-10-13): connections=1 uses a single range worker with a `.part… | tests/integration/http_engine.py |
| N36 | Non-range pause/restart | partial | Partial: single stream pause retains .part, no meta; next attempt restarts at ze… | tests/integration/http_engine.py |
| N37 | Crash recovery and stale running states | partial | Partial: daemon maps running/pausing→queued at open; segmented checkpoint valida… | — |
| N38 | Separate temporary and final directories | missing | Missing: .part and.meta sit beside destination; no distinct temp-root setting or… | — |
| N39 | Disk-full and access-denied assembly | partial | Partial: I/O errors propagate; no space preflight, temporary-save alternative, a… | — |
| N40 | Large files and filesystem limits | partial | Partial: u64 offsets/lengths and set_len; no >4GiB verification or filesystem li… | — |
| N41 | File timestamps | missing | Missing: no preserve-server-date option or timestamp write. | — |
| N42 | Finalize, rename and output conflicts | partial | Partial (2026-10-13): publication uses a hard link plus removal of `.part` (chec… | tests/integration/http_engine.py |
| N43 | Orphan partial cleanup | missing | Missing: stopped-task remove retains files by design; no orphan inventory/cleanu… | — |
| N44 | Default/per-server connection caps | partial | Partial: per-task1–16 only; daemon job cap1–16. No shared host connection budget… | — |
| N45 | Per-file limiter and remembered exceptions | partial | Partial: shared token bucket across a task workers, persisted task cap, adjustab… | tests/integration/task_rate.py |
| N46 | Aggregate/per-host bandwidth limits | missing | Missing: no aggregate/per-host bucket. | — |
| N47 | Usage quota and automatic resumption | missing | Missing: live counters exist but no durable quota ledger/window scheduler. | — |
| N48 | Resource efficiency contract | partial | Partial: bounded 64 KiB buffers, task-limit bounded threads; per-MiB sync checkp… | — |
| Q01 | Queue types and default queues | partial | Partial: only Main + named ordinary queues; no fixed Synchronization queue or mo… | tests/integration/daemon_ipc.py |
| Q02 | Named queues and membership | partial | Partial: create/rename/empty-remove/atomic stopped-task assign; Main fixed, max3… | tests/integration/named_queues.py |
| Q03 | Independent start/stop and global controls | partial | Partial: `queue start-one ID` sets a per-queue independent-start flag so the que… | tests/integration/queue_policies.py |
| Q04 | Per-queue simultaneous files | implemented | Implemented bounded per-queue1–16 plus global1–16; Main initial queue limit16/gl… | tests/integration/named_queues.py |
| Q05 | Manual queue order and priority | partial | Partial: `position` column (migrated; new tasks append via trigger), `daemon mov… | tests/integration/queue_policies.py |
| Q06 | Completion leaves ordinary queue | partial | Partial: when a task completes, `queue_id` becomes 0 (no live queue) and the ori… | tests/integration/queue_policies.py |
| Q07 | Queue retry count and rotation | partial | Partial: per-queue `queue retry ID COUNT | tests/integration/queue_policies.py |
| Q08 | One-time start/stop | partial | Partial: global one-shot as before, plus per-queue one-time schedule (`queue sch… | tests/integration/queue_policies.py |
| Q09 | Daily/weekday recurrence | partial | Partial: per-queue daily schedule (`queue schedule ID daily HH:MM HH:MM | tests/integration/queue_policies.py |
| Q10 | Synchronization repeat/check changed objects | missing | Missing: no conditional refresh/replacement or recurring validator watch. | — |
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
| B01 | B initialization (134–136) rejects non-Windows; C initial gu… | different | XDM is deliberately cross-platform; Linux Chromium was tested. | — |
| B02 | Chromium MV3 worker, all HTTP(S) frames at document start; h… | missing | XDM MV3 all-frame script runs document idle; no page helper. | — |
| B03 | B `yb/vc` stores client/version/update/silent/consent state;… | partial | XDM stores interception toggle only; homepage identity scheme intentionally unne… | — |
| B04 | B `Ba/Na/oa`: cycle two WebSocket endpoints then native host… | missing | XDM opens native messages per request; no proprietary WebSocket negotiation. | — |
| B05 | B `Ma`: 15 s keepalive for older negotiated versions; listen… | partial | XDM stateless submissions avoid persistent-port keepalive. | — |
| B06 | B `R/Gb/pb`: global/per-tab integration enablement, disabled… | partial | XDM action opens popup; global opt-in only, badge counts media. | — |
| B07 | B `Bb/Nb`: host settings enable menus and listeners; optiona… | missing | XDM always observes HTTP(S), interception toggle controls handoff; no shelf supp… | — |
| B08 | W startup/update/error/private-access/Firefox-consent panels… | different | XDM lacks onboarding/repair UX. Commercial prompts need an independent substitut… | — |
| B09 | B `Oc` checks competing packaged extension IDs and can disab… | missing | XDM does not disable other extensions; no duplicate-host diagnostic. | — |
| B10 | R registration redirect forwards hash command44, strips hash… | missing | XDM has no custom registration URL or capture page. | — |
| B11 | B `Nb/cb`: before-request observes GET/POST, request body, s… | partial | XB observes before-send-headers only; stores method; rejects observed non-GET. | — |
| B12 | B `Rb`: before-send-headers captures headers, correlates pag… | partial | XB retains only Referer/User-Agent, keyed by URL, maximum300. Authorization and … | — |
| B13 | B `Ub/Hc`: response status, MIME, Content-Disposition, chars… | partial | XB records MIME/media kind only; filename comes browser item. | — |
| B14 | B `Qb/cb/X`: maintains earlier URL/headers/body across redir… | missing | XDM HTTP redirects are handled by engine with origin-scoped credentials, but ext… | — |
| B15 | B `Fc/Gc/Hc/Y/yc`: host-provided extension/MIME/site/path ex… | partial | XDM basic HTTP/MIME/extension classification and global opt-in only. | — |
| B16 | B `qb`: onCreated correlates recent request by URL/time (obs… | partial | XDM pauses inside onDeterminingFilename; requires observed GET. | — |
| B17 | B `rb/lc/X`: filename listener filters then submits type13, … | partial | XDM submits independent JSON; daemon durable acceptance precedes browser cancel.… | — |
| B18 | B `$a` reply13 maps action and status3 to `kc/O`; accepted C… | partial | XDM accepts friendly `accepted` response. | — |
| B19 | B `oa` removes integration listeners; P disconnect-after-sen… | partial | XDM 10 s fallback timer exists. Late durable acceptance can still mean duplicate… | — |
| B20 | B `Sb/Tb/O`: completion/error release correlated request; he… | missing | XB recent map bounded300, but no age expiry or per-navigation cleanup. | — |
| B21 | B `U/V/Dc`: cookies per requested URL, original URL cookies … | implemented | XDM getAllURL serializes name=value; no partition/store/private context fidelity… | — |
| B22 | B `Kc/Bc`: CRLF request/response headers; raw bytes, URL-enc… | partial | XDM GET-only bridge and header subset. | — |
| B23 | B `Lc`: walk frame ancestry past about:blank for referrer; p… | partial | XDM initiator/documentURL/referrer fallback can lose full page URL and frame anc… | — |
| B24 | B `bb/Rb/Sb`, host requests20: direct/system/auto/PAC/fixed/… | missing | XDM does not transfer browser proxy context. | — |
| B25 | B `rc/Cb/Fb/wb`, C key/mouse31/32/33: host-defined force/byp… | missing | XDM lacks keyboard force/bypass capture. | — |
| B26 | B `Nb/Mb`, R: configured main-frame registration redirect us… | missing | XDM does not redirect navigation. | — |
| B27 | B `T/dc`: HTTP/HTTPS/FTP/FTPS plus proprietary registration/… | partial | XDM only HTTP(S), retains URLs according to fresh schema. | — |
| B28 | B `Cb/Ea/pa`: link,image,video context download; page/frame/… | missing | XDM context link/video/audio; image/page/frame menu absent. | — |
| B29 | B `Ea/Eb`, C `Zb`: media/image source can ask DOM anchor loo… | missing | XDM direct context submission lacks DOM label lookup. | — |
| B30 | B `Qa/Db/Zb/ec`, C `Xb/nb/N`: selected links and all links c… | missing | XDM lacks browser batch-link collection; desktop text/JSON import exists. | — |
| B31 | B `ec`: multi-link type15 initialization, count and batch id… | different | XDM desktop atomic batch imports use independent schema, not browser batch conte… | — |
| B32 | B `Db/Zb` + C mouse26: selected text can create desktop sele… | missing | XDM lacks selection panel. | — |
| B33 | B `nb/Ea` menu5: browser downloads-page link search; erases … | missing | XDM no downloads-page carry-over menu. | — |
| B34 | B `fc`: no host/no links produces browser-side error notific… | partial | XDM context action logs errors to console; popup supplies feedback. | — |
| B35 | B `Aa/Hc`: direct audio/video, MPEGTS, M4S, HLS/DASH/F4M, su… | missing | XDM narrower MIME/extensions including HLS/DASH; no F4M, ad/site rules or subtit… | — |
| B36 | B `X`: media detection type13 action discriminator differs p… | missing | XDM stores popup item, later media.inspect; no host-driven body request. | — |
| B37 | B reply13→content19; C `bc`: browser-context fetch with URL/… | missing | XDM daemon fetches manifests itself; media.inspect/media.submit payloads contain… | — |
| B38 | D `ua/Ka/La/Ja`: configured XHR open/header/send/loadend wra… | missing | XDM no page-context XHR hooks. | — |
| B39 | D `la/Ma/P/Na/ya`: fetch wrapper and Response text/json/arra… | missing | XDM no fetch/Response/stream capture. | — |
| B40 | D message1229212978 config dynamically installs/restores wra… | missing | XDM no negotiation of hook ability. | — |
| B41 | D message1229212981 property/selector extraction→1229212982;… | missing | XDM no page metadata query helper. | — |
| B42 | C `Gc/qc/Jc/xb/Ic/lc/mc/oc/nc`: host-version-gated site rule… | missing | XDM no site adapters or signed/expired URL refresh. Existing original rule confi… | — |
| B43 | C `Tb/H/ib`: video/audio/object/embed/source-child associati… | partial | XC scans video/audio currentSrc/src/source on initial/added nodes; attribute/src… | — |
| B44 | C `R/ib/jb/wb/O`: stable element/site IDs, replacement-playe… | missing | XDM set of URLs, no player identity lifecycle. | — |
| B45 | C `P/Lb/fa`: visibility/opacity/dimensions/clipped ancestors… | missing | XDM no overlay geometry. | — |
| B46 | C `Sb/cc`, B `hc/ic/nc`: nested frame IDs/rectangles transla… | partial | XDM all-frame URL collection only. | — |
| B47 | C mutation/resize/intersection observers, 200 ms debounce `W… | partial | XDM mutation added-node observer only. | — |
| B48 | B `ba/Fb/oc/ga/fc`: native panel states hidden/visible/backg… | partial | XDM popup only; no floating player panel. | — |
| B49 | B window listeners and `U/sc/pc/Oa`: window registration6, c… | missing | XDM no native browser-window tracking. | — |
| B50 | B `ab/xb/Ob/xa`: navigation and history changes refresh site… | partial | XDM deletes media on tab close only; navigation leaves previous-page items. | — |
| B51 | C `C/$b/ob` + B `ec`: title from top page/og:title, source c… | missing | XDM popup shows raw URL; no title/thumbnail/player mapping. | — |
| B52 | L21130 instructs selecting desired quality in player before … | partial | XDM public clear manifest inspection enumerates FFprobe tracks, popup sorts reso… | — |
| B53 | L21005/21006 audio-only/video-only; mux exports establish se… | partial | XDM video plus selectable audio/noaudio; popup requires video and cannot submit … | — |
| B54 | L21192 incompatible audio/video can leave audio separately; … | partial | XDM stream-copy MP4 only when compatible; limited fallback behavior. | — |
| B55 | L21000/21001 live stop/save and no same-position resume; sub… | implemented | XDM clear finite media fixture succeeds; live/subtitles/refresh pending, encrypt… | — |
| B56 | B `$a`12/`Ac/mc`: host resource asks browser to locate assoc… | missing | XDM no desktop→browser player query. | — |
| B57 | B `$a`24/`Dc`: host explicitly requests cookies for URL; res… | partial | XDM only submission-time cookie gathering. | — |
| B58 | B `$a`20: host asks proxy comparison/context; reply121/122 a… | missing | XDM no proxy handshake. | — |
| B59 | B `$a`22/`Ec`: optional notification permission, desktop-sup… | missing | XDM no native notification bridge. | — |
| B60 | B `$a`100: echo family when acknowledgment requested; defaul… | different | XDM versioned allowlisted JSON operations rather than legacy echo. | — |
| B61 | Native host `00402830` startup discovers Windows session/pro… | different | Rust host independent per-user Unix socket/named pipe, installation token; origi… | — |
| B62 | Standard four-byte native-endian UTF-8 JSON length, outer JS… | partial | XDM JSON objects, 1 MiB framing bound, schema/version validation. Native fixture… | — |
| address-refresh | Expired task enters a wait-for-new-session/address flow; one… | partial | task.properties: partial; task.resume: partial; task.wait: partial | — |
| all-links-and-clipboard | All-link collection can hide HTML/images/duplicates and use … | partial | browser.all_links: pending; settings.general: partial | — |
| appearance-and-progress | System dark mode, font choice/reset and progress tab/start-v… | partial | gui.appearance: pending; task.progress: partial | — |
| browser-platform-health | Original health depends on elevation, BFE, drivers/helpers a… | partial | browser.integration: partial; platform.privilege: platform-pending | — |
| category-retroactive | Extension/site rules and last-used paths classify future fil… | partial | settings.categories: partial; settings.paths: partial | — |
| completion-power | Task, queue and grabber completion can request shutdown/slee… | missing | power.shutdown: pending; power.restart: pending; task.completion: pending; grabb… | — |
| completion-precedence | Per-file settings can be unavailable during queue processing… | partial | task.completion: pending; app.exit: partial; queue.schedule: partial; task.progr… | — |
| completion-program-and-scanner | Completion can open a configured file; scanner arguments acc… | missing | security.scanner: pending; grabber.schedule: pending | — |
| connection-profiles-and-host-caps | Default profile/connection count and per-server exceptions a… | partial | settings.connections: partial; settings.host_connections: pending | — |
| content-identity-recovery | Changed date/size can restart a task; date checking can be d… | partial | task.resume: partial; settings.downloads: partial | — |
| duplicate-policy | Remembered duplicate policies are ask, numbered duplicate, o… | partial | task.duplicate: partial; settings.downloads: partial | — |
| eager-confirmation | Transfer may start while confirmation is visible, so Downloa… | partial | task.add: partial; settings.downloads: partial; queue.assignment: partial | — |
| file-actions | Move/rename, open-with, configurable double-click and comple… | partial | task.properties: partial; task.completion: pending | — |
| force-bypass-capture | Force and bypass modifier sets must not conflict; force capt… | missing | browser.shortcuts: pending | — |
| generated-batch | A URL wildcard expands numeric/alphabetic sequences with wid… | missing | batch.generated: pending | — |
| grabber-auth-and-offline | Crawler supports manual login, excludes logout patterns, use… | missing | grabber.crawl: pending; grabber.destination: pending; grabber.project: pending | — |
| grabber-project-recovery | Corrupt/incompatible projects/templates, partial loading and… | missing | grabber.load: pending; grabber.save: pending; grabber.schedule: pending | — |
| limiter-precedence | Startup limiter default and remembered per-file exceptions/p… | partial | speed.task: partial | — |
| live-stop-save | Stopping live recording can finalize captured content, with … | partial | media.overlay: pending; task.resume: partial | — |
| minimum-media-size | Separate audio/video panel thresholds only apply when total … | missing | media.minimum: pending | — |
| miscapture-exceptions | Two cancellations or similar mistaken captures can prompt fo… | missing | browser.exceptions: pending | — |
| player-panel-policy | Full/mini media panel, site exclusions and protected-content… | partial | media.overlay: pending; browser.types: partial; browser.exceptions: pending | — |
| playlist-link-expansion | Downloaded M3U can offer its MP3 links as one download or a … | missing | media.overlay: pending; browser.all_links: pending | — |
| proxy-selection | Manual, auto-detected, system and none modes; PAC, protocol-… | missing | settings.proxy: pending | — |
| quality-and-track-catalog | Other quality may appear only after player selection; separa… | missing | media.overlay: pending | — |
| queue-independent-start | Named queues have individual start/stop commands; toolbar st… | partial | queue.manage: partial; queue.master: partial | — |
| queue-order-and-retry | Manual queue ordering and a scheduler retry count per failed… | partial | queue.files: partial; queue.schedule: partial | — |
| quota-window | A megabyte-per-hour-window budget can warn, stop downloads a… | missing | settings.quota: pending | — |
| recurrence-and-wake | Daily/weekday and periodic jobs require the application runn… | partial | queue.schedule: partial; grabber.schedule: pending | — |
| remove-versus-delete | Removing rows is separate from deleting completed files; inc… | partial | task.remove: partial; task.remove_completed: pending | — |
| search-scopes | Name/description/source/site/parent/referrer search supports… | partial | gui.search: partial; gui.find_next: pending | — |
| server-name-reconciliation | Redirect/response filename and MIME-derived extension can pr… | partial | task.properties: partial; task.add: partial | — |
| single-use-request | A second request may return HTML instead of the file; origin… | partial | browser.fallback: verified-subset; browser.shortcuts: pending; task.add: partial | — |
| stream-assembly-fallback | Media part processing can seek a matching frame; muxing has … | partial | media.overlay: pending; task.progress: partial | — |
| synchronization | Synchronization and grabber updates revisit sources, update … | partial | queue.files: partial; grabber.schedule: pending | — |
| temporary-storage-cleanup | Final and temporary volumes have separate capacity/file-size… | partial | settings.paths: partial; task.resume: partial | — |
| tls-substitution | Experimental original TLS mode changes a global Windows sett… | partial | settings.connections: partial | — |
| unknown-length-completion | EOF completion includes a notice that a file of unknown size… | partial | task.progress: partial; task.completion: pending | — |
| updater-integrity | Update metadata/signature/download validation and site-rule … | partial | app.update: pending; browser.integration: partial | — |

## Work Queue

### Missing/Partial contracts with NO referenced test (highest priority)

| ID | Title | Status | Source Document |
|---|---|---|---|
| N13 | Ignore modification-time preference | missing | idm_core_queue_contracts.md |
| N14 | User-visible filename/type changes | missing | idm_core_queue_contracts.md |
| N17 | Timeouts, DNS and diagnostics | partial | idm_core_queue_contracts.md |
| N22 | FTP optional connections/pipelining | missing | idm_core_queue_contracts.md |
| N26 | System proxy/PAC/WPAD | partial | idm_core_queue_contracts.md |
| N27 | Browser proxy/VPN context fallback | missing | idm_core_queue_contracts.md |
| N28 | Windows dial-up/VPN and redial | missing | idm_core_queue_contracts.md |
| N31 | Stall recovery and adaptive connections | missing | idm_core_queue_contracts.md |
| N32 | Overlap/data-corruption validation | partial | idm_core_queue_contracts.md |
| N34 | Checkpoint schema and write ordering | partial | idm_core_queue_contracts.md |
| N37 | Crash recovery and stale running states | partial | idm_core_queue_contracts.md |
| N38 | Separate temporary and final directories | missing | idm_core_queue_contracts.md |
| N39 | Disk-full and access-denied assembly | partial | idm_core_queue_contracts.md |
| N40 | Large files and filesystem limits | partial | idm_core_queue_contracts.md |
| N41 | File timestamps | missing | idm_core_queue_contracts.md |
| N43 | Orphan partial cleanup | missing | idm_core_queue_contracts.md |
| N44 | Default/per-server connection caps | partial | idm_core_queue_contracts.md |
| N46 | Aggregate/per-host bandwidth limits | missing | idm_core_queue_contracts.md |
| N47 | Usage quota and automatic resumption | missing | idm_core_queue_contracts.md |
| N48 | Resource efficiency contract | partial | idm_core_queue_contracts.md |
| Q10 | Synchronization repeat/check changed objects | missing | idm_core_queue_contracts.md |
| Q12 | Missed runs and daemon availability | partial | idm_core_queue_contracts.md |
| Q13 | Sleep wake timers | missing | idm_core_queue_contracts.md |
| Q16 | Disconnect modem/VPN and app exit | partial | idm_core_queue_contracts.md |
| Q18 | Force process termination setting | missing | idm_core_queue_contracts.md |
| Q19 | Schedule/order/settings durability | partial | idm_core_queue_contracts.md |
| Q20 | Pause/resume/redownload state semantics | partial | idm_core_queue_contracts.md |
| B02 | Chromium MV3 worker, all HTTP(S) frames at document start; h… | missing | idm_browser_behavior_contracts.md |
| B03 | B `yb/vc` stores client/version/update/silent/consent state;… | partial | idm_browser_behavior_contracts.md |
| B04 | B `Ba/Na/oa`: cycle two WebSocket endpoints then native host… | missing | idm_browser_behavior_contracts.md |
| B05 | B `Ma`: 15 s keepalive for older negotiated versions; listen… | partial | idm_browser_behavior_contracts.md |
| B06 | B `R/Gb/pb`: global/per-tab integration enablement, disabled… | partial | idm_browser_behavior_contracts.md |
| B07 | B `Bb/Nb`: host settings enable menus and listeners; optiona… | missing | idm_browser_behavior_contracts.md |
| B09 | B `Oc` checks competing packaged extension IDs and can disab… | missing | idm_browser_behavior_contracts.md |
| B10 | R registration redirect forwards hash command44, strips hash… | missing | idm_browser_behavior_contracts.md |
| B11 | B `Nb/cb`: before-request observes GET/POST, request body, s… | partial | idm_browser_behavior_contracts.md |
| B12 | B `Rb`: before-send-headers captures headers, correlates pag… | partial | idm_browser_behavior_contracts.md |
| B13 | B `Ub/Hc`: response status, MIME, Content-Disposition, chars… | partial | idm_browser_behavior_contracts.md |
| B14 | B `Qb/cb/X`: maintains earlier URL/headers/body across redir… | missing | idm_browser_behavior_contracts.md |
| B15 | B `Fc/Gc/Hc/Y/yc`: host-provided extension/MIME/site/path ex… | partial | idm_browser_behavior_contracts.md |
| B16 | B `qb`: onCreated correlates recent request by URL/time (obs… | partial | idm_browser_behavior_contracts.md |
| B17 | B `rb/lc/X`: filename listener filters then submits type13, … | partial | idm_browser_behavior_contracts.md |
| B18 | B `$a` reply13 maps action and status3 to `kc/O`; accepted C… | partial | idm_browser_behavior_contracts.md |
| B19 | B `oa` removes integration listeners; P disconnect-after-sen… | partial | idm_browser_behavior_contracts.md |
| B20 | B `Sb/Tb/O`: completion/error release correlated request; he… | missing | idm_browser_behavior_contracts.md |
| B22 | B `Kc/Bc`: CRLF request/response headers; raw bytes, URL-enc… | partial | idm_browser_behavior_contracts.md |
| B23 | B `Lc`: walk frame ancestry past about:blank for referrer; p… | partial | idm_browser_behavior_contracts.md |
| B24 | B `bb/Rb/Sb`, host requests20: direct/system/auto/PAC/fixed/… | missing | idm_browser_behavior_contracts.md |
| B25 | B `rc/Cb/Fb/wb`, C key/mouse31/32/33: host-defined force/byp… | missing | idm_browser_behavior_contracts.md |
| B26 | B `Nb/Mb`, R: configured main-frame registration redirect us… | missing | idm_browser_behavior_contracts.md |
| B27 | B `T/dc`: HTTP/HTTPS/FTP/FTPS plus proprietary registration/… | partial | idm_browser_behavior_contracts.md |
| B28 | B `Cb/Ea/pa`: link,image,video context download; page/frame/… | missing | idm_browser_behavior_contracts.md |
| B29 | B `Ea/Eb`, C `Zb`: media/image source can ask DOM anchor loo… | missing | idm_browser_behavior_contracts.md |
| B30 | B `Qa/Db/Zb/ec`, C `Xb/nb/N`: selected links and all links c… | missing | idm_browser_behavior_contracts.md |
| B32 | B `Db/Zb` + C mouse26: selected text can create desktop sele… | missing | idm_browser_behavior_contracts.md |
| B33 | B `nb/Ea` menu5: browser downloads-page link search; erases … | missing | idm_browser_behavior_contracts.md |
| B34 | B `fc`: no host/no links produces browser-side error notific… | partial | idm_browser_behavior_contracts.md |
| B35 | B `Aa/Hc`: direct audio/video, MPEGTS, M4S, HLS/DASH/F4M, su… | missing | idm_browser_behavior_contracts.md |
| B36 | B `X`: media detection type13 action discriminator differs p… | missing | idm_browser_behavior_contracts.md |
| B37 | B reply13→content19; C `bc`: browser-context fetch with URL/… | missing | idm_browser_behavior_contracts.md |
| B38 | D `ua/Ka/La/Ja`: configured XHR open/header/send/loadend wra… | missing | idm_browser_behavior_contracts.md |
| B39 | D `la/Ma/P/Na/ya`: fetch wrapper and Response text/json/arra… | missing | idm_browser_behavior_contracts.md |
| B40 | D message1229212978 config dynamically installs/restores wra… | missing | idm_browser_behavior_contracts.md |
| B41 | D message1229212981 property/selector extraction→1229212982;… | missing | idm_browser_behavior_contracts.md |
| B42 | C `Gc/qc/Jc/xb/Ic/lc/mc/oc/nc`: host-version-gated site rule… | missing | idm_browser_behavior_contracts.md |
| B43 | C `Tb/H/ib`: video/audio/object/embed/source-child associati… | partial | idm_browser_behavior_contracts.md |
| B44 | C `R/ib/jb/wb/O`: stable element/site IDs, replacement-playe… | missing | idm_browser_behavior_contracts.md |
| B45 | C `P/Lb/fa`: visibility/opacity/dimensions/clipped ancestors… | missing | idm_browser_behavior_contracts.md |
| B46 | C `Sb/cc`, B `hc/ic/nc`: nested frame IDs/rectangles transla… | partial | idm_browser_behavior_contracts.md |
| B47 | C mutation/resize/intersection observers, 200 ms debounce `W… | partial | idm_browser_behavior_contracts.md |
| B48 | B `ba/Fb/oc/ga/fc`: native panel states hidden/visible/backg… | partial | idm_browser_behavior_contracts.md |
| B49 | B window listeners and `U/sc/pc/Oa`: window registration6, c… | missing | idm_browser_behavior_contracts.md |
| B50 | B `ab/xb/Ob/xa`: navigation and history changes refresh site… | partial | idm_browser_behavior_contracts.md |
| B51 | C `C/$b/ob` + B `ec`: title from top page/og:title, source c… | missing | idm_browser_behavior_contracts.md |
| B52 | L21130 instructs selecting desired quality in player before … | partial | idm_browser_behavior_contracts.md |
| B53 | L21005/21006 audio-only/video-only; mux exports establish se… | partial | idm_browser_behavior_contracts.md |
| B54 | L21192 incompatible audio/video can leave audio separately; … | partial | idm_browser_behavior_contracts.md |
| B56 | B `$a`12/`Ac/mc`: host resource asks browser to locate assoc… | missing | idm_browser_behavior_contracts.md |
| B57 | B `$a`24/`Dc`: host explicitly requests cookies for URL; res… | partial | idm_browser_behavior_contracts.md |
| B58 | B `$a`20: host asks proxy comparison/context; reply121/122 a… | missing | idm_browser_behavior_contracts.md |
| B59 | B `$a`22/`Ec`: optional notification permission, desktop-sup… | missing | idm_browser_behavior_contracts.md |
| B62 | Standard four-byte native-endian UTF-8 JSON length, outer JS… | partial | idm_browser_behavior_contracts.md |
| address-refresh | Expired task enters a wait-for-new-session/address flow; one… | partial | idm_behavior_review.md |
| all-links-and-clipboard | All-link collection can hide HTML/images/duplicates and use … | partial | idm_behavior_review.md |
| appearance-and-progress | System dark mode, font choice/reset and progress tab/start-v… | partial | idm_behavior_review.md |
| browser-platform-health | Original health depends on elevation, BFE, drivers/helpers a… | partial | idm_behavior_review.md |
| category-retroactive | Extension/site rules and last-used paths classify future fil… | partial | idm_behavior_review.md |
| completion-power | Task, queue and grabber completion can request shutdown/slee… | missing | idm_behavior_review.md |
| completion-precedence | Per-file settings can be unavailable during queue processing… | partial | idm_behavior_review.md |
| completion-program-and-scanner | Completion can open a configured file; scanner arguments acc… | missing | idm_behavior_review.md |
| connection-profiles-and-host-caps | Default profile/connection count and per-server exceptions a… | partial | idm_behavior_review.md |
| content-identity-recovery | Changed date/size can restart a task; date checking can be d… | partial | idm_behavior_review.md |
| duplicate-policy | Remembered duplicate policies are ask, numbered duplicate, o… | partial | idm_behavior_review.md |
| eager-confirmation | Transfer may start while confirmation is visible, so Downloa… | partial | idm_behavior_review.md |
| file-actions | Move/rename, open-with, configurable double-click and comple… | partial | idm_behavior_review.md |
| force-bypass-capture | Force and bypass modifier sets must not conflict; force capt… | missing | idm_behavior_review.md |
| generated-batch | A URL wildcard expands numeric/alphabetic sequences with wid… | missing | idm_behavior_review.md |
| grabber-auth-and-offline | Crawler supports manual login, excludes logout patterns, use… | missing | idm_behavior_review.md |
| grabber-project-recovery | Corrupt/incompatible projects/templates, partial loading and… | missing | idm_behavior_review.md |
| limiter-precedence | Startup limiter default and remembered per-file exceptions/p… | partial | idm_behavior_review.md |
| live-stop-save | Stopping live recording can finalize captured content, with … | partial | idm_behavior_review.md |
| minimum-media-size | Separate audio/video panel thresholds only apply when total … | missing | idm_behavior_review.md |
| miscapture-exceptions | Two cancellations or similar mistaken captures can prompt fo… | missing | idm_behavior_review.md |
| player-panel-policy | Full/mini media panel, site exclusions and protected-content… | partial | idm_behavior_review.md |
| playlist-link-expansion | Downloaded M3U can offer its MP3 links as one download or a … | missing | idm_behavior_review.md |
| proxy-selection | Manual, auto-detected, system and none modes; PAC, protocol-… | missing | idm_behavior_review.md |
| quality-and-track-catalog | Other quality may appear only after player selection; separa… | missing | idm_behavior_review.md |
| queue-independent-start | Named queues have individual start/stop commands; toolbar st… | partial | idm_behavior_review.md |
| queue-order-and-retry | Manual queue ordering and a scheduler retry count per failed… | partial | idm_behavior_review.md |
| quota-window | A megabyte-per-hour-window budget can warn, stop downloads a… | missing | idm_behavior_review.md |
| recurrence-and-wake | Daily/weekday and periodic jobs require the application runn… | partial | idm_behavior_review.md |
| remove-versus-delete | Removing rows is separate from deleting completed files; inc… | partial | idm_behavior_review.md |
| search-scopes | Name/description/source/site/parent/referrer search supports… | partial | idm_behavior_review.md |
| server-name-reconciliation | Redirect/response filename and MIME-derived extension can pr… | partial | idm_behavior_review.md |
| single-use-request | A second request may return HTML instead of the file; origin… | partial | idm_behavior_review.md |
| stream-assembly-fallback | Media part processing can seek a matching frame; muxing has … | partial | idm_behavior_review.md |
| synchronization | Synchronization and grabber updates revisit sources, update … | partial | idm_behavior_review.md |
| temporary-storage-cleanup | Final and temporary volumes have separate capacity/file-size… | partial | idm_behavior_review.md |
| tls-substitution | Experimental original TLS mode changes a global Windows sett… | partial | idm_behavior_review.md |
| unknown-length-completion | EOF completion includes a notice that a file of unknown size… | partial | idm_behavior_review.md |
| updater-integrity | Update metadata/signature/download validation and site-rule … | partial | idm_behavior_review.md |

**Count: 121**

### Partial contracts WITH tests (next priority)

| ID | Title | Status | Tests | Source Document |
|---|---|---|---|---|
| N02 | HTTP methods and request bodies | partial | tests/integration/request_context.py | idm_core_queue_contracts.md |
| N03 | One-shot and expiring URLs | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N07 | Known and unknown response length | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N08 | Redirect chains and credentials | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N09 | Cookie/User-Agent/Referer/header forwarding | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N11 | Authentication retries and expiry | partial | tests/integration/credentials.py, crates/xdm-daemon/src/credentials.rs | idm_core_queue_contracts.md |
| N12 | Remote-object validators | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N15 | Permission/permanent HTTP errors | partial | tests/integration/http_engine.py, tests/integration/cli_http.py | idm_core_queue_contracts.md |
| N16 | Transient server/network retries | partial | tests/integration/http_engine.py, tests/integration/cli_http.py | idm_core_queue_contracts.md |
| N19 | FTP authentication/control replies | partial | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N20 | FTP passive and active data modes | partial | crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N21 | FTP REST/SIZE and resume | partial | tests/integration/protocol_tasks.py, crates/xdm-ftp/tests/ftp.rs | idm_core_queue_contracts.md |
| N23 | Manual per-protocol proxy | partial | tests/integration/http_engine.py, tests/integration/request_context.py | idm_core_queue_contracts.md |
| N24 | SOCKS4/SOCKS5 and remote DNS | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N25 | Proxy exclusions and host patterns | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N30 | Dynamic splitting and worker reuse | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N36 | Non-range pause/restart | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N42 | Finalize, rename and output conflicts | partial | tests/integration/http_engine.py | idm_core_queue_contracts.md |
| N45 | Per-file limiter and remembered exceptions | partial | tests/integration/task_rate.py | idm_core_queue_contracts.md |
| Q01 | Queue types and default queues | partial | tests/integration/daemon_ipc.py | idm_core_queue_contracts.md |
| Q02 | Named queues and membership | partial | tests/integration/named_queues.py | idm_core_queue_contracts.md |
| Q03 | Independent start/stop and global controls | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q05 | Manual queue order and priority | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q06 | Completion leaves ordinary queue | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q07 | Queue retry count and rotation | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q08 | One-time start/stop | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q09 | Daily/weekday recurrence | partial | tests/integration/queue_policies.py | idm_core_queue_contracts.md |
| Q14 | Completion success/failure/manual-stop rules | partial | tests/integration/completion_actions.py, tests/integration/queue_policies.py | idm_core_queue_contracts.md |

**Count: 28**
