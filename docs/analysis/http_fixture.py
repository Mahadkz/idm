"""Local HTTP fixture for observing the original IDM under Wine.

This is investigation tooling, not the proposed download engine.
Run with an output directory; prints its port and writes requests.jsonl.
"""
import argparse
import hashlib
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
parser.add_argument('--port', type=int, default=0)
parser.add_argument('--delay', type=float, default=0.25)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
SIZE = 64 * 1024 * 1024
BLOCK = bytes(range(256)) * 256
lock = threading.Lock()

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *args):
        pass

    def record(self, event, **extra):
        record = {'time': time.time(), 'event': event, 'method': self.command,
                  'path': self.path, 'headers': dict(self.headers), **extra}
        with lock, (args.output / 'requests.jsonl').open('a') as f:
            f.write(json.dumps(record) + '\n')

    def do_HEAD(self):
        self.respond(False)

    def do_GET(self):
        self.respond(True)

    def respond(self, body):
        self.record('request')
        if self.path == '/':
            data = b'<html><body><a href="/range.bin">Range fixture</a><a href="/no-range.bin">Sequential fixture</a></body></html>'
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            if body:
                self.wfile.write(data)
            return
        if self.path not in ['/range.bin', '/no-range.bin']:
            self.send_error(404)
            return
        start, end, status = 0, SIZE - 1, 200
        value = self.headers.get('Range')
        if value and self.path == '/range.bin':
            match = re.fullmatch(r'bytes=(\d+)-(\d*)', value)
            if not match or int(match[1]) >= SIZE:
                self.send_response(416)
                self.send_header('Content-Range', f'bytes */{SIZE}')
                self.send_header('Content-Length', '0')
                self.end_headers()
                return
            start = int(match[1])
            end = min(int(match[2]) if match[2] else SIZE-1, SIZE-1)
            if end < start:
                self.send_error(416)
                return
            status = 206
        self.send_response(status)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(end - start + 1))
        self.send_header('ETag', '"xdm-analysis-v1"')
        self.send_header('Last-Modified', 'Mon, 12 Oct 2026 00:00:00 GMT')
        self.send_header('Accept-Ranges', 'bytes' if self.path == '/range.bin' else 'none')
        if status == 206:
            self.send_header('Content-Range', f'bytes {start}-{end}/{SIZE}')
        self.end_headers()
        if not body:
            return
        position = start
        try:
            while position <= end:
                count = min(32768, end-position+1)
                offset = position % 256
                self.wfile.write(BLOCK[offset:offset+count])
                self.wfile.flush()
                position += count
                time.sleep(args.delay)
            self.record('finished', start=start, end=end, bytes_sent=position-start)
        except (ConnectionError, OSError):
            self.record('disconnected', start=start, end=end, bytes_sent=position-start)

server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
(args.output / 'server.json').write_text(json.dumps({'port': server.server_port, 'size': SIZE}))
print(json.dumps({'port': server.server_port, 'size': SIZE}), flush=True)
server.serve_forever()
