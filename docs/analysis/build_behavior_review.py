"""Reviewed semantic sections of the complete shipped language catalog.

These are manual thematic associations, not recovered runtime caller identities.
The pinned input digest prevents index-based review from silently drifting.
No source labels from this catalog are used in the application.
"""
import argparse
import hashlib
import json
from pathlib import Path

from build_feature_coverage import build as coverage

ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "docs/analysis/idm_language_catalog.json"
DIGEST = "507b3bef06362820a7b1d201cbf6e081ffb45a52e4b21a50891aa8746778539b"

# Inclusive catalog indexes. Every text was read before making these associations.
# A section can contain several functions; it is deliberately not a caller claim.
# start, end, feature families, original surface/behavior indicated by its text
SECTIONS = [
    (0, 24, "gui.shell task.resume task.remove queue.master settings.options queue.schedule app.help", "Task columns, toolbar commands, selected-file actions, all-download stop, settings and scheduler entry points."),
    (25, 33, "app.commercial task.confirmation", "Registration identity and activation fields, license information, OK and cancel."),
    (34, 60, "task.progress task.resume", "Expandable per-file progress, connection-position display, resume capability, connection details and time formatting."),
    (61, 74, "task.add task.archive auth.challenge queue.assignment", "Start/later confirmation with filename, folder, category, description, preview and optional credentials."),
    (75, 103, "settings.options settings.general browser.types settings.connections settings.downloads settings.proxy auth.store platform.dialup settings.sounds", "Configuration tabs, startup, clipboard monitoring, browser integration and capture extensions."),
    (104, 124, "settings.proxy", "Separate HTTP, FTP and HTTPS proxies, credentials, browser-setting retrieval and passive FTP selection."),
    (125, 127, "settings.connections settings.quota", "Connection-speed profile, default connection count and transfer quota."),
    (128, 149, "settings.paths settings.downloads security.scanner task.progress platform.dialup", "Remembered save/temp paths, start/complete/progress dialogs, scanner arguments and dial-up retry interval/count."),
    (150, 152, "settings.sounds", "Event sound file selection and playback preview."),
    (153, 158, "auth.store", "Add, edit and remove site/path credential profiles."),
    (159, 169, "task.properties task.completion auth.store", "File properties include type, size, destination, credentials, open action and last-attempt result."),
    (170, 184, "app.about app.update app.commercial", "Version/project/support links, licensing and update entry points."),
    (185, 187, "power.shutdown", "Queue-completion shutdown announcement and immediate shutdown action."),
    (188, 191, "task.confirmation", "Yes/no and apply-to-all choices; the labels alone do not identify callers."),
    (192, 227, "queue.schedule queue.files settings.quota platform.dialup power.shutdown app.exit", "One-off/daily/weekday queue schedule, stop time, simultaneous files, reorder/remove, quota warning, modem disconnect, exit and forced termination."),
    (228, 234, "platform.dialup", "Connection progress, manual redial, selected connection and saved credentials."),
    (235, 237, "app.update", "Optional component download dialog."),
    (238, 244, "lists.export lists.import", "Export queue, selection or all files; import-link editing and deletion."),
    (245, 252, "browser.types app.tips", "Capture extension list and startup/next-tip preferences, including missing or unreadable tip files."),
    (253, 256, "auth.store", "Path-specific login profile with password confirmation."),
    (257, 262, "app.update settings.quota", "Update discovery/install and quota-exceeded notice."),
    (263, 267, "task.archive", "ZIP member preview columns: name, packed size, size and modification date."),
    (268, 274, "settings.categories settings.paths", "Category name, extension masks, remembered folder and future destination."),
    (275, 316, "gui.shell task.add gui.drop lists.export lists.import task.resume task.remove queue.master task.remove_completed queue.schedule settings.options gui.category_visibility gui.sort gui.language app.help app.tips app.update app.about app.commercial", "Main menu hierarchy, sorting criteria, category visibility, language, help and commercial entries."),
    (317, 340, "app.about platform.dialup", "About command and Windows RAS connection/authentication state messages."),
    (341, 348, "task.completion settings.paths auth.store app.commercial", "Completion summary, temp-to-final rename failure, login validation and credential deletion confirmation."),
    (349, 356, "auth.store auth.challenge settings.proxy", "Apply changed profile credentials to existing tasks, duplicate profile prevention, site authorization and proxy authorization."),
    (357, 371, "task.wait settings.connections settings.paths", "Connection/memory/permission/DNS/network failures, connection exhaustion and final-volume disk-full handling."),
    (372, 378, "settings.paths settings.proxy task.wait", "Partial-volume disk full, origin/proxy lookup and connect failures, and separate connect timeouts."),
    (379, 386, "task.resume settings.paths", "Assembly and recovery failures: part size, memory, open/read/write errors and old-part rebuilding."),
    (387, 412, "task.wait settings.proxy task.resume", "HTTP/socket retry states and FTP control/data connection, reply parsing, passive/active mode, unknown size and restart support errors."),
    (413, 430, "task.properties task.progress task.wait", "Redirect filename-change confirmation, assembly, optional connections and per-connection state messages."),
    (431, 472, "task.progress settings.proxy task.resume", "FTP command sequence/pipelining probes; authentication, PASV/PORT, CWD, TYPE, REST, RETR, SIZE and LIST states."),
    (473, 489, "task.resume task.progress settings.paths", "Missing-local-data restart, HTTP HEAD/GET, partial rebuild, total-byte result, disk space, completed-task path guard and changed-server-date restart prompt."),
    (490, 495, "app.update task.properties task.progress", "Latest-version notice, settings-save failure, memory/DNS failure, rebuilding and uncertain completion for unknown-size responses."),
    (496, 499, "queue.files gui.shell", "No-selection validation and queue move-up/move-down/delete actions."),
    (500, 516, "task.wait app.update app.help task.completion task.duplicate platform.dialup browser.integration task.properties", "Malformed HTTP reply, cancellation, missing file, authorization, open/move result and integration registry permissions."),
    (517, 528, "task.duplicate lists.import task.resume task.properties media.overlay", "Destination/list conflicts, import corruption, FTP restart choice, server filename/type reconciliation and M3U-to-MP3-link expansion prompts."),
    (529, 545, "security.scanner platform.dialup settings.quota app.help task.resume settings.proxy", "Scanner launch failure, modem disconnect countdown, quota validation, help installation, changed-size restart, proxy and TLS failures."),
    (546, 553, "settings.options settings.sounds", "Download/queue start-stop-success-failure sound events and sound-file copy failure."),
    (554, 579, "app.commercial settings.categories gui.category_visibility settings.paths", "Built-in categories, create/edit/delete validation, folder creation and retroactive category reassignment without moving files."),
    (580, 582, "task.resume settings.paths", "Assembly denied by an open file, per-category default folder and last-selected folder."),
    (583, 630, "platform.dialup gui.tray app.help app.commercial task.resume task.remove task.properties settings.proxy task.duplicate app.update auth.store queue.assignment queue.manage task.completion task.remove_completed", "Tray and task context actions, URL validation, duplicate/resume choices, queue commands, open-only-one selection and completed-list removal confirmation."),
    (631, 638, "gui.drop app.about settings.paths task.progress", "Drop target, copyright information, folder creation errors and generic file/status labels."),
    (639, 650, "security.scanner task.properties app.commercial", "Completion scanning state and media metadata summaries for AVI/WAV/MPEG and music tags; trial/full labels."),
    (651, 655, "platform.dialup gui.language settings.quota", "Redial result, restart on language change, quota stop and automatic resume at the next allowed window."),
    (656, 679, "task.progress lists.import lists.export settings.paths browser.fallback security.scanner settings.sounds app.help task.properties", "Assembly/disconnect states, list formats, assembly interruption notice, browser handoff status, path/file pickers and normal/minimized/hidden progress preferences."),
    (680, 683, "app.commercial gui.language gui.toolbar", "Commercial edition gates, first-run language choice and customization entry."),
    (684, 703, "gui.columns gui.sort gui.toolbar settings.categories", "Additional columns and sort keys, column show/hide/order/reset/width and Compressed category."),
    (704, 713, "settings.host_connections", "Per-server connection-count exceptions with default/new/edit/delete and server/count list."),
    (714, 726, "browser.all_links task.properties task.redownload task.completion", "All-page-link selection, link text descriptions, HTML/duplicate filtering, async type checks, extension reconciliation, redownload and open-folder."),
    (727, 733, "task.add task.resume app.update app.commercial browser.shortcuts browser.integration task.wait", "Unsupported scheme, assembly permission failure, update-license gates, conflicting modifier keys, DNS guidance and missing integration files."),
    (734, 742, "settings.general browser.integration browser.shortcuts", "Detect new download applications, browser selection, force/bypass modifiers and optional left-click requirement."),
    (743, 749, "settings.paths settings.proxy", "Batch category/single-folder destination choices and proxy prefix bypass syntax."),
    (750, 759, "browser.integration settings.general browser.all_links", "Network integration repair/reboot/application restart, hide page images and advanced integration toggle."),
    (760, 779, "network.check app.report settings.proxy platform.dialup browser.integration", "Test-page diagnostics, review/send problem report, first-run proxy/dial-up import and integration conflict recovery."),
    (780, 790, "browser.integration task.redownload queue.manage app.tips browser.types browser.exceptions", "Advanced integration installation/reboot, queue start-stop and automatic-capture guidance with disable-tip preference."),
    (791, 804, "app.commercial gui.toolbar task.progress", "License conflict, toolbar styles and byte/time units."),
    (805, 807, "task.resume grabber.wizard", "Interrupted assembly corruption guidance and wizard back/next navigation."),
    (808, 837, "grabber.filters grabber.files grabber.statistics grabber.project grabber.wizard", "Crawler include/exclude type, size, path/domain filters; same-site selection, eager download, statistics, seed/name/template and manual login."),
    (838, 846, "grabber.destination grabber.files grabber.settings", "Relative folders, offline HTML-link rewriting, overwrite, add selected results to main list, link descriptions and independent explore/download concurrency."),
    (847, 857, "grabber.crawl", "Parent-directory prohibition, base/external depth, whole-site scope, include/exclude page paths, same-domain sites and popup exclusion."),
    (858, 882, "grabber.templates grabber.save grabber.load grabber.project grabber.recent grabber.settings grabber.schedule grabber.files app.help", "Template/project save/load/recent/menu actions and separate explore/download/check/add-to-main toolbar commands."),
    (883, 888, "grabber.templates", "Preset crawl plans for pictures, audio, video, non-page files and whole-site capture."),
    (889, 939, "grabber.wizard grabber.project grabber.files grabber.templates grabber.save grabber.load grabber.settings grabber.schedule app.help", "Wizard stages, manual login validation, main-list capacity errors, reserved/duplicate names, project lifecycle and damaged/incompatible/partial project loading."),
    (940, 946, "browser.integration auth.challenge auth.store", "Legacy integration scope and origin/realm authentication prompt with save-password choice."),
    (947, 965, "batch.generated browser.all_links settings.paths", "Numeric/alphabetic wildcard URL generation, fixed width, first/second/last preview, missing-wildcard validation and original 1000-file bound."),
    (966, 982, "grabber.filters grabber.files grabber.wizard", "Named file-type filter presets, site-map view and unique nonempty filter names/masks."),
    (983, 1005, "grabber.cleanup grabber.save grabber.load grabber.project grabber.crawl grabber.settings grabber.files", "Project cleanup/load/save/apply states, disk-space failures, complete-page assets, manual login/logout masks, duplicate locations, JavaScript processing and legacy IE-cache lookup."),
    (1006, 1028, "grabber.files grabber.filters grabber.statistics", "Link/folder/thumbnail views, external-site grouping, explored/matched/downloaded totals and missing/forbidden/error results."),
    (1029, 1055, "grabber.schedule grabber.project grabber.crawl power.shutdown", "One-time versus periodic synchronization, explore-only/explore-and-download/checked-download, completion program, periodic interval, running/editing skip, re-exploration and changed/new-file update policy without deleting old discoveries."),
    (1056, 1070, "grabber.project gui.tray task.completion settings.paths settings.downloads", "Project rename, tray minimize/restore all progress windows, unknown-size completion, remembered category folder and completion open-folder/dismiss preference."),
    (1071, 1092, "settings.downloads app.update browser.exceptions gui.tray task.resume settings.categories settings.paths task.add task.properties", "Ignore-modification-date preference, update interval, address wildcard, parent referrer, no-resume restart choice, eager downloading during confirmation and download-later partial-data behavior."),
    (1093, 1098, "power.restart browser.types", "Restart countdown/immediate/cancel and captured extension types."),
    (1099, 1135, "browser.integration browser.firefox browser.shortcuts task.completion", "Browser discovery/version/executable details, reintegration/plugin/extension install-remove-restart, legacy exception/modifier limits and open-with."),
    (1136, 1151, "task.duplicate settings.downloads task.completion browser.integration", "Duplicate URL policies: ask, numbered filename, overwrite, completed-dialog or resume; remembered choice and drag completed file from its icon."),
    (1152, 1159, "platform.privilege browser.integration settings.general", "Elevated filesystem operation confirmation, installation reboot/startup and legacy browser click-monitoring module."),
    (1160, 1172, "browser.exceptions", "Prompt after two canceled interceptions, site/address/wildcard exceptions and preference to suppress that prompt."),
    (1173, 1175, "browser.fallback browser.shortcuts task.add", "Single-use request replay can return HTML; eager request reuse and interception before the browser's first request are suggested workarounds."),
    (1176, 1210, "queue.manage queue.files queue.schedule settings.quota queue.assignment", "Named Main/Synchronization/custom queues, membership indicator, create/delete/edit/start/stop, schedule retry count, tray-running requirement and toolbar queue dropdowns."),
    (1211, 1230, "task.resume browser.firefox browser.context media.overlay browser.all_links browser.integration", "Nonresumable queue retry-from-zero guidance, extension updates, recent ten FLV requests, all-link context action and player-not-yet-requested guidance."),
    (1231, 1247, "browser.integration browser.firefox app.exit browser.exceptions app.report", "Browser context-menu item selection, exit with other active downloads, similar-address exclusions and miscapture report with parent referrer."),
    (1248, 1261, "speed.task", "Global limiter enable/default and per-download override, startup preference, remembered exceptions, maximum per-file rate and no-resume warning."),
    (1262, 1264, "media.overlay settings.paths platform.privilege", "Unassociated media fallback to recent requests, CRC error and all-user registration elevation."),
    (1265, 1271, "task.remove task.remove_completed", "Single/multi/incomplete/completed removal confirmations with separate optional deletion of completed files on disk."),
    (1272, 1292, "media.overlay browser.types", "Full/mini player panel, download/all/close/about actions, eligible media types and option to prevent automatic player capture when the panel is disabled."),
    (1293, 1296, "settings.general browser.integration platform.privilege browser.firefox", "Start on startup, advanced integration requirement and mismatched browser/desktop elevation failure."),
    (1297, 1312, "task.properties task.wait task.resume browser.fallback", "Expired-address refresh waits for recaptured URL/session, one pending refresh at a time, reopen owner page, cancel wait, confirm matching file and handle single-use HTML."),
    (1313, 1325, "queue.assignment browser.all_links", "No checked files, remembered queue-selection panel for later/batch, no-queue option, start selected queue and clipboard multi-URL detection."),
    (1326, 1331, "gui.tray app.commercial", "Tray icon styles/hidden preference and license-expiry notices."),
    (1332, 1337, "queue.schedule auth.store", "Require at least one scheduled weekday and prompt before trimming leading/trailing username/password spaces."),
    (1338, 1346, "browser.selection_panel", "Selected-text link panel, always/never/site-only policy and selected-file batch action."),
    (1347, 1356, "browser.integration browser.fallback platform.privilege app.help", "Browser selection, missing helper, HTML miscapture/site help and elevation/compatibility troubleshooting."),
    (1357, 1363, "task.properties browser.integration settings.general task.completion", "Media dimensions/duration, all-user advanced-integration changes, application-detection cost, driver compatibility and missing player guidance."),
    (1364, 1383, "gui.category_visibility task.add queue.assignment settings.general gui.drop browser.fallback task.properties gui.toolbar app.help browser.firefox app.commercial browser.shortcuts", "Toolbar tips, clipboard URL prompt, HTML save/rename choices, toolbar discovery, Firefox restart, installation-layout errors and force-key exclusion of page/script/image requests."),
    (1384, 1392, "browser.integration media.overlay browser.types platform.privilege app.commercial task.completion", "Integration/panel diagnostics, excluded media behavior, driver startup failure and completion-option availability restrictions under queue processing."),
    (1393, 1411, "task.progress task.completion app.exit platform.dialup power.shutdown media.overlay speed.task", "Per-file completion actions, protected-stream notification, limiter persistence and configurable progress tabs/start view; queue progress begins minimized."),
    (1412, 1414, "platform.dialup media.overlay", "VPN connection troubleshooting and quality label."),
    (1415, 1428, "settings.proxy", "Per-protocol proxy plus SOCKS with authorization/type/connect failures and exceptions."),
    (1429, 1431, "media.overlay browser.exceptions", "Site wildcard exclusions for the player panel."),
    (1432, 1433, "browser.integration", "Original advanced integration depends on Windows Base Filtering Engine availability."),
    (1434, 1443, "browser.types browser.selection_panel settings.categories app.commercial app.update settings.paths browser.firefox", "Custom file type/site entry, category site masks, recent items, update-system repair prompts, server creation date and Firefox component update wait."),
    (1444, 1445, "task.resume", "Inconsistent data across connections triggers single-connection fallback or restart from zero."),
    (1446, 1449, "browser.integration settings.proxy", "Chrome extension update notice, PAC selection/authorization and browser PAC import."),
    (1450, 1451, "media.overlay task.resume", "Stopping a live broadcast offers saving a completed file and warns that the same position cannot be resumed."),
    (1452, 1460, "settings.proxy browser.integration task.wait task.progress media.overlay", "PAC download failure, firewall guidance, media-part overlap/frame matching, assembly errors, unsupported media capture and Chrome/incognito extension enablement."),
    (1461, 1467, "settings.connections settings.paths", "Low/medium/high connection profiles, excessive-connection warning, filename validation and final/temp filesystem maximum-file-size errors."),
    (1468, 1469, "media.overlay", "Audio-only and video-only representations."),
    (1470, 1473, "app.update browser.integration task.properties", "Invalid update information, integration repair and move/move-rename task actions."),
    (1474, 1484, "gui.search gui.find_next", "Find by name/description, case and whole-string flags, no-results/end-of-search and Ctrl-F/F3 commands."),
    (1485, 1491, "app.update media.overlay task.completion grabber.settings browser.integration settings.paths", "Updater launch failure, quality discovery by player switching, destination picker, missing codec playback guidance and advanced web-page processing."),
    (1492, 1500, "queue.schedule app.update app.commercial app.help", "Wake-timer requirement, Windows sleep-mode caveat, updater/hosts repair behavior and commercial license conflict."),
    (1501, 1502, "settings.paths app.commercial", "Prompt to delete unreferenced old partial files and commercial identifier display."),
    (1503, 1508, "media.minimum settings.connections", "Separate minimum audio/video panel sizes; unknown-size media bypasses the threshold; obsolete TLS-library guidance."),
    (1509, 1520, "power.shutdown queue.schedule grabber.schedule task.completion", "Shutdown, sleep and hibernate after task, queue or grabber completion, with immediate/countdown controls."),
    (1521, 1529, "browser.firefox settings.proxy browser.integration", "Firefox advanced-integration requirement, manual/auto/system/no-proxy modes, WPAD lookup failure and Edge extension installation."),
    (1530, 1540, "app.commercial app.report browser.integration browser.fallback settings.proxy", "Site rule reporting/update/version compatibility, request-before-browser for single-use links, insufficient replay context and browser VPN/proxy mismatch."),
    (1541, 1544, "security.scanner settings.downloads", "Manual URL User-Agent, completion scanner executable, argument expansion with a file token and default appended filename."),
    (1545, 1547, "media.overlay task.completion task.properties", "Incompatible audio is saved separately; double-click action is configurable and incomplete-file open is refused."),
    (1548, 1558, "browser.integration settings.connections settings.proxy browser.firefox", "Missing site rules/helper module repair, experimental global Windows TLS setting, SSL library errors, SOCKS applies to protocols excluded from proxy and browser-proxy retry preference."),
    (1559, 1563, "media.overlay app.commercial gui.appearance", "Subtitle label, trial notice and system-controlled dark mode with restart."),
    (1564, 1573, "app.update app.commercial", "Updater signature/download failure and commercial license/trial/purchase states."),
    (1574, 1583, "gui.search gui.find_next gui.sort", "Search bar close/settings, enabled search scopes including source site/link/parent/referrer, required scope selection and parent-page sorting."),
    (1584, 1585, "queue.assignment browser.fallback app.report", "Queue-only add option and site-rule handoff failure report."),
    (1586, 1590, "gui.appearance", "Custom/reset/system font and restart to apply main-window font changes."),
    (1591, 1594, "media.overlay platform.dialup", "Audio/video mixing state and Windows dial-up/VPN selection."),
    (1595, 1598, "app.update browser.integration app.commercial", "Frequent update guidance, required Windows platform update and license/support email validation."),
    (1599, 1603, "browser.integration", "Advanced integration enabled/disabled/restart/enable status and Edge extension enablement notice."),
]

# Fine-grained state transitions not adequately captured by dialog-family names.
# id, language keys, families, original behavior, independent requirement, test scenario
CONTRACTS = [
    ("address-refresh", "20867 20868 20870 20873 20874 20875 20876 20877 20878", "task.properties task.resume task.wait", "Expired task enters a wait-for-new-session/address flow; one refresh at a time; owner page reopening and user confirmation identify the replacement.", "Persist an explicit refresh state, correlate a new browser request to the paused task and verify object identity before retaining old bytes; cancel returns to paused.", "Expired signed URL plus changed cookies; matching recapture resumes with identical hash; mismatched object is refused; second refresh and canceled/late recapture are handled."),
    ("single-use-request", "20763 20764 20879 21184", "browser.fallback browser.shortcuts task.add", "A second request may return HTML instead of the file; original guidance includes early interception, eager download and avoiding pause for one-shot links.", "Record method/body and request lifetime. Do not promise replayability; avoid unnecessary probes and preserve browser ownership when a supported handoff cannot be established.", "One-use GET and POST fixtures; the target file stays intact or the browser completes; HTML login/error output is not silently published under a media filename."),
    ("eager-confirmation", "20700 1623 1862", "task.add settings.downloads queue.assignment", "Transfer may start while confirmation is visible, so Download Later can retain already received bytes; queue-only mode is also offered.", "Represent provisional, confirmed, deferred and canceled task states, with an explicit user preference and bounded provisional transfer; document queue-only precedence.", "Slow fixture; select later/cancel/start before and after bytes arrive; verify ownership, partial retention and absence of unrequested final output."),
    ("duplicate-policy", "20742 20743 20744 20745 1645 1646", "task.duplicate settings.downloads", "Remembered duplicate policies are ask, numbered duplicate, overwrite, or show completion/resume according to task state.", "Keep URL identity separate from destination collision; make overwrite explicit and race safe; expose policy independently of current fresh-path redownload.", "Same URL with completed/paused/active rows, existing destination and competing submissions; verify each policy and no accidental overwrite."),
    ("content-identity-recovery", "20150 20237 1615 20995 20996", "task.resume settings.downloads", "Changed date/size can restart a task; date checking can be disabled; mismatched bytes can reduce connection count or restart.", "Never infer safe resume from size alone. Keep strict validators; define explicit restart choices and safe single-stream retry on range inconsistency.", "Same-size changed content, weak/missing ETag, changed date, inconsistent ranges and corrupt checkpoint; no mixed-version final output."),
    ("unknown-length-completion", "20156 20692", "task.progress task.completion", "EOF completion includes a notice that a file of unknown size might be incomplete.", "Distinguish EOF, verified declared length and interrupted transport; show indeterminate progress and avoid fabricated ETA/100% before EOF.", "Chunked response succeeds; abrupt disconnect fails; canceled unknown-length task keeps recoverable state without a completed row."),
    ("server-name-reconciliation", "20074 20219 20513", "task.properties task.add", "Redirect/response filename and MIME-derived extension can prompt to replace the initially selected name.", "Sanitize server suggestions and retain explicit user filename overrides; check destination conflicts before publishing a renamed output.", "Unicode/quoted disposition, MIME mismatch, redirected name, traversal payload, explicit override and an existing destination."),
    ("queue-independent-start", "20773 20774 t34030 t34031", "queue.manage queue.master", "Named queues have individual start/stop commands; toolbar start/stop can choose a queue.", "Compare original independent start with XDM's current global gate; specify whether enabling a queue also starts its dispatcher before claiming equivalent behavior.", "Start one queue while all are stopped; stop it with another queue active; verify each queue and the global concurrency bound."),
    ("queue-order-and-retry", "t1229 t1230 1680", "queue.files queue.schedule", "Manual queue ordering and a scheduler retry count per failed file are exposed separately from connection retry.", "Persist queue position and queue-attempt policy; keep network retries distinct from whole-task retries and user-paused tasks.", "Reorder three jobs across restart; transient and permanent failures; verify retry count, skip/stop rules and paused-task exclusion."),
    ("synchronization", "20769 1584 20682 20683 20684", "queue.files grabber.schedule", "Synchronization and grabber updates revisit sources, update changed files, optionally add new matches, and retain old discoveries.", "Add retained-source queue mode, object validator checks and atomic replacement; define discovery retention and update scope independently of download queues.", "Unchanged/changed/new/deleted objects, concurrent edit, offline run and restart mid-update; unchanged output is not redownloaded."),
    ("recurrence-and-wake", "1221 20891 20892 1595 20679 20680 21134 21136", "queue.schedule grabber.schedule", "Daily/weekday and periodic jobs require the application running; busy/editing grabber runs can be skipped; wake timers may be required.", "Persist time zone, recurrence and missed-run policy; distinguish process availability from supported OS wake scheduling.", "DST gap/fold, clock correction, sleep/wake, missed start and simultaneous stop; stub OS wake calls and verify one execution per scheduled occurrence."),
    ("quota-window", "1266 1267 1268 20408 20409", "settings.quota", "A megabyte-per-hour-window budget can warn, stop downloads and automatically resume at the next permitted time.", "Persist aggregate usage and window boundaries across restart; specify payload accounting and resume only tasks stopped by quota.", "Cross limit with two jobs, restart near rollover, advance/backshift clock and manually pause one task; cap accounting and resumption stay correct."),
    ("completion-precedence", "1622 20933 20934 20935 20822 1774", "task.completion app.exit queue.schedule task.progress", "Per-file settings can be unavailable during queue processing; exit warns about other active transfers; queue progress starts minimized.", "Define task/queue/global completion-action precedence, success/failure eligibility and cancelable countdown; never apply an unrelated task's completion action to the whole service.", "One task completes while another runs; failed/canceled queue member, canceled countdown and GUI close; record actions through a fake OS adapter."),
    ("completion-power", "21153 21154 21155 21157 21158 21159 21160 21161 21162 21164 21165 20706 20707", "power.shutdown power.restart task.completion grabber.schedule", "Task, queue and grabber completion can request shutdown/sleep/hibernate; reboot and immediate/countdown controls are exposed.", "Create explicit per-scope policies with a cancelable countdown, platform capabilities and action audit. Test with fake adapters before native Windows/Linux validation.", "Each power mode, denied capability, failed job, other active queue, cancellation and repeated completion event; dispatch at most once without affecting the test host."),
    ("completion-program-and-scanner", "1590 1318 1319 21190 21191 20225", "security.scanner grabber.schedule", "Completion can open a configured file; scanner arguments accept a file token, or append the downloaded path when parameters are absent.", "Use structured process arguments with exact path substitution and explicit configuration; track scanner exit status and avoid shell interpretation.", "Path with spaces/metacharacters, missing executable, malicious-looking filename, nonzero scanner exit and timeout; no unintended process is launched."),
    ("category-retroactive", "20280 20281 1790 1793 1239", "settings.categories settings.paths", "Extension/site rules and last-used paths classify future files; old rows may be reassigned without moving their files.", "Separate task category from physical destination; define deterministic rule precedence and optional retroactive reassignment.", "Overlapping type/site rules, explicit override, deleted category, restart and recategorized completed output; existing files remain at their known paths."),
    ("temporary-storage-cleanup", "20002 20032 20033 20146 20702 21118 21119 21146", "settings.paths task.resume", "Final and temporary volumes have separate capacity/file-size limits; failed publication can retain temp output; unreferenced old parts can be offered for deletion.", "Track durable ownership of checkpoints/partials, distinguish final/temp failures and expose bounded orphan cleanup with retention rules and no silent user-output deletion.", "Disk full during download/publication, cross-volume move, file-size-limited volume, abandoned/live checkpoints and symlinks; only confirmed unowned parts are eligible."),
    ("file-actions", "21125 21126 20738 21193 21194 20749 20750", "task.properties task.completion", "Move/rename, open-with, configurable double-click and completed-file drag are exposed; incomplete open is refused.", "Persist filesystem moves transactionally with path reservations, expose selection-aware actions and handle missing files/permission/cross-volume errors.", "Completed versus partial rows, rename collision, cross-volume failure, missing file and chosen double-click action; database and physical paths stay consistent."),
    ("remove-versus-delete", "1713 20833 20834 20835 20836", "task.remove task.remove_completed", "Removing rows is separate from deleting completed files; incomplete and mixed selections have distinct confirmations.", "Add atomic bulk row removal and an explicit optional completed-output delete policy while retaining correct request replay receipts.", "Mixed active/paused/completed selection, permission failure, duplicate path and submission replay after removal; no active output or unrelated file is removed."),
    ("search-scopes", "1811 1812 1859 21215 21216 34040 34041", "gui.search gui.find_next", "Name/description/source/site/parent/referrer search supports case and whole-string flags, scope validation and Find Next.", "Extend persisted task metadata and indexed/virtualized GUI search; define wrap and selection behavior for F3.", "Mixed case, whole-string, missing metadata, no enabled scopes, add/remove while searching and 10,000 rows; stable result selection and measured latency."),
    ("force-bypass-capture", "20519 1361 1067 1366 20924 1753", "browser.shortcuts", "Force and bypass modifier sets must not conflict; force capture can require a left click and exclude scripts/images/pages.", "Specify browser-supported input capture, transient scope and precedence over extension/type/site preferences; preserve normal navigation on bypass.", "Overlapping modifiers, delayed download, simultaneous page assets, click and keyboard activation; only the intended eligible download transfers ownership."),
    ("miscapture-exceptions", "1651 1652 1653 1659 20823 20824", "browser.exceptions", "Two cancellations or similar mistaken captures can prompt for site/address pattern exclusions, with suppression preference.", "Store independently authored matching rules and per-site cancellation history; distinguish exact URLs, sites and patterns without exposing secrets in reports.", "Second cancellation, navigation reset, pattern matching boundary, enable/disable and bypass; excluded downloads complete in the browser."),
    ("all-links-and-clipboard", "1234 1055 1060 1385 20885 20887 34036", "browser.all_links settings.general", "All-link collection can hide HTML/images/duplicates and use link text descriptions; clipboard supports single and multi-URL prompts.", "Implement explicit bounded collection and preview, preserve page context, validate URLs and make clipboard polling opt-in and resource measured.", "Duplicate relative/absolute URLs, selected text, missing type, navigation, malformed clipboard and over-limit batch; no task is canceled before durable acceptance."),
    ("generated-batch", "1553 1540 1541 1547 1558 1559 1560 20645 20647", "batch.generated", "A URL wildcard expands numeric/alphabetic sequences with width and previews; original catalog describes a 1000-file bound.", "Implement bounded deterministic expansion, validation and preview with explicit independent limits; reuse atomic import rather than treating URL import as generator parity.", "Padding, descending/invalid bounds, Unicode letters, multiple wildcards, duplicates and expansion limit; displayed preview equals committed tasks."),
    ("player-panel-policy", "1717 1718 1720 1762 1788 1789 20863 20928", "media.overlay browser.types browser.exceptions", "Full/mini media panel, site exclusions and protected-content visibility coexist with automatic capture preferences; disabling a panel type can otherwise trigger capture.", "Define a unified precedence table for panel visibility, player auto-capture, media type, site exception and explicit action; create original content-script UI.", "Panel-disabled type, site exclusion, multiple players, nested frames, fullscreen and protected manifest; controls preserve playback and never imply DRM support."),
    ("quality-and-track-catalog", "21129 21130 21005 21006 21203", "media.overlay", "Other quality may appear only after player selection; separate audio/video tracks and subtitle labels are present.", "Combine observed-request qualities with clear-manifest representations and label origin/availability. Add explicit audio-only/video-only/subtitle selection and codec compatibility.", "Player changes resolution, missing audio group, duplicate URLs, multiple languages and subtitles; chosen output has the selected tracks and decodes."),
    ("live-stop-save", "21000 21001", "media.overlay task.resume", "Stopping live recording can finalize captured content, with a warning that the same stream point cannot be resumed.", "Create recording/finalizing/complete states and a stop-and-save command distinct from ordinary resumable pause; define reconnect gaps and output validity.", "Sliding-window HLS/DASH, stop during segment, disconnect, reconnect gap and stream end; finalized capture decodes and no expired segment is promised resumable."),
    ("stream-assembly-fallback", "20898 21106 21192 21219", "media.overlay task.progress", "Media part processing can seek a matching frame; muxing has its own state/errors; incompatible audio may be saved separately.", "Specify segment timeline/discontinuity and mux errors, codec/container selection and atomic publication of separate tracks when combined output is unsupported.", "Discontinuity, overlap, unsupported MP4 codec pair and mux failure; valid output or clearly identified separate tracks, with no silently missing audio."),
    ("minimum-media-size", "21148 21149 21150", "media.minimum", "Separate audio/video panel thresholds only apply when total media size is known; duration-only/unknown-size media remain eligible.", "Filter at representation level using reliable whole-file size, preserve unknown sizes and never substitute manifest/segment length for whole media length.", "Small known audio/video, unknown content length, short manifest with large segments and duration-only candidate; threshold obeys the documented rule."),
    ("proxy-selection", "1823 1824 1825 1826 1796 21169 1843 1844 21188", "settings.proxy", "Manual, auto-detected, system and none modes; PAC, protocol-selective proxy/SOCKS and optional browser-request proxy fallback; browser VPN routes may be unknown.", "Use an explicit resolver with credentials/bypass/DNS scope; validate PAC fetch/parse and avoid pretending browser VPN routes are automatically available.", "HTTP/HTTPS/SOCKS fixtures, PAC failure/auth, bypass, DNS route, WPAD timeout and differing browser route; no credential leakage or unbounded autodetection delay."),
    ("connection-profiles-and-host-caps", "1306 1348 1350 1343 21113 21114 21115 21116", "settings.connections settings.host_connections", "Default profile/connection count and per-server exceptions are exposed; too many connections can reduce speed or cause server restrictions.", "Persist defaults and enforce per-origin budgets shared across tasks; adapt segment count to measured useful throughput and bounded CPU/memory.", "Two tasks to one origin plus another origin, redirects, server 429 and connection stalls; host/global caps are respected and no worker count leaks."),
    ("limiter-precedence", "1704 1705 1706 1708 1765", "speed.task", "Startup limiter default and remembered per-file exceptions/persistence can differ from global enablement.", "Separate global aggregate rate, default per-file rate and task override; document precedence and preserve live updates across restart.", "Global enable/disable plus task override, pause/restart, simultaneous jobs and extremely low rate; measured throughput and cancellation responsiveness stay bounded."),
    ("browser-platform-health", "20950 20951 20906 20907 21105 21196 21197 21225 1865 1866 21233 21234", "browser.integration platform.privilege", "Original health depends on elevation, BFE, drivers/helpers and Windows updates; enabled/disabled/repair/restart states are exposed.", "Provide health diagnostics for the independent extension/native-host/daemon chain; use supported OS APIs rather than copying original driver or system-repair behavior.", "Missing manifest, wrong extension ID, unavailable daemon, mismatched user/privilege, host path change and update; report the failing boundary and preserve browser fallback."),
    ("updater-integrity", "21120 21143 21144 21185", "app.update browser.integration", "Update metadata/signature/download validation and site-rule format compatibility have failure paths.", "Create independently distributed signed releases/rules, schema compatibility and rollback; do not contact proprietary endpoints or modify hosts to bypass local policy.", "Invalid signature, corrupted/truncated update, unsupported rule version, captive-portal HTML and offline check; installed working version remains usable."),
    ("tls-substitution", "1842 21198 21199 21200 21201", "settings.connections", "Experimental original TLS mode changes a global Windows setting and has library-load errors.", "Use supported Rust TLS with certificate/hostname validation and clear errors; keep application TLS configuration local and platform trust behavior explicit.", "Expired/self-signed certificate, hostname mismatch, TLS versions, missing trust store and HTTPS redirect; no silent downgrade or system setting mutation."),
    ("appearance-and-progress", "34042 21205 21206 34045 34046 21218 1775 1768 1769 1773", "gui.appearance task.progress", "System dark mode, font choice/reset and progress tab/start-view settings can require restart in the original.", "Implement independent theme/font/progress preferences, accessible contrast and scaling, preserving window/selection state; decide which changes apply live.", "Light/dark OS changes, large fonts/high DPI, hidden tabs, restored queue windows and restart; no clipping or loss of task controls."),
    ("grabber-auth-and-offline", "1468 1439 1441 1561 1564 1514 1469", "grabber.crawl grabber.destination grabber.project", "Crawler supports manual login, excludes logout patterns, uses relative folders and rewrites links for offline browsing with optional overwrite.", "Design a bounded authenticated crawl context, explicit navigation rules and collision-safe offline mapping; scripts execute only in a separately specified browser mode.", "Login/logout cycle, cross-origin links, relative assets, path collisions, query variants and overwrite disabled; crawl stays within policy and saved links resolve locally."),
    ("grabber-project-recovery", "20624 20625 20626 20656 20657 20658 20677", "grabber.load grabber.save grabber.schedule", "Corrupt/incompatible projects/templates, partial loading and disk-space errors are distinguished; scheduling requires a saved project.", "Use versioned independent schemas, atomic saves and explicit partial recovery; prevent scheduling unsaved edits and clarify project versus task persistence.", "Truncated/old/new schema, missing project assets, disk-full save, concurrently running/editing project and restart; previous valid project is preserved."),
    ("playlist-link-expansion", "20220 20221 20222", "media.overlay browser.all_links", "Downloaded M3U can offer its MP3 links as one download or a batch, separate from live/segmented HLS.", "Classify generic URL playlists independently of HLS manifests, validate relative URLs and preview bounded audio-link expansion.", "One/multiple MP3 URLs, relative links, malformed and mixed playlists, HLS tags and duplicates; avoid treating HLS segments as unrelated MP3 tasks."),
]


def build():
    raw = CATALOG.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == DIGEST, "Catalog changed: re-review index ranges before regeneration"
    entries = json.loads(raw)["entries"]
    ledger = coverage()
    families = {row["id"]: row for row in ledger["families"]}
    families.update({row["id"]: row for row in ledger["non_ui_paths"]})
    rows, sections, contracts = [], [], []
    next_index = 0
    for start, end, targets, original in SECTIONS:
        assert start == next_index and start <= end < len(entries), (start, end, next_index)
        next_index = end + 1
        target_ids = targets.split()
        assert all(key in families for key in target_ids), target_ids
        section_id = f"review-{start:04d}-{end:04d}"
        sections.append({"id": section_id, "catalog_indexes_inclusive": [start, end],
                         "source_keys": [entries[start]["key"], entries[end]["key"]],
                         "target_families": target_ids, "source_semantics": original,
                         "evidence_level": "S: shipped text, manually reviewed",
                         "association_scope": "thematic-section; not a unique runtime caller",
                         "xdm_family_scope": [{"id": key, "state": families[key]["state"],
                                               "behavior_and_remaining_acceptance": families[key]["behavior_and_acceptance"]}
                                              for key in target_ids]})
        for index in range(start, end + 1):
            entry = entries[index]
            rows.append({"source_key": f"L{entry['key']}@{entry['line']}", "catalog_index": index,
                         "catalog_key": entry["key"], "source_line": entry["line"],
                         "semantic_section": section_id, "specific_contracts": [],
                         "caller_state": "not-established-by-semantic-review"})
    assert next_index == len(entries)
    by_key = {}
    for row in rows:
        by_key.setdefault(row["catalog_key"], []).append(row)
    for identity, keys, targets, original, requirement, test in CONTRACTS:
        target_ids = targets.split()
        assert all(key in families for key in target_ids), target_ids
        references = []
        for key in keys.split():
            assert key in by_key, (identity, key)
            for row in by_key[key]:
                row["specific_contracts"].append(identity)
                references.append(row["source_key"])
        contracts.append({"id": identity, "source_references": references, "target_families": target_ids,
                          "original_text_indicates": original, "original_evidence": "S: catalog; triggering runtime path requires validation",
                          "xdm_requirement": requirement, "acceptance_scenario": test,
                          "verification_state": "specified; not asserted passing",
                          "xdm_family_states": {key: families[key]["state"] for key in target_ids}})
    assert len(rows) == len(entries) == len({row["source_key"] for row in rows})
    assert len(contracts) == len({row["id"] for row in contracts})
    return {"schema_version": 1, "catalog_source": str(CATALOG.relative_to(ROOT)), "catalog_sha256": DIGEST,
            "scope": "All catalog texts manually reviewed by semantic section; selected dynamic transitions have explicit contracts. This does not establish exact callers or runtime parity.",
            "counts": {"reviewed_catalog_entries": len(rows), "semantic_sections": len(sections),
                       "detailed_behavior_contracts": len(contracts),
                       "entries_with_detailed_contracts": sum(bool(row["specific_contracts"]) for row in rows),
                       "runtime_callers_established_by_this_review": 0},
            "sections": sections, "contracts": contracts, "references": rows}


def markdown(review):
    n = review["counts"]
    lines = ["# IDM dynamic behavior review", "",
             f"All **{n['reviewed_catalog_entries']} shipped language entries** have been read and associated with **{n['semantic_sections']} reviewed sections**. **{n['detailed_behavior_contracts']} detailed contracts** expand the state transitions and edge cases that dialog names alone miss. The coverage ledger still records 1,136 references without a static-resource association; reading and classifying a message does not recover its runtime caller.", "",
             "These are semantic associations from the supplied catalog (evidence S), including historical entries. They do not prove a feature is reachable in this build, identify a unique caller, or verify an acceptance test. Exact runtime callers remain separate investigation work in the [coverage ledger](feature_coverage.md). The [machine-readable review](idm_behavior_review.json) preserves every language key and source line, section, contract association and current XDM family scope.", "",
             "No original translation text is embedded in the application. Requirements below describe independent behavior. Family implementation states describe the current subset and do not mean each contract passes. Native Windows behavior, live streams, authenticated media and browser-specific paths still need verification.", "",
             "## Detailed behavior contracts", "",
             "| Contract and source keys | Current family states | Original text indicates | Independent XDM requirement | Acceptance scenario (not yet a passing result) |", "|---|---|---|---|---|"]
    for row in review["contracts"]:
        keys = ", ".join(ref.split("@")[0] for ref in row["source_references"])
        states = "; ".join(f"`{key}`: {state}" for key, state in row["xdm_family_states"].items())
        lines.append(f"| `{row['id']}` ({keys}) | {states} | {row['original_text_indicates']} | {row['xdm_requirement']} | {row['acceptance_scenario']} |")
    lines += ["", "## Complete semantic section index", "",
              "Catalog indexes are zero-based and inclusive; language keys are preserved separately. Sections with multiple families remain deliberately ambiguous at caller level. The JSON includes each family's current implementation state and remaining work.", "",
              "| Catalog indexes | Source endpoint keys | Feature families | Reviewed semantics |", "|---|---|---|---|"]
    for row in review["sections"]:
        start, end = row["catalog_indexes_inclusive"]
        lines.append(f"| {start}–{end} | {', '.join(row['source_keys'])} | {', '.join('`'+key+'`' for key in row['target_families'])} | {row['source_semantics']} |")
    lines += ["", "## Verification and boundaries", "",
              "Regenerate with `python3 docs/analysis/build_behavior_review.py`; validate with `python3 docs/analysis/build_behavior_review.py --check`. The generator pins the catalog digest, rejects gaps/overlaps, validates all feature IDs and cited keys, and emits exactly one semantic-section record per catalog entry. These checks establish traceability, not runtime equivalence.", "",
              "Original runtime evidence is in [the bounded audit](idm_audit_completion.md), [browser/media analysis](idm_browser_media_analysis.md), and [native-core analysis](idm_native_core_analysis.md). Current code/test coverage is in [implementation status](implementation_status.md). The next mapping pass must refine control-level actions and runtime callers where ambiguous, rather than treating section associations as solved caller analysis.", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    review = build()
    outputs = {ROOT / "docs/idm_behavior_review.json": json.dumps(review, indent=2, ensure_ascii=False) + "\n",
               ROOT / "docs/idm_behavior_review.md": markdown(review)}
    for path, data in outputs.items():
        if args.check:
            assert path.read_text() == data, f"stale artifact: {path}"
        else:
            path.write_text(data)
    print(json.dumps(review["counts"]))
