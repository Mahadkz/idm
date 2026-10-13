#!/usr/bin/env python3
"""Deterministic Range-capable HTTP fixture for the XDM resource benchmark.

Serves a body of ``--size-mib`` MiB (default 64) built by tiling a fixed 1 MiB
block, so byte N is the same no matter which window a client requests. There is
no artificial delay and every connection is handled by its own thread, which
keeps the fixture from becoming the bottleneck the benchmark is measuring.

Prints a single JSON line ``{"port", "size", "sha256"}`` on startup so the
harness can hash-verify downloads, then serves until terminated.
"""

import argparse
import hashlib
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BLOCK = bytes(range(256)) * (1024 * 1024 // 256)
CHUNK = 256 * 1024


def parse_range(value: str, size: int):
    """Returns None for no/invalid Range, "unsatisfiable", or (start, end)."""
    match = re.fullmatch(r"bytes=(\d+)-(\d*)", value or "")
    if not match:
        return None
    start = int(match.group(1))
    if start >= size:
        return "unsatisfiable"
    end = int(match.group(2)) if match.group(2) else size - 1
    return start, min(end, size - 1)


def expected_sha256(size: int) -> str:
    digest = hashlib.sha256()
    written = 0
    while written < size:
        count = min(len(BLOCK), size - written)
        digest.update(BLOCK[:count])
        written += count
    return digest.hexdigest()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    size = 0

    def log_message(self, *unused):
        pass

    def do_HEAD(self):
        self.send_response(200)
        self.send_header("Content-Length", str(self.size))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"xdm-bench-v1"')
        self.end_headers()

    def do_GET(self):
        size = self.size
        requested = parse_range(self.headers.get("Range"), size)
        if requested == "unsatisfiable":
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if requested is None:
            start, end, status = 0, size - 1, 200
        else:
            start, end, status = requested[0], requested[1], 206
        self.send_response(status)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", '"xdm-bench-v1"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        try:
            position = start
            while position <= end:
                offset = position % len(BLOCK)
                count = min(len(BLOCK) - offset, end - position + 1, CHUNK)
                self.wfile.write(BLOCK[offset:offset + count])
                position += count
        except (BrokenPipeError, ConnectionResetError, OSError):
            # A cancelled benchmark leaves half-read sockets behind.
            pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--size-mib", type=int, default=64)
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    if args.size_mib <= 0:
        parser.error("--size-mib must be positive")
    Handler.size = args.size_mib * 1024 * 1024
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    print(
        json.dumps(
            {
                "port": server.server_port,
                "size": Handler.size,
                "sha256": expected_sha256(Handler.size),
            }
        ),
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
