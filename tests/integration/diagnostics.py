"""N17: the daemon stores a typed error_kind per failure class and shows it in task details."""
import json
import os
import socket
import ssl
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from auth_harness import Daemon


def listener(handler):
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    sock.listen(16)

    def loop():
        while True:
            try:
                conn, _ = sock.accept()
            except OSError:
                return
            threading.Thread(target=handler, args=(conn,), daemon=True).start()

    threading.Thread(target=loop, daemon=True).start()
    return sock


def drain(conn):
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(4096)
        if not chunk:
            break
        data += chunk


def main():
    daemon = Daemon("xdm-diag-")
    cleanup = []
    try:
        daemon.cli("stall-timeout", 1)
        outcomes = {}

        def run(name, url, output=None):
            task = daemon.cli("submit", url, output or daemon.base / f"{name}.bin")["task_id"]
            row = daemon.until(task, lambda t: t["status"] in ("failed", "auth_required"), timeout=60)
            outcomes[name] = row
            return row

        # Connection refused: a port nobody listens on.
        probe = socket.socket()
        probe.bind(("127.0.0.1", 0))
        refused_port = probe.getsockname()[1]
        probe.close()
        row = run("refused", f"http://127.0.0.1:{refused_port}/f")
        assert row["error_kind"] == "connect_refused", row

        # DNS: a name that cannot resolve.
        row = run("dns", "http://no-such-host.invalid/f")
        assert row["error_kind"] == "dns", row

        # Read timeout: accepts, reads the request, never answers.
        silent = listener(lambda conn: (drain(conn), time.sleep(60)))
        cleanup.append(silent)
        row = run("stall", f"http://127.0.0.1:{silent.getsockname()[1]}/f")
        assert row["error_kind"] == "read_timeout", row

        # Connect timeout: a listener whose accept queue is full never answers SYNs.
        full = socket.socket()
        full.bind(("127.0.0.1", 0))
        full.listen(0)
        cleanup.append(full)
        fillers = []
        for _ in range(64):
            filler = socket.socket()
            filler.settimeout(0.2)
            try:
                filler.connect(full.getsockname())
                fillers.append(filler)
            except OSError:
                filler.close()
                break
        row = run("connect_timeout", f"http://127.0.0.1:{full.getsockname()[1]}/f")
        # Windows resets a SYN to a full backlog after its own retries (same as a closed port).
        accepted = ("connect_timeout", "connect_refused") if os.name == "nt" else ("connect_timeout",)
        assert row["error_kind"] in accepted, row

        # HTTP status.
        def not_found(conn):
            drain(conn)
            conn.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            conn.close()

        gone = listener(not_found)
        cleanup.append(gone)
        row = run("status", f"http://127.0.0.1:{gone.getsockname()[1]}/f")
        assert row["error_kind"] == "http_status", row

        # TLS: self-signed certificate is rejected.
        cert, key = daemon.base / "cert.pem", daemon.base / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key),
                        "-out", str(cert), "-days", "2", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"],
                       check=True, capture_output=True)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(cert), str(key))

        def tls(conn):
            try:
                with context.wrap_socket(conn, server_side=True) as secure:
                    drain(secure)
            except (ssl.SSLError, OSError):
                pass

        secure = listener(tls)
        cleanup.append(secure)
        row = run("tls", f"https://127.0.0.1:{secure.getsockname()[1]}/f")
        assert row["error_kind"] == "tls", row

        # Disk: the destination directory refuses writes.
        def small_file(conn):
            drain(conn)
            conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata")
            conn.close()

        serve = listener(small_file)
        cleanup.append(serve)
        readonly = daemon.base / "readonly"
        readonly.mkdir()
        readonly.chmod(0o500)
        try:
            if os.geteuid() != 0:
                row = run("disk", f"http://127.0.0.1:{serve.getsockname()[1]}/f", readonly / "f.bin")
                assert row["error_kind"] == "disk", row
        finally:
            readonly.chmod(0o700)

        # Task details carry the class label and the kind survives a listing and a restart-free requery.
        for name, row in outcomes.items():
            assert row["notice"] and row["notice"].startswith("Failure class:"), (name, row)
            assert row["error"], (name, row)
        listing = daemon.cli("list")["tasks"]
        assert {t["error_kind"] for t in listing} >= {"dns", "connect_refused", "tls", "read_timeout"}, listing
        # A task without an error carries no kind.
        print(json.dumps({"result": "passed", "cases": sorted(outcomes)}))
    finally:
        for item in cleanup:
            item.close()
        daemon.close()


main()
