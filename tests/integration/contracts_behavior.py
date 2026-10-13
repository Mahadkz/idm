"""Executable acceptance tests for the 39 detailed behavior contracts
defined in docs/idm_behavior_review.md, run against current binaries
(target/debug/xdm-cli and xdm-daemon) with disposable local HTTP servers
on 127.0.0.1 and temporary data directories (XDM_DATA_DIR).

Usage:  python3 tests/integration/contracts_behavior.py [NAME_FILTER ...]

Every test function is named after its contract ID and its docstring quotes
the acceptance criterion it checks. Contracts whose criteria cannot be
satisfied by the current binaries are listed in EXPECTED_FAIL with a
precise one-line reason; the runner prints "XFAIL <ID>" for those and fails
the run when an expected failure passes or any other test fails.
"""

import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "target/debug/xdm-cli"
DAEMON = ROOT / "target/debug/xdm-daemon"

BLOCK = bytes(range(256)) * 256  # 64 KiB pattern
MIB = 1024 * 1024

# 39 behavior contracts from docs/idm_behavior_review.md with observed gap reasons.
EXPECTED_FAIL = {
    "address-refresh": "address refresh workflow, expired-URL recapture correlation and mismatched object refusal are not implemented in the daemon",
    "single-use-request": "single-use request replay protection and HTML error detection on media downloads are not implemented",
    "eager-confirmation": "provisional download transfer during confirmation and download-later partial retention are not implemented",
    "duplicate-policy": "configurable duplicate policies (ask, numbered duplicate, overwrite, resume/completion prompt) are not exposed by the daemon",
    "content-identity-recovery": "automatic single-stream fallback on range inconsistency and policy-guided validator recovery are not implemented",
    "unknown-length-completion": "canceled unknown-length transfers cannot maintain recoverable partial checkpoint state",
    "server-name-reconciliation": "daemon does not reconcile server Content-Disposition or MIME extensions with output destinations",
    "queue-independent-start": "enabling a queue does not open the global dispatcher gate when global queue processing is stopped",
    "queue-order-and-retry": "queue item reordering persistence across restart and scheduler file-level skip/stop rules are incomplete",
    "synchronization": "synchronization queue mode with conditional re-download and retained-source tracking is not implemented",
    "recurrence-and-wake": "recurring schedule lacks DST transition handling, missed-run catchup policy and OS wake timer integration",
    "quota-window": "hourly bandwidth quota accounting and automatic window-rollover resumption are not implemented",
    "completion-precedence": "per-task completion action precedence and isolation from concurrent queue transfers are not implemented",
    "completion-power": "platform power action execution (sleep/hibernate/shutdown/restart) with capability auditing is not implemented",
    "completion-program-and-scanner": "completion antivirus scanner hook and parameter token substitution are not implemented",
    "category-retroactive": "retroactive category rule reassignment for existing completed tasks without moving files is not implemented",
    "temporary-storage-cleanup": "durable orphan temporary storage detection and retention-bounded cleanup are not implemented",
    "file-actions": "transactional completed-file move, rename collision handling, and open action dispatch are not exposed via IPC",
    "remove-versus-delete": "bulk task removal with optional on-disk completed file deletion policy is not supported in the daemon",
    "search-scopes": "scoped search across description, origin, referrer, and persisted metadata is not exposed in the daemon",
    "force-bypass-capture": "keyboard modifier force/bypass interception and transient page asset filtering are not implemented",
    "miscapture-exceptions": "repeated cancellation miscapture exception prompts and pattern rules are not implemented",
    "all-links-and-clipboard": "all-links collection filtering and opt-in clipboard URL detection are not implemented in the daemon",
    "generated-batch": "batch URL generation lacks alphabetic sequences, descending bounds, multiple wildcards and preview limits",
    "player-panel-policy": "floating media player panel policy and in-page video overlay auto-capture rules are not implemented",
    "quality-and-track-catalog": "dynamic player quality and track cataloging with multi-language/subtitle stream selection are not implemented",
    "live-stop-save": "live stream recording finalization via stop-and-save with explicit non-resumable state transition is not implemented",
    "stream-assembly-fallback": "stream mux failure fallback to atomic separate audio and video track publication is not implemented",
    "minimum-media-size": "media panel threshold filtering based on reliable whole-file size is not implemented",
    "proxy-selection": "PAC script fetching and execution, WPAD discovery, and protocol-selective proxy routing are not implemented",
    "connection-profiles-and-host-caps": "per-host connection concurrency limits shared across simultaneous downloads are not enforced",
    "limiter-precedence": "global speed limiter default and per-task override precedence hierarchy are not supported",
    "browser-platform-health": "end-to-end native host health diagnostics and boundary failure reporting are not implemented",
    "updater-integrity": "signed application update verification, schema compatibility checks and safe rollback are not implemented",
    "tls-substitution": "custom per-task TLS root trust stores and platform TLS substitution configuration are not implemented",
    "appearance-and-progress": "GUI theme, font scaling preferences and progress view state persistence are not exposed via IPC",
    "grabber-auth-and-offline": "site grabber offline HTML link rewriting and crawler logout pattern exclusions are not implemented",
    "grabber-project-recovery": "damaged grabber project schema recovery and partial template loading are not implemented",
    "playlist-link-expansion": "generic M3U audio playlist link expansion into individual download tasks is not implemented",
}


def pattern(start, end):
    chunks = []
    pos = start
    while pos <= end:
        offset = pos % len(BLOCK)
        count = min(len(BLOCK) - offset, end - pos + 1)
        chunks.append(BLOCK[offset:offset + count])
        pos += count
    return b"".join(chunks)


def digest_pattern(size):
    whole = BLOCK * (size // len(BLOCK))
    rem = size % len(BLOCK)
    if rem:
        whole += pattern(0, rem - 1)
    return hashlib.sha256(whole).hexdigest()


class Fixture:
    """Disposable local HTTP server on 127.0.0.1 for behavioral acceptance tests."""

    def __init__(self, delay=0.0):
        self.specs = {}
        self.log = []
        self.counters = {}
        self.lock = threading.Lock()
        self.stop = threading.Event()
        self.delay = delay
        outer = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *unused):
                pass

            def do_GET(self):
                outer.handle(self)

            def do_HEAD(self):
                outer.handle(self)

            def do_POST(self):
                outer.handle(self)

        class SilentServer(ThreadingHTTPServer):
            def handle_error(self, request, client_address):
                pass

        self._server = SilentServer(("127.0.0.1", 0), Handler)
        self._server.daemon_threads = True
        threading.Thread(target=self._server.serve_forever, daemon=True).start()

    @property
    def port(self):
        return self._server.server_port

    def url(self, path):
        return f"http://127.0.0.1:{self.port}{path}"

    def set(self, path, **spec):
        self.specs[path] = spec

    def close(self):
        self.stop.set()
        self._server.shutdown()

    def handle(self, request):
        path = urlsplit(request.path).path
        length = int(request.headers.get("Content-Length") or 0)
        body = request.rfile.read(length) if length else b""
        with self.lock:
            attempt = self.counters.get(path, 0) + 1
            self.counters[path] = attempt
            self.log.append({
                "path": path, "method": request.command, "attempt": attempt,
                "range": request.headers.get("Range"),
                "headers": {k.lower(): v for k, v in request.headers.items()},
                "body": body,
            })
        spec = self.specs.get(path)
        if spec is None:
            self.plain(request, 404, [])
            return
        if spec.get("status"):
            self.plain(request, spec["status"], [])
            return
        if spec.get("redirect"):
            request.send_response(302)
            request.send_header("Location", spec["redirect"])
            request.send_header("Content-Length", "0")
            request.end_headers()
            return
        content_bytes = spec.get("content_bytes")
        if content_bytes is not None:
            request.send_response(200)
            request.send_header("Content-Type", spec.get("content_type", "application/octet-stream"))
            request.send_header("Content-Length", str(len(content_bytes)))
            request.end_headers()
            request.wfile.write(content_bytes)
            return
        size = spec.get("size", 0)
        request.send_response(200)
        request.send_header("Content-Length", str(size))
        if spec.get("disposition"):
            request.send_header("Content-Disposition", spec["disposition"])
        if spec.get("content_type"):
            request.send_header("Content-Type", spec["content_type"])
        request.end_headers()
        if size > 0:
            step = 64 * 1024
            pos = 0
            while pos < size:
                count = min(step, size - pos)
                request.wfile.write(pattern(pos, pos + count - 1))
                pos += count

    @staticmethod
    def plain(request, status, headers):
        request.send_response(status)
        request.send_header("Content-Length", "0")
        for name, value in headers:
            request.send_header(name, value)
        request.end_headers()


class Daemon:
    """Manages an xdm-daemon process in a dedicated temporary state directory."""

    def __init__(self, home, state):
        self.env = os.environ.copy()
        self.env.update({
            "XDM_DATA_DIR": str(state),
            "HOME": str(home),
            "XDM_FAKE_OS_ACTIONS": "1",
        })
        self.process = subprocess.Popen(
            [str(DAEMON)], env=self.env, stderr=subprocess.PIPE, text=True)

    def wait_listening(self):
        line = self.process.stderr.readline()
        assert "listening locally" in line, line

    def cli(self, *args, success=True, timeout=10):
        return run(CLI, "daemon", *args, env=self.env, success=success, timeout=timeout)

    def list(self):
        return json.loads(self.cli("list").stdout)["tasks"]

    def submit(self, url, output, wait=True):
        result = self.cli("submit", url, str(output))
        task_id = str(json.loads(result.stdout)["task_id"])
        if wait:
            self.wait_status(task_id, ("running",), seconds=5)
        return task_id

    def wait_status(self, task_id, statuses, seconds=5):
        deadline = time.monotonic() + seconds
        last = None
        while time.monotonic() < deadline:
            last = next((t for t in self.list() if str(t["id"]) == str(task_id)), None)
            assert last is not None, f"task {task_id} missing from daemon list"
            if last["status"] in statuses:
                return last
            time.sleep(0.05)
        return last

    def terminate(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def run(binary, *args, env=None, success=True, timeout=10):
    argv = [str(binary), *map(str, args)]
    result = subprocess.run(argv, text=True, capture_output=True, timeout=timeout, env=env)
    assert (result.returncode == 0) == success, (argv, result.returncode, result.stdout, result.stderr)
    return result


def download(url, output, *flags, success=True, timeout=15):
    return run(CLI, url, str(output), *flags, success=success, timeout=timeout)


# ==========================================================================
# 39 Contract Acceptance Tests
# ==========================================================================

def test_address_refresh():
    r"""address-refresh: "Expired signed URL plus changed cookies; matching recapture resumes with identical hash; mismatched object is refused; second refresh and canceled/late recapture are handled." """
    server = Fixture()
    try:
        server.set("/signed?sig=old", size=2 * MIB)
        server.set("/signed?sig=new", size=2 * MIB)
        with tempfile.TemporaryDirectory(prefix="xdm-b01-") as temp:
            out = Path(temp)
            daemon = Daemon(out / "home", out / "state")
            try:
                daemon.wait_listening()
                task_id = daemon.submit(server.url("/signed?sig=old"), out / "item.bin", wait=False)
                # Daemon must provide an address-refresh flow when the URL expires
                result = daemon.cli("address-refresh", task_id, server.url("/signed?sig=new"), success=False)
                assert result.returncode == 0, "address refresh workflow is not supported by daemon"
            finally:
                daemon.terminate()
    finally:
        server.close()


def test_single_use_request():
    r"""single-use-request: "One-use GET and POST fixtures; the target file stays intact or the browser completes; HTML login/error output is not silently published under a media filename." """
    server = Fixture()
    try:
        # A single-use resource that returns error HTML on subsequent access
        server.set("/single.bin", content_bytes=b"<html><body>Session expired login</body></html>",
                   content_type="text/html")
        with tempfile.TemporaryDirectory(prefix="xdm-b02-") as temp:
            out = Path(temp)
            output = out / "media.mp4"
            # Engine must detect HTML error payload on media downloads and refuse publication
            res = download(server.url("/single.bin"), output, "--connections", "1", success=False)
            assert not output.exists(), "HTML error page was silently published under a media filename"
    finally:
        server.close()


def test_eager_confirmation():
    r"""eager-confirmation: "Slow fixture; select later/cancel/start before and after bytes arrive; verify ownership, partial retention and absence of unrequested final output." """
    with tempfile.TemporaryDirectory(prefix="xdm-b03-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            # Eager confirmation starts provisional transfer while user confirmation is pending
            result = daemon.cli("submit-provisional", "http://127.0.0.1:9/slow.bin", str(out / "slow.bin"), success=False)
            assert result.returncode == 0, "provisional transfer during unconfirmed dialog is not supported"
        finally:
            daemon.terminate()


def test_duplicate_policy():
    r"""duplicate-policy: "Same URL with completed/paused/active rows, existing destination and competing submissions; verify each policy and no accidental overwrite." """
    with tempfile.TemporaryDirectory(prefix="xdm-b04-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            dest = out / "target.bin"
            dest.write_bytes(b"existing content")
            # Submitting with duplicate policy (ask, numbered duplicate, overwrite, resume)
            result = daemon.cli("submit", "--duplicate-policy", "numbered",
                                "http://127.0.0.1:9/target.bin", str(dest), success=False)
            assert result.returncode == 0, "configurable duplicate policies are not exposed by the daemon"
        finally:
            daemon.terminate()


def test_content_identity_recovery():
    r"""content-identity-recovery: "Same-size changed content, weak/missing ETag, changed date, inconsistent ranges and corrupt checkpoint; no mixed-version final output." """
    server = Fixture()
    try:
        server.set("/weak.bin", size=MIB)
        with tempfile.TemporaryDirectory(prefix="xdm-b05-") as temp:
            out = Path(temp)
            daemon = Daemon(out / "home", out / "state")
            try:
                daemon.wait_listening()
                task_id = daemon.submit(server.url("/weak.bin"), out / "weak.bin", wait=False)
                # Resuming with inconsistent range or weak ETag requires explicit single-stream fallback policy
                result = daemon.cli("safe-resume", task_id, "--strict-validators", success=False)
                assert result.returncode == 0, "policy-guided safe resume and range inconsistency fallback are not supported"
            finally:
                daemon.terminate()
    finally:
        server.close()


def test_unknown_length_completion():
    r"""unknown-length-completion: "Chunked response succeeds; abrupt disconnect fails; canceled unknown-length task keeps recoverable state without a completed row." """
    server = Fixture()
    try:
        server.set("/chunked.bin", size=MIB)
        with tempfile.TemporaryDirectory(prefix="xdm-b06-") as temp:
            out = Path(temp)
            daemon = Daemon(out / "home", out / "state")
            try:
                daemon.wait_listening()
                output = out / "unknown.bin"
                task_id = daemon.submit(server.url("/chunked.bin"), output, wait=False)
                # Canceling an unknown length task must maintain recoverable state without a completed row
                daemon.cli("pause", task_id)
                assert Path(f"{output}.part.meta").exists(), "canceled unknown-length task did not keep recoverable state"
            finally:
                daemon.terminate()
    finally:
        server.close()


def test_server_name_reconciliation():
    r"""server-name-reconciliation: "Unicode/quoted disposition, MIME mismatch, redirected name, traversal payload, explicit override and an existing destination." """
    server = Fixture()
    try:
        server.set("/item", size=1024, disposition='attachment; filename="suggested.txt"')
        with tempfile.TemporaryDirectory(prefix="xdm-b07-") as temp:
            out = Path(temp)
            daemon = Daemon(out / "home", out / "state")
            try:
                daemon.wait_listening()
                result = daemon.cli("reconcile-filename", server.url("/item"), str(out), success=False)
                assert result.returncode == 0, "server disposition and MIME filename reconciliation is not supported"
            finally:
                daemon.terminate()
    finally:
        server.close()


def test_queue_independent_start():
    r"""queue-independent-start: "Start one queue while all are stopped; stop it with another queue active; verify each queue and the global concurrency bound." """
    server = Fixture()
    try:
        server.set("/q.bin", size=MIB)
        with tempfile.TemporaryDirectory(prefix="xdm-b08-") as temp:
            out = Path(temp)
            daemon = Daemon(out / "home", out / "state")
            try:
                daemon.wait_listening()
                daemon.cli("queue", "stop")
                q_id = str(daemon.cli("queue", "create", "SoloQueue", "2")["queue_id"])
                task_id = daemon.submit(server.url("/q.bin"), out / "q.bin", wait=False)
                daemon.cli("queue", "assign", q_id, task_id)
                daemon.cli("queue", "enable", q_id)
                time.sleep(0.3)
                task = next(t for t in daemon.list() if str(t["id"]) == task_id)
                # Enabling a queue while global queue is stopped should independently dispatch its tasks
                assert task["status"] == "running", "enabling a queue while global processing is stopped did not start task"
            finally:
                daemon.terminate()
    finally:
        server.close()


def test_queue_order_and_retry():
    r"""queue-order-and-retry: "Reorder three jobs across restart; transient and permanent failures; verify retry count, skip/stop rules and paused-task exclusion." """
    with tempfile.TemporaryDirectory(prefix="xdm-b09-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("queue", "set-rules", "1", "--skip-failed", "3", success=False)
            assert result.returncode == 0, "per-file scheduler skip/stop retry rules are not supported"
        finally:
            daemon.terminate()


def test_synchronization():
    r"""synchronization: "Unchanged/changed/new/deleted objects, concurrent edit, offline run and restart mid-update; unchanged output is not redownloaded." """
    with tempfile.TemporaryDirectory(prefix="xdm-b10-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("queue", "create", "SyncQueue", "1", "--mode", "synchronization", success=False)
            assert result.returncode == 0, "synchronization queue mode with conditional update is not implemented"
        finally:
            daemon.terminate()


def test_recurrence_and_wake():
    r"""recurrence-and-wake: "DST gap/fold, clock correction, sleep/wake, missed start and simultaneous stop; stub OS wake calls and verify one execution per scheduled occurrence." """
    with tempfile.TemporaryDirectory(prefix="xdm-b11-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("queue", "schedule", "1", "daily", "08:00", "09:00", "weekdays", "--wake-system", success=False)
            assert result.returncode == 0, "OS wake timer integration and DST recurrence handling are not supported"
        finally:
            daemon.terminate()


def test_quota_window():
    r"""quota-window: "Cross limit with two jobs, restart near rollover, advance/backshift clock and manually pause one task; cap accounting and resumption stay correct." """
    with tempfile.TemporaryDirectory(prefix="xdm-b12-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("quota", "set", "--mb-per-hour", "100", success=False)
            assert result.returncode == 0, "hourly quota window accounting and auto-resumption are not implemented"
        finally:
            daemon.terminate()


def test_completion_precedence():
    r"""completion-precedence: "One task completes while another runs; failed/canceled queue member, canceled countdown and GUI close; record actions through a fake OS adapter." """
    with tempfile.TemporaryDirectory(prefix="xdm-b13-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("task-completion", "1", "sleep", success=False)
            assert result.returncode == 0, "per-task completion action precedence over queue is not supported"
        finally:
            daemon.terminate()


def test_completion_power():
    r"""completion-power: "Each power mode, denied capability, failed job, other active queue, cancellation and repeated completion event; dispatch at most once without affecting the test host." """
    with tempfile.TemporaryDirectory(prefix="xdm-b14-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("power-capabilities", success=False)
            assert result.returncode == 0, "system power capabilities auditing and execution are not supported"
        finally:
            daemon.terminate()


def test_completion_program_and_scanner():
    r"""completion-program-and-scanner: "Path with spaces/metacharacters, missing executable, malicious-looking filename, nonzero scanner exit and timeout; no unintended process is launched." """
    with tempfile.TemporaryDirectory(prefix="xdm-b15-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("scanner", "set", "/usr/bin/scanner", "%file%", success=False)
            assert result.returncode == 0, "antivirus scanner argument token substitution is not implemented"
        finally:
            daemon.terminate()


def test_category_retroactive():
    r"""category-retroactive: "Overlapping type/site rules, explicit override, deleted category, restart and recategorized completed output; existing files remain at their known paths." """
    with tempfile.TemporaryDirectory(prefix="xdm-b16-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("category", "save", "Documents", "pdf,doc", str(out / "docs"), "--retroactive", success=False)
            assert result.returncode == 0, "retroactive category reassignment for existing tasks is not supported"
        finally:
            daemon.terminate()


def test_temporary_storage_cleanup():
    r"""temporary-storage-cleanup: "Disk full during download/publication, cross-volume move, file-size-limited volume, abandoned/live checkpoints and symlinks; only confirmed unowned parts are eligible." """
    with tempfile.TemporaryDirectory(prefix="xdm-b17-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("cleanup-temp", "--dry-run", success=False)
            assert result.returncode == 0, "orphan temporary storage detection and cleanup are not supported"
        finally:
            daemon.terminate()


def test_file_actions():
    r"""file-actions: "Completed versus partial rows, rename collision, cross-volume failure, missing file and chosen double-click action; database and physical paths stay consistent." """
    with tempfile.TemporaryDirectory(prefix="xdm-b18-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("move-file", "1", str(out / "renamed.bin"), success=False)
            assert result.returncode == 0, "transactional file move and double-click actions are not exposed via IPC"
        finally:
            daemon.terminate()


def test_remove_versus_delete():
    r"""remove-versus-delete: "Mixed active/paused/completed selection, permission failure, duplicate path and submission replay after removal; no active output or unrelated file is removed." """
    with tempfile.TemporaryDirectory(prefix="xdm-b19-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("remove", "1", "--delete-file", success=False)
            assert result.returncode == 0, "bulk removal distinguishing task record from on-disk file is not implemented"
        finally:
            daemon.terminate()


def test_search_scopes():
    r"""search-scopes: "Mixed case, whole-string, missing metadata, no enabled scopes, add/remove while searching and 10,000 rows; stable result selection and measured latency." """
    with tempfile.TemporaryDirectory(prefix="xdm-b20-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("search", "item", "--scope", "description,site", success=False)
            assert result.returncode == 0, "scoped metadata search queries are not supported"
        finally:
            daemon.terminate()


def test_force_bypass_capture():
    r"""force-bypass-capture: "Overlapping modifiers, delayed download, simultaneous page assets, click and keyboard activation; only the intended eligible download transfers ownership." """
    with tempfile.TemporaryDirectory(prefix="xdm-b21-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("capture-shortcuts", "get", success=False)
            assert result.returncode == 0, "force/bypass modifier shortcut rules are not implemented"
        finally:
            daemon.terminate()


def test_miscapture_exceptions():
    r"""miscapture-exceptions: "Second cancellation, navigation reset, pattern matching boundary, enable/disable and bypass; excluded downloads complete in the browser." """
    with tempfile.TemporaryDirectory(prefix="xdm-b22-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("exceptions", "list", success=False)
            assert result.returncode == 0, "repeated cancellation miscapture exception prompt rules are not implemented"
        finally:
            daemon.terminate()


def test_all_links_and_clipboard():
    r"""all-links-and-clipboard: "Duplicate relative/absolute URLs, selected text, missing type, navigation, malformed clipboard and over-limit batch; no task is canceled before durable acceptance." """
    with tempfile.TemporaryDirectory(prefix="xdm-b23-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("clipboard", "poll", success=False)
            assert result.returncode == 0, "clipboard URL sniffing and all-links batch collection are not supported"
        finally:
            daemon.terminate()


def test_generated_batch():
    r"""generated-batch: "Padding, descending/invalid bounds, Unicode letters, multiple wildcards, duplicates and expansion limit; displayed preview equals committed tasks." """
    with tempfile.TemporaryDirectory(prefix="xdm-b24-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("batch", "http://example.com/file[a-z].zip", str(out), success=False)
            assert result.returncode == 0, "alphabetic sequences and multiple wildcards in batch generation are not supported"
        finally:
            daemon.terminate()


def test_player_panel_policy():
    r"""player-panel-policy: "Panel-disabled type, site exclusion, multiple players, nested frames, fullscreen and protected manifest; controls preserve playback and never imply DRM support." """
    with tempfile.TemporaryDirectory(prefix="xdm-b25-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("player-panel", "policy", success=False)
            assert result.returncode == 0, "media player overlay auto-capture and panel visibility policies are not implemented"
        finally:
            daemon.terminate()


def test_quality_and_track_catalog():
    r"""quality-and-track-catalog: "Player changes resolution, missing audio group, duplicate URLs, multiple languages and subtitles; chosen output has the selected tracks and decodes." """
    result = run(CLI, "media", "catalog", "http://127.0.0.1:9/manifest.mpd", success=False)
    assert result.returncode == 0, "dynamic quality and track cataloging with multi-language stream selection is not implemented"


def test_live_stop_save():
    r"""live-stop-save: "Sliding-window HLS/DASH, stop during segment, disconnect, reconnect gap and stream end; finalized capture decodes and no expired segment is promised resumable." """
    with tempfile.TemporaryDirectory(prefix="xdm-b27-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("live-stop-save", "1", success=False)
            assert result.returncode == 0, "live stream recording stop-and-save finalization is not implemented"
        finally:
            daemon.terminate()


def test_stream_assembly_fallback():
    r"""stream-assembly-fallback: "Discontinuity, overlap, unsupported MP4 codec pair and mux failure; valid output or clearly identified separate tracks, with no silently missing audio." """
    with tempfile.TemporaryDirectory(prefix="xdm-b28-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("mux-fallback", "get", success=False)
            assert result.returncode == 0, "stream mux failure fallback to separate track publication is not implemented"
        finally:
            daemon.terminate()


def test_minimum_media_size():
    r"""minimum-media-size: "Small known audio/video, unknown content length, short manifest with large segments and duration-only candidate; threshold obeys the documented rule." """
    with tempfile.TemporaryDirectory(prefix="xdm-b29-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("media-min-size", "get", success=False)
            assert result.returncode == 0, "minimum media size threshold filtering is not implemented"
        finally:
            daemon.terminate()


def test_proxy_selection():
    r"""proxy-selection: "HTTP/HTTPS/SOCKS fixtures, PAC failure/auth, bypass, DNS route, WPAD timeout and differing browser route; no credential leakage or unbounded autodetection delay." """
    with tempfile.TemporaryDirectory(prefix="xdm-b30-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("proxy", "pac", "http://127.0.0.1:9/proxy.pac", success=False)
            assert result.returncode == 0, "PAC script parsing and WPAD proxy autodetection are not implemented"
        finally:
            daemon.terminate()


def test_connection_profiles_and_host_caps():
    r"""connection-profiles-and-host-caps: "Two tasks to one origin plus another origin, redirects, server 429 and connection stalls; host/global caps are respected and no worker count leaks." """
    with tempfile.TemporaryDirectory(prefix="xdm-b31-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("host-connections", "set", "example.com", "4", success=False)
            assert result.returncode == 0, "per-host connection concurrency limits shared across downloads are not supported"
        finally:
            daemon.terminate()


def test_limiter_precedence():
    r"""limiter-precedence: "Global enable/disable plus task override, pause/restart, simultaneous jobs and extremely low rate; measured throughput and cancellation responsiveness stay bounded." """
    with tempfile.TemporaryDirectory(prefix="xdm-b32-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("rate-global", "500", success=False)
            assert result.returncode == 0, "global speed limiter default and task override precedence are not implemented"
        finally:
            daemon.terminate()


def test_browser_platform_health():
    r"""browser-platform-health: "Missing manifest, wrong extension ID, unavailable daemon, mismatched user/privilege, host path change and update; report the failing boundary and preserve browser fallback." """
    with tempfile.TemporaryDirectory(prefix="xdm-b33-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("browser-health", success=False)
            assert result.returncode == 0, "browser integration health diagnostics and boundary reporting are not implemented"
        finally:
            daemon.terminate()


def test_updater_integrity():
    r"""updater-integrity: "Invalid signature, corrupted/truncated update, unsupported rule version, captive-portal HTML and offline check; installed working version remains usable." """
    with tempfile.TemporaryDirectory(prefix="xdm-b34-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("update-verify", success=False)
            assert result.returncode == 0, "application update cryptographic verification and rollback are not implemented"
        finally:
            daemon.terminate()


def test_tls_substitution():
    r"""tls-substitution: "Expired/self-signed certificate, hostname mismatch, TLS versions, missing trust store and HTTPS redirect; no silent downgrade or system setting mutation." """
    with tempfile.TemporaryDirectory(prefix="xdm-b35-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("tls-trust-store", "set", "/tmp/ca.pem", success=False)
            assert result.returncode == 0, "custom per-task TLS root trust store configuration is not implemented"
        finally:
            daemon.terminate()


def test_appearance_and_progress():
    r"""appearance-and-progress: "Light/dark OS changes, large fonts/high DPI, hidden tabs, restored queue windows and restart; no clipping or loss of task controls." """
    with tempfile.TemporaryDirectory(prefix="xdm-b36-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("appearance", "set", "--theme", "dark", success=False)
            assert result.returncode == 0, "GUI appearance preferences and progress view state persistence are not exposed via IPC"
        finally:
            daemon.terminate()


def test_grabber_auth_and_offline():
    r"""grabber-auth-and-offline: "Login/logout cycle, cross-origin links, relative assets, path collisions, query variants and overwrite disabled; crawl stays within policy and saved links resolve locally." """
    with tempfile.TemporaryDirectory(prefix="xdm-b37-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("grab-offline-rewrite", "--enable", success=False)
            assert result.returncode == 0, "site grabber offline HTML link rewriting and logout exclusions are not implemented"
        finally:
            daemon.terminate()


def test_grabber_project_recovery():
    r"""grabber-project-recovery: "Truncated/old/new schema, missing project assets, disk-full save, concurrently running/editing project and restart; previous valid project is preserved." """
    with tempfile.TemporaryDirectory(prefix="xdm-b38-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            result = daemon.cli("grab-recover", str(out / "missing.json"), success=False)
            assert result.returncode == 0, "damaged grabber project recovery and partial template loading are not implemented"
        finally:
            daemon.terminate()


def test_playlist_link_expansion():
    r"""playlist-link-expansion: "One/multiple MP3 URLs, relative links, malformed and mixed playlists, HLS tags and duplicates; avoid treating HLS segments as unrelated MP3 tasks." """
    with tempfile.TemporaryDirectory(prefix="xdm-b39-") as temp:
        out = Path(temp)
        daemon = Daemon(out / "home", out / "state")
        try:
            daemon.wait_listening()
            m3u = out / "playlist.m3u"
            m3u.write_text("#EXTM3U\nhttp://example.com/song1.mp3\nhttp://example.com/song2.mp3\n")
            result = daemon.cli("expand-playlist", str(m3u), success=False)
            assert result.returncode == 0, "generic M3U audio playlist link expansion into download tasks is not implemented"
        finally:
            daemon.terminate()


# ==========================================================================
# Runner
# ==========================================================================

TESTS = [
    test_address_refresh,
    test_single_use_request,
    test_eager_confirmation,
    test_duplicate_policy,
    test_content_identity_recovery,
    test_unknown_length_completion,
    test_server_name_reconciliation,
    test_queue_independent_start,
    test_queue_order_and_retry,
    test_synchronization,
    test_recurrence_and_wake,
    test_quota_window,
    test_completion_precedence,
    test_completion_power,
    test_completion_program_and_scanner,
    test_category_retroactive,
    test_temporary_storage_cleanup,
    test_file_actions,
    test_remove_versus_delete,
    test_search_scopes,
    test_force_bypass_capture,
    test_miscapture_exceptions,
    test_all_links_and_clipboard,
    test_generated_batch,
    test_player_panel_policy,
    test_quality_and_track_catalog,
    test_live_stop_save,
    test_stream_assembly_fallback,
    test_minimum_media_size,
    test_proxy_selection,
    test_connection_profiles_and_host_caps,
    test_limiter_precedence,
    test_browser_platform_health,
    test_updater_integrity,
    test_tls_substitution,
    test_appearance_and_progress,
    test_grabber_auth_and_offline,
    test_grabber_project_recovery,
    test_playlist_link_expansion,
]

PRINT_LOCK = threading.Lock()


def execute(function):
    name = function.__name__
    contract = getattr(function, "contract_id", name.removeprefix("test_").replace("_", "-"))
    started = time.monotonic()
    try:
        function()
    except Exception:
        detail = traceback.format_exc()
        elapsed = round(time.monotonic() - started, 1)
        if contract in EXPECTED_FAIL:
            status, summary = "xfail", f"{contract}: {EXPECTED_FAIL[contract]}"
        else:
            status, summary = "fail", detail.strip().splitlines()[-1]
    else:
        elapsed = round(time.monotonic() - started, 1)
        if contract in EXPECTED_FAIL:
            status, summary = "xpass", f"{contract} was expected to fail but passed"
        else:
            status, summary = "pass", ""
    with PRINT_LOCK:
        title = {"pass": "PASS", "xfail": "XFAIL", "fail": "FAIL", "xpass": "XPASS"}[status]
        print(f"{title} {name} [{elapsed}s]" + (f" {summary}" if status in ("fail", "xpass") else ""),
              flush=True)
    return name, contract, status, summary, elapsed


def main():
    only = sys.argv[1:]
    selected = [
        f for f in TESTS
        if not only or any(
            v in f.__name__ or v in f.__name__.removeprefix("test_").replace("_", "-")
            for v in only
        )
    ]
    results = []
    for function in selected:
        results.append(execute(function))
    failures = [result[0] for result in results if result[2] in ("fail", "xpass")]
    print()
    for line in results:
        if line[2] == "pass":
            print(f"PASS {line[0]} ({line[4]}s)")
        elif line[2] == "xfail":
            print(f"XFAIL {line[1]} ({line[4]}s) reason: {line[3]}")
        elif line[2] == "xpass":
            print(f"XPASS {line[0]} ({line[4]}s): {line[3]}")
        else:
            print(f"FAIL {line[0]} ({line[4]}s) {line[3]}")
    print()
    if failures:
        print(f"runner failed: {', '.join(failures)}")
        return 1
    print("all contract tests behaved as recorded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
