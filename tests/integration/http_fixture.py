"""Disposable HTTP server with engine faults: one-shot links, POST-only files, no-range and flaky bodies, stalls and range corruption."""
import argparse
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('log', type=Path)
parser.add_argument('--delay', type=float, default=0.02)
args = parser.parse_args()
SIZE = 4 * 1024 * 1024
BLOCK = bytes(range(256)) * 256
lock = threading.Lock()
attempts = {}


def payload(start, end):
    chunks = []
    position = start
    while position <= end:
        offset = position % len(BLOCK)
        count = min(len(BLOCK) - offset, end - position + 1)
        chunks.append(BLOCK[offset:offset + count])
        position += count
    return b''.join(chunks)


class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *unused):
        pass

    def do_HEAD(self):
        self.handle_request(b'')

    def do_GET(self):
        self.handle_request(b'')

    def do_POST(self):
        length = int(self.headers.get('Content-Length') or 0)
        self.handle_request(self.rfile.read(length))

    def plain(self, status, headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def handle_request(self, body):
        path = urlsplit(self.path).path
        with lock:
            attempts[path] = attempts.get(path, 0) + 1
            attempt = attempts[path]
            with args.log.open('a') as file:
                file.write(json.dumps({'path': path, 'method': self.command, 'attempt': attempt,
                                       'range': self.headers.get('Range'),
                                       'cookie': self.headers.get('Cookie'),
                                       'body': body.decode('latin-1')}) + '\n')
        if self.command == 'HEAD':
            return self.plain(405)
        ranged = path not in ['/norange.bin', '/flaky-norange.bin', '/oneshot-norange.bin']
        size = 0 if path == '/empty.bin' else SIZE
        if path in ['/oneshot.bin', '/oneshot-norange.bin'] and attempt > 1:
            return self.plain(410)
        if path == '/post.bin' and (self.command != 'POST' or body != b'token=abc'):
            return self.plain(405 if self.command != 'POST' else 403)
        if path == '/post-307' and self.command == 'POST':
            return self.plain(307, [('Location', '/post.bin')])
        if path == '/post-302':
            return self.plain(302, [('Location', '/post.bin')])
        if path == '/cookie-redirect':
            target = parse_qs(urlsplit(self.path).query)['to'][0]
            return self.plain(302, [('Location', target)])
        start, end, status = 0, size - 1, 200
        value = self.headers.get('Range')
        if value and ranged:
            match = re.fullmatch(r'bytes=(\d+)-(\d*)', value)
            if not match or int(match[1]) >= size:
                return self.plain(416, [('Content-Range', f'bytes */{size}')])
            start, end, status = int(match[1]), min(int(match[2] or size - 1), size - 1), 206
        flip = path == '/flip.bin' and attempt > 2
        if flip:
            # N05: let the honest streams bank checkpointed progress before the
            # corrupt answer arrives, so the failure leaves a resumable checkpoint.
            time.sleep(0.2)
        self.send_response(status)
        self.send_header('Content-Length', str(end - start + 1))
        self.send_header('ETag', '"engine-v1"')
        if status == 206:
            announced = start + 1 if flip else start
            self.send_header('Content-Range', f'bytes {announced}-{end}/{size}')
        self.end_headers()
        position = start
        half = start + (end - start + 1) // 2
        try:
            while position <= end:
                count = min(64 * 1024, end - position + 1)
                if path == '/flaky-norange.bin' and attempt == 1 and position >= SIZE // 2:
                    self.close_connection = True
                    return  # disconnect halfway through the first full response
                # N31: the second range request sends half its body, then holds the
                # connection open for a long time without closing it or sending more.
                if path == '/stall.bin' and attempt == 2 and position >= half:
                    time.sleep(30)
                self.wfile.write(payload(position, position + count - 1))
                self.wfile.flush()
                position += count
                time.sleep(args.delay)
        except (ConnectionError, OSError):
            pass


server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
server.daemon_threads = True
print(json.dumps({'port': server.server_port, 'size': SIZE}), flush=True)
try:
    server.serve_forever()
except KeyboardInterrupt:
    pass
