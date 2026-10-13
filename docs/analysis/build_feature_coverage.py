"""Map every supplied dialog/control/menu identity to an independently designed feature.

This validates inventory coverage, not runtime equivalence. Unresolved controls remain explicit.
Run normally to generate docs/feature_coverage.{json,md}; --check verifies current artifacts.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "docs/analysis/idm_ui_resources.json"

# id, dialog IDs, feature, implementation state, target component, behavior/remaining acceptance
SPECS = [
    ("app.about", [100], "About and license information", "pending", "desktop", "Show XDM version, GPL license, dependencies and project links."),
    ("gui.shell", [102], "Main shell and category/queue navigation", "partial", "desktop", "Task list and filters exist; full table/tree, menus, multi-selection and keyboard navigation remain."),
    ("task.progress", [103, 363, 367], "Detailed progress and segment view", "partial", "desktop/http", "Live HTTP totals/speed/ETA exist; connection visualization, resume capability and configurable detail tabs remain."),
    ("app.tips", [105, 236], "Tips and integration guidance", "pending", "desktop/help", "Original help and tips must be replaced with independently written guidance and a dismiss/startup preference."),
    ("task.add", [130, 148], "Add URL and download confirmation", "partial", "desktop/service", "URL/path and queue import exist; category/description/auth fields, destination picker, history and download-later confirmation remain."),
    ("task.properties", [135], "Download properties", "partial", "desktop/service", "Details exist; editable destination/description, parent page/referrer, last-attempt metadata and address refresh remain."),
    ("app.commercial", [138], "Commercial registration and purchase", "intentional-substitute", "about/license", "XDM is GPL and has no paid activation. Map the information role to open-source license/project information; do not reproduce IDM license checks."),
    ("settings.general", [139, 231], "General settings and integration", "partial", "desktop/settings", "Browser opt-in exists; startup, clipboard monitoring, browser detection and integration preferences remain. D231 is an additional resource variant."),
    ("settings.proxy", [140, 369], "Proxy, SOCKS and advanced proxy settings", "pending", "network/settings", "Implement explicit none/system/PAC/manual choices, per-protocol credentials/bypass, SOCKS DNS and safe browser proxy context; native platform tests required."),
    ("settings.connections", [141], "Connection defaults", "partial", "http/settings", "Task connections and job limits exist; persistent default profiles and related settings UI remain."),
    ("settings.paths", [143], "Save paths and file dates", "partial", "categories/settings", "Built-in routing exists; remembered/category/temp paths, creation-date policy and destination rules remain."),
    ("app.update", [144, 182, 190, 290], "Updates and optional components", "pending", "packaging/updater", "Provide independent release checks and optional component handling with version/integrity validation; no IDM update endpoints."),
    ("auth.store", [145, 146], "Site/path credential profiles", "pending", "credentials/settings", "Origin/path-scoped credentials, protected storage and explicit edit/remove workflows are required."),
    ("task.confirmation", [147, 300], "Generic task confirmations", "unresolved", "desktop/dialogs", "Resource captions establish choices but not every caller. Trace triggering conditions before specifying yes-all/no-all or continue behavior."),
    ("power.shutdown", [150], "Queue completion shutdown", "pending", "scheduler/platform", "Explicit per-queue success/failure policy, cancelable countdown and OS adapter; fixture tests must never shut down the host."),
    ("platform.dialup", [184, 185, 186], "Dial-up/VPN and redial", "platform-pending", "platform/network", "Windows connection selection, credential handling and reconnect policy require native Windows validation; Linux equivalents need separate capability design."),
    ("gui.drop", [187], "Floating URL drop target", "pending", "desktop/integration", "Original visual design, drag/drop URL validation, show/hide and queue submission."),
    ("lists.export", [191], "Download list export", "partial", "transfer-list", "Portable JSON/text exports exist; selected/queue scope and original-format compatibility remain. JSON is an intentional new format."),
    ("browser.types", [193, 294, 341], "Capture file types", "partial", "extension/settings", "Hard-coded media detection exists; configurable extension types, defaults and site exclusions remain."),
    ("settings.quota", [194, 309], "Usage quota and warning", "pending", "scheduler/settings", "Persist byte budgets/window rules and warning/resume behavior, separately from speed caps."),
    ("settings.sounds", [195], "Event sounds", "pending", "desktop/platform", "Per-event sound selection, preview playback and enable/disable preferences."),
    ("task.archive", [196], "ZIP member preview", "pending", "archive/http", "Bounded range-aware archive inspection and a member list; unsupported/large/malformed archives need explicit handling."),
    ("settings.categories", [204], "Custom categories and rules", "partial", "categories/settings", "Built-in extension/MIME categories exist; create/edit/delete, site patterns, custom folders and user overrides remain."),
    ("gui.language", [208], "Language selection", "pending", "desktop/localization", "Independent translations, persistent locale and runtime switching; no copied IDM translation catalog."),
    ("gui.columns", [209], "Task columns", "pending", "desktop/table", "Show/hide/reorder columns and persist widths/order; a row label is not column parity."),
    ("settings.host_connections", [226], "Per-server connection caps", "pending", "http/settings", "Host pattern limits shared across simultaneous tasks, distinct from per-task segment count."),
    ("browser.all_links", [228], "Capture and select all page links", "pending", "extension/desktop", "Collect eligible links with selection, category/destination rules and durable batch handoff."),
    ("browser.shortcuts", [229], "Force/bypass modifier keys", "pending", "extension/settings", "Per-browser tested key combinations and transient capture override; no accidental default browser cancellation."),
    ("network.check", [230], "Connection diagnostics", "pending", "network/desktop", "Bounded connectivity/integration checks and useful status without leaking authentication."),
    ("app.report", [232, 336], "Problem reports", "pending", "diagnostics/desktop", "Create a redacted, reviewable local report; sending must be an explicit user action."),
    ("grabber.wizard", [239, 241], "Site grabber wizard", "pending", "grabber/desktop", "Independent project wizard with crawl plan, destination, results and start/stop steps."),
    ("grabber.files", [240], "Grabber results and file filters", "pending", "grabber/desktop", "Selection tree/table, duplicate rules, same-site restriction and filters."),
    ("legacy.empty", [242, 504, 30734], "Empty resource templates", "reference-only", "audit", "Retain identities; empty templates do not establish a separately reachable product feature."),
    ("grabber.filters", [244, 245], "Grabber filter editor", "pending", "grabber/settings", "Named wildcard include/exclude masks with add/edit/delete and validation."),
    ("grabber.project", [257], "Grabber project creation", "pending", "grabber/store", "Project name, seed URL, templates and durable project state."),
    ("grabber.destination", [259], "Grabber destination policy", "pending", "grabber/categories", "Category folder/single-folder rules, offline path mapping and conflict handling."),
    ("grabber.settings", [262], "Grabber concurrency and cache settings", "pending", "grabber/settings", "Separate explore/download budgets; replace legacy IE cache behavior with a supported cache policy."),
    ("grabber.crawl", [264], "Crawl scope and depth", "pending", "grabber/engine", "Depth, base-site/outside-site limits, navigation filters, robots/rate policy and cycle bounds."),
    ("grabber.templates", [268], "Grabber templates", "pending", "grabber/store", "Named validated templates and independent settings schema."),
    ("grabber.save", [269], "Save grabber projects", "pending", "grabber/store", "Atomic project save, name conflicts and optional prompt preference."),
    ("grabber.load", [270], "Load grabber projects", "pending", "grabber/store", "Project listing, schema migration and damaged-project recovery."),
    ("grabber.statistics", [271], "Grabber statistics", "pending", "grabber/events", "Processed pages/files, discovered links, filtered/skipped/error counts and progress events."),
    ("auth.challenge", [279], "HTTP authentication challenge", "pending", "credentials/network", "Realm/origin prompt, protected credentials, cancellation and retry; do not blindly forward credentials across origins."),
    ("batch.generated", [280], "Generated URL batches", "pending", "transfer-list/desktop", "Numeric/alphabetic sequence ranges, padding, preview and bounded expansion. URL-list import is not generated-batch parity."),
    ("grabber.cleanup", [281], "Grabber temporary-file cleanup", "pending", "grabber/storage", "Cancelable cleanup progress and interrupted-project recovery without deleting user outputs."),
    ("grabber.schedule", [284], "Grabber project schedule", "pending", "grabber/scheduler", "One-shot/recurring crawl/download phases, time limits and completion rules."),
    ("task.completion", [286, 364], "Completion dialog and actions", "pending", "desktop/platform", "Open/open-with/folder, dialog preference, destination action, app exit and explicit power policy."),
    ("settings.downloads", [291], "Download defaults", "partial", "desktop/settings", "Explicit queue/paused import exists; start/complete prompts, eager transfer, duplicate defaults, User-Agent and completion options remain."),
    ("power.restart", [292], "Queue completion restart", "pending", "scheduler/platform", "Opt-in completion policy and cancelable restart countdown; native Windows/Linux tests required."),
    ("browser.integration", [296, 297, 334, 381, 489], "Chromium-family integration", "partial", "extension/native-host", "Linux Chrome for Testing handoff/fallback passes; installed Chrome/Edge/Brave profiles and native Windows registration/GUI validation remain."),
    ("task.duplicate", [298], "Duplicate download choices", "partial", "service/desktop", "Conflicts reject safely and explicit fresh-destination redownload exists; numbered duplicate/overwrite/resume-complete dialog policies remain."),
    ("browser.exceptions", [302, 303, 304], "Capture address/site exceptions", "pending", "extension/settings", "Persistent site/address masks, cancellation prompts and once-only fallback behavior."),
    ("browser.fallback", [305], "Handoff failure and browser fallback", "verified-subset", "extension/native-host", "Real Linux acceptance/cancel and daemon-unavailable browser completion pass; single-use GET/POST, HTML miscapture, other platforms and disconnect/late-ack races remain."),
    ("queue.schedule", [306, 332], "Queue schedule", "partial", "daemon/scheduler", "Persistent global one-shot start/stop exists; per-queue times, recurring weekday/daily, time zones and missed-run policy remain."),
    ("queue.files", [307], "Queue membership, order and concurrency", "partial", "daemon/desktop", "Atomic stopped-task assignment, priority/FIFO and per-queue caps exist; manual ordering and sync retention remain."),
    ("queue.manage", [308, 310], "Named queue management", "partial", "daemon/desktop", "Create/rename/empty-delete/start/stop/limits and GUI filtering pass local tests; enabling a queue does not open the global gate, so independent original queue start semantics remain a parity gap. Stopping a queue interrupts only its workers."),
    ("browser.firefox", [335], "Firefox browser adapter", "pending", "extension/firefox", "Independent event adapter, host registration and real browser tests; Chromium hooks cannot be assumed equivalent."),
    ("speed.task", [337, 365], "Speed limiter", "partial", "http/desktop", "Persisted live HTTP task caps exist; application startup defaults, media caps and limiter-tab customization remain."),
    ("task.remove", [338, 339], "Task and optional file deletion", "partial", "service/desktop", "Stopped-task removal preserves outputs/checkpoints and replay receipts; optional completed-file deletion and multi-task delete remain."),
    ("media.overlay", [340, 370], "Floating player panel", "pending", "extension/content", "Full/mini player overlay, position/visibility, site exclusions and protected-content notification; toolbar popup is not overlay parity."),
    ("platform.privilege", [342], "Privilege/integration compatibility", "platform-pending", "platform/native-host", "Per-user IPC exists; Windows integrity levels, broker behavior and browser integration checks need native validation."),
    ("task.wait", [344], "Stop waiting and reconnect UI", "partial", "http/desktop", "Bounded retries exist; user-facing stop-waiting state and interrupted-read responsiveness need further tests."),
    ("queue.assignment", [345], "Deferred tasks and queue choice", "partial", "daemon/desktop", "Import paused/enqueue and queue assignment exist; per-download later/start queue prompt and remembered choices remain."),
    ("browser.selection_panel", [348, 349], "Selected-link download panel", "pending", "extension/content", "Selected URL detection, full/mini panel and always/never/site-specific preferences."),
    ("app.exit", [352], "Exit and state saving", "partial", "daemon/desktop", "SQLite persists commands and GUI can close independently; explicit whole-app stop/exit policy and graceful daemon shutdown remain."),
    ("gui.search", [487, 505, 506], "Find file", "partial", "desktop/model", "Case-insensitive URL/path/status/category search exists; description/page/referrer scopes, case/whole-string and find-next remain."),
    ("media.minimum", [488], "Minimum detected media size", "pending", "extension/settings", "Configurable minimum size and unavailable-size behavior; do not discard manifests by segment size."),
    ("security.scanner", [494], "Antivirus hook", "pending", "completion/platform", "Explicit scanner executable/arguments, completion timing and result/error handling."),
    ("legacy.generic", [30721], "Generic native New template", "unresolved", "audit", "Retain this framework-looking resource; caller and reachable product behavior have not been established."),
    ("lists.import", [], "URL and task list import", "verified-subset", "transfer-list/daemon", "Preview, safe names, atomic paused/queued import and restart receipts pass; source proprietary format compatibility remains separate."),
    ("task.resume", [], "Start/resume task", "partial", "http/service", "HTTP pause/resume passes; manual Download Now bypass/preemption semantics and expired URL refresh remain."),
    ("task.redownload", [], "Redownload task", "partial", "service/desktop", "A new task at a fresh path inherits configuration; in-place overwrite/restart choice remains."),
    ("queue.master", [], "Pause/stop all", "partial", "daemon", "Global stop checkpoints and requeues active tasks; distinct pause-all versus stop-all behavior needs original-product comparison."),
    ("task.remove_completed", [], "Remove completed tasks", "pending", "service/desktop", "Bulk completed-row removal and separate optional file deletion."),
    ("gui.find_next", [], "Find next", "pending", "desktop/model", "Cycle search results with F3, wrapping and selection preservation."),
    ("settings.options", [], "Settings entrypoint", "partial", "desktop/settings", "Secondary controls exist; complete structured settings, validation and persistent defaults remain."),
    ("gui.category_visibility", [], "Category visibility", "pending", "desktop/settings", "Show/hide category navigation independently of the task table and persist preference."),
    ("gui.sort", [], "Task sorting", "pending", "desktop/model", "Sort by addition/name/size/status/ETA/rate/last attempt/description/path/referrer/page with stable tie handling."),
    ("gui.toolbar", [], "Toolbar customization", "pending", "desktop/settings", "Button order/visibility, size and original XDM themes; no IDM toolbar assets."),
    ("gui.tray", [], "Tray icon", "pending", "desktop/platform", "Show/hide, original icon choices, task/queue menu and background lifecycle on Linux/Windows."),
    ("gui.appearance", [], "Dark mode and fonts", "pending", "desktop/settings", "Persistent light/dark/system theme, font selection/reset and accessible contrast."),
    ("app.help", [], "Help and project links", "pending", "desktop/help", "Independent help/tutorials/queue/grabber chapters and project/support links."),
    ("grabber.recent", [], "Recent grabber projects", "pending", "grabber/store", "Bounded recent-project list and missing-project handling."),
    ("gui.menu_container", [], "Menu containers and separators", "pending", "desktop/menu", "A native menu bar with independently labeled commands; source hierarchy is evidence, not copied product assets."),
]

MENU_GROUPS = {
    "task.add": [32771], "batch.generated": [34024], "browser.all_links": [34036],
    "grabber.wizard": [33010], "gui.drop": [32809], "lists.export": [32821,32823],
    "lists.import": [32816,32818], "app.exit": [32772], "task.resume": [32774,32776],
    "task.remove": [32775], "task.redownload": [33009], "queue.master": [32778,32779],
    "task.remove_completed": [32794], "gui.search": [34040], "gui.find_next": [34041],
    "queue.schedule": [32801], "queue.manage": [34026,34028], "speed.task": [34033,34034,34035],
    "settings.options": [32781], "gui.category_visibility": [32825],
    "gui.sort": [32828,32830,32832,32834,32836,32838,33006,32840,33004,33005,34043],
    "gui.toolbar": [33001,34011,34010,34012,33510], "gui.tray": [34037,34038,34039],
    "gui.columns": [33002], "gui.appearance": [34042,34045,34046], "gui.language": [32841],
    "app.help": [57670,32811,34029,34022,32813,32784,32788], "app.tips": [106],
    "app.update": [32787], "app.about": [32786], "app.commercial": [32785,32783],
    "grabber.project": [1458], "grabber.load": [1438], "grabber.save": [1442],
    "grabber.templates": [34020], "grabber.recent": [34013,34014,34015,34016],
    "grabber.cleanup": [1079], "grabber.settings": [1457], "grabber.schedule": [34021],
}

# Functional paths that cannot be inferred from desktop templates alone.
CORE = [
    ("http.probe", "HEAD/GET and actual Range probing", "verified-subset", "CLI fixtures verify real byte ranges and safe single-stream fallback; unusual compressed/redirected/unknown-size cases still need coverage."),
    ("http.partition", "Parallel range boundaries and response validation", "verified-subset", "Unit split/range parsing plus hash fixtures pass. Server validators, identity encoding and declared byte bounds are checked."),
    ("http.dynamic", "Dynamic split and stalled-worker redistribution", "pending", "Lease-safe reassignment of the largest remaining segment, minimum split size and stall cancellation."),
    ("http.recovery", "Checkpoint and crash recovery", "partial", "Segmented HTTP pause/resume and changed-validator refusal pass. The current single-stream path (including connections=1 on range-capable servers) restarts at zero and has no resume checkpoint; disk-full, forced crash, >4 GiB and filesystem edge cases remain."),
    ("http.retry", "Retry and error classification", "partial", "Transient retries and 403/malformed ranges pass; Retry-After/jitter, expired URL workflows and user-driven reconnect remain."),
    ("http.storage", "Sparse allocation, offset I/O and publication", "partial", "Pre-sized shared partial files and direct offset writes exist; native Windows sparse semantics and no-clobber completion races need more validation."),
    ("http.auth", "Headers, cookies and origin-scoped redirects", "partial", "Direct browser/CLI headers exist; cross-origin authenticated redirects stop. Protected credential storage and authenticated media replay remain."),
    ("http.protocols", "Protocol and proxy coverage", "partial", "HTTP(S) is implemented; FTP/PAC/SOCKS/profile fidelity from shipped settings requires independent work and native tests."),
    ("http.global_cap", "Aggregate bandwidth cap", "pending", "Requested XDM feature; distinguish aggregate global/host caps from IDM's observed per-file limiter."),
    ("ipc.native", "Native Messaging framing and daemon bridge", "verified-subset", "Partial framing/size tests, per-user local IPC and real Linux browser host handoff pass; native Windows ACL/integration validation remains."),
    ("browser.intercept", "Browser handoff transaction", "partial", "Durable registration before browser cancel and bounded fallback exist; late acceptance/timeout ownership reconciliation remains."),
    ("browser.context", "Link and media context menus", "partial", "Direct link/media actions exist; selected-link batches, modifiers and browser download-list context surfaces remain."),
    ("media.discovery", "DOM and network media discovery", "partial", "HTTP media elements, source children, MIME and URL candidates exist; player overlay, navigation lifecycle and all site variants remain."),
    ("media.body_capture", "Page-context XHR/fetch and manifest-body capture", "pending", "Original extension producers were mapped; fresh bounded page-context body/chunk/hash protocol is not implemented."),
    ("media.catalog", "Available representation and audio catalog", "partial", "Public clear FFprobe tracks and popup selection work; compatibility/language/subtitle/audio-group details need broader fixtures."),
    ("media.hls", "Clear HLS selection and assembly", "partial", "High/low video plus separate audio fixtures decode; authentication, live capture, discontinuities and expiring URLs remain."),
    ("media.dash", "Clear DASH selection and assembly", "partial", "Selected clear video/audio MP4 fixtures decode; additional profiles, subtitles and authenticated segments remain."),
    ("media.containers", "Mux and output format choices", "partial", "FFmpeg stream-copy MP4 exists when codecs fit; MKV/WebM choices and explicit conversion remain."),
    ("media.audio", "Audio-only and MP3 extraction", "pending", "Explicit audio-only track workflow and optional MP3 transcode; MP4/MP3 are not interchangeable without conversion."),
    ("media.protection", "Protected-content handling", "verified-subset", "Encrypted HLS and DASH ContentProtection refusal pass. DRM decryption is outside the requested clean-room scope; detection/notification fidelity remains."),
    ("queue.sync", "Synchronization queues", "pending", "Retained source tasks, validator recheck intervals, changed-object replacement and sync scheduling."),
    ("scheduler.recurring", "Daily/weekday and periodic schedules", "pending", "Time-zone/DST, clock changes, missed-run policy, duration/quota cutoffs and durable execution IDs."),
    ("scheduler.actions", "Completion program, sleep, exit and power actions", "pending", "Explicit per-queue policy, cancelable countdown, process launching and platform capability tests."),
    ("platform.install", "Browser host registration and packaging", "partial", "Linux/Windows registration scripts exist; native Windows browser profiles, installer, signing/update and file associations remain."),
    ("platform.legacy", "Windows monitor drivers and legacy IE helpers", "intentional-substitute", "No proprietary driver clone. Map download handoff to supported browser/native-host APIs; legacy-only compatibility is explicitly unverified."),
    ("performance.native", "CPU/RAM and native toolkit benchmark", "pending", "Bounded buffers and separate GUI/daemon exist; release PSS/private-byte, wakeup, 10k-row and Qt-versus-Slint measurements remain."),
]


def caption(value):
    if isinstance(value, str):
        return " ".join(value.replace("&", "").split())[:160]
    return f"resource:{value.get('ordinal')}" if isinstance(value, dict) else ""


def role(control):
    cls = control["class"]
    if isinstance(cls, dict):
        cls = {128:"Button",129:"Edit",130:"Static",131:"ListBox",132:"ScrollBar",133:"ComboBox"}.get(cls.get("ordinal"), str(cls))
    if cls == "Button":
        kind = control["style"] & 15
        return cls, "group" if kind == 7 else "choice" if kind in [2,3,4,5,6,9] else "action"
    if cls == "Static":
        return cls, "interactive-display" if control["style"] & 0x100 else "display"
    if cls in ["Edit", "ComboBox", "msctls_trackbar32", "msctls_updown32", "SysDateTimePick32"]:
        return cls, "input"
    if cls in ["ListBox", "SysListView32", "SysTreeView32", "SysTabControl32"]:
        return cls, "collection"
    if cls in ["ScrollBar", "msctls_progress32"]:
        return cls, "scroll-or-progress"
    return str(cls), "unresolved-native-control"


def build():
    raw = SOURCE.read_bytes()
    source = json.loads(raw)
    families = {key: {"id":key,"title":title,"state":state,"target":target,"behavior_and_acceptance":behavior,"dialog_ids":ids}
                for key,ids,title,state,target,behavior in SPECS}
    dialogs = {}
    for key, ids, *_ in SPECS:
        for identity in ids:
            assert identity not in dialogs, f"duplicate dialog mapping D{identity}"
            dialogs[identity] = key
    source_ids = {dialog["resource_id"] for dialog in source["dialogs"]}
    assert source_ids == set(dialogs), f"dialog coverage mismatch: {source_ids ^ set(dialogs)}"
    menu_groups = {}
    for key, commands in MENU_GROUPS.items():
        assert key in families
        for command in commands:
            assert command not in menu_groups, command
            menu_groups[command] = key
    templates = []
    controls = []
    for dialog in source["dialogs"]:
        identity = dialog["resource_id"]
        group = dialogs[identity]
        templates.append({"resource_id":identity,"source_caption":caption(dialog["title"]),"feature":group,"state":families[group]["state"],"controls":len(dialog["controls"])})
        for index, control in enumerate(dialog["controls"]):
            cls, ui_role = role(control)
            text = caption(control["title"])
            behavior = "source-caption-reference" if text and not text.startswith("resource:") else "unresolved-field-or-resource-reference"
            controls.append({"source_key":f"D{identity}/C{index}","dialog_id":identity,"index":index,
                             "control_id":control["id"],"class":cls,"role":ui_role,"caption_reference":text,
                             "feature":group,"mapping_evidence":behavior,
                             "implementation_state":"reference-only" if ui_role in ["display","group","scroll-or-progress"] else "requires-control-level-verification"})
    menu_nodes = []
    commands = []
    def walk(menu_id, entries, parents, index_path):
        for index, entry in enumerate(entries):
            indexes = index_path + [index]
            text = caption(entry["text"])
            path = parents + [text]
            command = entry.get("id")
            kind = "submenu" if entry.get("children") else "command" if command else "separator"
            feature = menu_groups[command] if command else "gui.menu_container"
            node = {"source_key":f"M{menu_id}/"+".".join(map(str,indexes)),"menu_id":menu_id,"command_id":command,
                    "role":kind,"path_reference":path,"feature":feature,"state":families[feature]["state"]}
            menu_nodes.append(node)
            if kind == "command": commands.append(node)
            if entry.get("children"): walk(menu_id, entry["children"], path, indexes)
    for menu in source["menus"]: walk(menu["resource_id"], menu["entries"], [], [])
    assert {node["command_id"] for node in commands} == set(menu_groups), "menu coverage mismatch"
    assert len(controls) == sum(len(dialog["controls"]) for dialog in source["dialogs"])
    assert len({row["source_key"] for row in controls}) == len(controls)
    # Preserve dynamic labels/messages without pretending a text match establishes its caller.
    language_path = ROOT / "docs/analysis/idm_language_catalog.json"
    language_raw = language_path.read_bytes()
    language = json.loads(language_raw)
    caption_features = {}
    control_features = {}
    for row in controls:
        if row["caption_reference"]:
            caption_features.setdefault(row["caption_reference"].lower(), set()).add(row["feature"])
        control_features.setdefault(str(row["control_id"]), set()).add(row["feature"])
    for row in commands:
        caption_features.setdefault(row["path_reference"][-1].lower(), set()).add(row["feature"])
    def string_reference(identity, text):
        matches = set(caption_features.get(caption(text).lower(), []))
        command = identity.lstrip("t")
        if command.isdigit() and int(command) in menu_groups:
            matches.add(menu_groups[int(command)])
        # An input ID alone is not unique across dialogs: retain all candidates and ambiguity.
        candidates = sorted(control_features.get(identity, []))
        return {"caption_reference":caption(text),"caption_or_command_features":sorted(matches),
                "control_id_candidate_features":candidates,
                "behavior_state":"requires-caller-verification" if matches or candidates else "unresolved-dynamic-reference"}
    dynamic = [{"source_key":f"L{entry['key']}@{entry['line']}","catalog_key":entry["key"],"source_line":entry["line"],
                **string_reference(entry["key"],entry["text"])} for entry in language["entries"]]
    pe_strings = [{"source_key":f"S{entry['string_id']}@{entry['language']}","string_id":entry["string_id"],
                   **string_reference(str(entry["string_id"]),entry["text"])} for entry in source["strings"]]
    assert len({row["source_key"] for row in dynamic}) == len(dynamic)
    return {"schema_version":1,"source":"docs/analysis/idm_ui_resources.json","source_sha256":hashlib.sha256(raw).hexdigest(),
            "language_source":"docs/analysis/idm_language_catalog.json","language_source_sha256":hashlib.sha256(language_raw).hexdigest(),
            "scope":"Complete identity mapping of the supplied parsed UI resources, plus audited non-UI functional paths. Mapping is not implementation or runtime equivalence.",
            "counts":{"dialogs":len(templates),"controls":len(controls),"menu_trees":len(source["menus"]),"menu_nodes":len(menu_nodes),"menu_commands":len(commands),"feature_families":len(families),"non_ui_paths":len(CORE),
                      "language_entries":len(dynamic),"pe_string_entries":len(pe_strings),
                      "unresolved_dynamic_references":sum(row["behavior_state"]=="unresolved-dynamic-reference" for row in dynamic+pe_strings)},
            "families":list(families.values()),"dialogs":templates,"controls":controls,"menus":menu_nodes,
            "language_references":dynamic,"pe_string_references":pe_strings,
            "non_ui_paths":[{"id":key,"title":title,"state":state,"behavior_and_acceptance":behavior} for key,title,state,behavior in CORE]}


def markdown(ledger):
    counts = ledger["counts"]
    lines = ["# IDM feature-to-XDM coverage ledger", "",
             f"Mapped identities: **{counts['dialogs']} dialog templates, {counts['controls']} controls, {counts['menu_trees']} menu trees, {counts['menu_commands']} menu command occurrences**. Every menu container/separator is also retained. There are {counts['feature_families']} target feature families and {counts['non_ui_paths']} additional non-UI functional paths.", "",
             "This is complete coverage of the supplied parsed resource inventory, not a claim that every hidden IDM behavior is known or that XDM has reached parity. The machine-readable [ledger](feature_coverage.json) includes one source key for every control and menu node. IDs disambiguate repeated captions and generic controls. Empty/framework templates, unknown fields/callers, and native Windows behavior remain explicit rather than being labeled implemented.", "",
             "`verified-subset` means local behavior has evidence; `partial` lists remaining work; `pending` has no implementation; `unresolved` requires more behavior evidence; `platform-pending` requires native platform work; `intentional-substitute` identifies proprietary/commercial/legacy surfaces replaced by an independent open-source design. A family's state does not verify each child control. Individual functional controls still need control-level checks; static/display references are recorded as UI evidence.", "",
             "Original assets/code are investigation inputs only. XDM has original source and UI assets. No DRM decryption is planned. Native Windows runtime validation and real browser/media edge cases remain necessary.", "",
             f"Dynamic text coverage: all {counts['language_entries']} catalog entries and {counts['pe_string_entries']} PE string entries have stable reference keys. Caption/command matches and non-unique control-ID candidates are retained as associations, not proof of the triggering behavior. **{counts['unresolved_dynamic_references']} dynamic references have no such resource association** and remain explicit investigation work. This prevents the static dialog count from hiding dynamically assembled functionality.", "",
             "## Dialog families", "", "| Original dialog(s) | XDM feature | State | Behavior and remaining acceptance |", "|---|---|---|---|"]
    for family in ledger["families"]:
        if family["dialog_ids"]:
            lines.append(f"| {', '.join('D'+str(i) for i in family['dialog_ids'])} | `{family['id']}` — {family['title']} | {family['state']} | {family['behavior_and_acceptance']} |")
    lines += ["", "## Every static menu command", "", "| Source | Original command path | XDM feature | State |", "|---|---|---|---|"]
    for row in ledger["menus"]:
        if row["role"] == "command":
            lines.append(f"| M{row['menu_id']}/{row['command_id']} | {' → '.join(row['path_reference']).replace('|','/')} | `{row['feature']}` | {row['state']} |")
    lines += ["", "## Non-UI behavior", "", "| Feature | State | Behavior and remaining acceptance |", "|---|---|---|"]
    for row in ledger["non_ui_paths"]:
        lines.append(f"| `{row['id']}` — {row['title']} | {row['state']} | {row['behavior_and_acceptance']} |")
    lines += ["", "## Evidence and maintenance", "", "The [completed bounded audit](idm_audit_completion.md), [dialog catalog](analysis/idm_dialog_catalog.md), [browser/media audit](idm_browser_media_analysis.md), and [native-core audit](idm_native_core_analysis.md) support the mapping. [Implementation status](implementation_status.md) and the tests under `tests/integration` support the current XDM subset.", "",
              "All language entries have also received a [semantic review](idm_behavior_review.md), with detailed dynamic state-transition contracts. Semantic association does not establish the original runtime caller, so the unresolved association count above is retained.", "",
              "Regenerate with `python3 docs/analysis/build_feature_coverage.py`; validate coverage and checked-in outputs with `python3 docs/analysis/build_feature_coverage.py --check`. The generator rejects unmapped dialog/menu command identities and checks that every source control has exactly one stable mapping key. New implementation evidence must update the family/path state and acceptance text, not just the coverage count.", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    ledger = build()
    outputs = {ROOT/"docs/feature_coverage.json":json.dumps(ledger,indent=2,ensure_ascii=False)+"\n",
               ROOT/"docs/feature_coverage.md":markdown(ledger)}
    for path, data in outputs.items():
        if args.check:
            assert path.read_text() == data, f"stale artifact: {path}"
        else: path.write_text(data)
    print(json.dumps(ledger["counts"]))
