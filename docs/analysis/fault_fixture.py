"""Controlled local HTTP faults for auditing the original download manager."""
import argparse
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('output', type=Path)
parser.add_argument('--port', type=int, default=0)
parser.add_argument('--delay', type=float, default=0.08)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
SIZE = 8 * 1024 * 1024
BLOCK = bytes(range(256)) * 128
lock = threading.Lock()
attempts = {}
version = 1

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *unused):
        pass

    def record(self, event, **fields):
        with lock, (args.output/'requests.jsonl').open('a') as file:
            file.write(json.dumps({'time': time.time(), 'event': event,
                                   'path': self.path, **fields})+'\n')

    def do_HEAD(self):
        self.respond(False)

    def do_GET(self):
        self.respond(True)

    def respond(self, body):
        global version
        path = urlsplit(self.path).path
        if path == '/control/version2':
            version = 2
            self.send_response(200)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        with lock:
            attempts[path] = attempts.get(path, 0)+1
            attempt = attempts[path]
            served_version = version if path == '/changed.bin' else 1
        status, start, end = 200, 0, SIZE-1
        value = self.headers.get('Range')
        if value:
            match = re.fullmatch(r'bytes=(\d+)-(\d*)', value)
            if not match or int(match[1]) >= SIZE:
                status = 416
            else:
                start = int(match[1])
                end = min(int(match[2]) if match[2] else SIZE-1, SIZE-1)
                status = 206 if start <= end else 416
        if path == '/retry.bin' and attempt <= 2:
            status = 503
        if path == '/denied.bin':
            status = 403
        self.record('request', method=self.command, headers=dict(self.headers),
                    status=status, attempt=attempt, version=served_version)
        self.send_response(status)
        if status in [403, 416, 503]:
            self.send_header('Content-Length', '0')
            if status == 503:
                self.send_header('Retry-After', '1')
            if status == 416:
                self.send_header('Content-Range', f'bytes */{SIZE}')
            self.end_headers()
            self.record('finished', attempt=attempt, bytes_sent=0)
            return
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(end-start+1))
        self.send_header('Accept-Ranges', 'bytes')
        self.send_header('ETag', f'"audit-v{served_version}"')
        self.send_header('Last-Modified', 'Mon, 12 Oct 2026 00:00:00 GMT' if served_version == 1 else 'Mon, 12 Oct 2026 01:00:00 GMT')
        if status == 206:
            announced_start = start+1 if path == '/wrong-range.bin' else start
            self.send_header('Content-Range', f'bytes {announced_start}-{end}/{SIZE}')
        self.end_headers()
        sent = 0
        outcome = 'finished'
        try:
            if body:
                position = start
                while position <= end:
                    count = min(len(BLOCK), end-position+1)
                    data_position = position+1 if status == 206 and path == '/wrong-range.bin' else position
                    data = (BLOCK*2)[data_position % len(BLOCK):data_position % len(BLOCK)+count]
                    if served_version == 2:
                        data = data.translate(bytes(reversed(range(256))))
                    self.wfile.write(data)
                    self.wfile.flush()
                    sent += count
                    position += count
                    time.sleep(args.delay)
        except (ConnectionError, OSError):
            outcome = 'disconnected'
        finally:
            self.record(outcome, attempt=attempt, bytes_sent=sent)

server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
server.daemon_threads = True
info = {'port': server.server_port, 'size': SIZE, 'delay': args.delay}
(args.output/'server.json').write_text(json.dumps(info)+'\n')
print(json.dumps(info), flush=True)
try:
    server.serve_forever()
except KeyboardInterrupt:
    pass
finally:
    server.server_close()
