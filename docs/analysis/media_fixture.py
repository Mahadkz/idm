"""Serve local clear-media fixtures for inspecting the original application.

No extraction, DRM, or downloader functionality is implemented here.
"""
import argparse
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('assets', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--port', type=int, default=0)
args = parser.parse_args()
args.assets = args.assets.resolve()
args.output.mkdir(parents=True, exist_ok=True)
lock = threading.Lock()
mime = {'.mp4':'video/mp4', '.m3u8':'application/vnd.apple.mpegurl',
        '.mpd':'application/dash+xml', '.ts':'video/mp2t', '.m4s':'video/iso.segment',
        '.html':'text/html'}

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *unused):
        pass

    def do_GET(self):
        self.respond(True)

    def do_HEAD(self):
        self.respond(False)

    def respond(self, body):
        path = unquote(urlsplit(self.path).path)
        protected = path.startswith('/auth/')
        if protected:
            path = path.removeprefix('/auth')
        authorized = not protected or 'audit_cookie=local_fixture' in self.headers.get('Cookie','')
        if path == '/':
            path = '/player.html'
        file = (args.assets / path.lstrip('/')).resolve()
        if not file.is_relative_to(args.assets) or not file.is_file():
            self.send_error(404)
            return
        status = 200 if authorized else 403
        size = file.stat().st_size
        start, end = 0, size-1
        value = self.headers.get('Range')
        if authorized and value:
            match = re.fullmatch(r'bytes=(\d+)-(\d*)', value)
            if not match or int(match[1]) >= size:
                status = 416
            else:
                start = int(match[1])
                end = min(int(match[2]) if match[2] else size-1,size-1)
                status = 206 if start<=end else 416
        with lock, (args.output/'requests.jsonl').open('a') as f:
            f.write(json.dumps({'time':time.time(),'method':self.command,'url':self.path,
                                'headers':dict(self.headers),'status':status})+'\n')
        self.send_response(status)
        if status in [403,416]:
            self.send_header('Content-Length','0')
            if status == 416:
                self.send_header('Content-Range',f'bytes */{size}')
            self.end_headers()
            return
        self.send_header('Content-Type',mime.get(file.suffix,'application/octet-stream'))
        self.send_header('Content-Length',str(end-start+1))
        self.send_header('Accept-Ranges','bytes')
        self.send_header('ETag',f'"audit-{file.name}-{size}"')
        if status == 206:
            self.send_header('Content-Range',f'bytes {start}-{end}/{size}')
        self.end_headers()
        if body:
            try:
                with file.open('rb') as f:
                    f.seek(start)
                    remaining=end-start+1
                    while remaining:
                        chunk=f.read(min(32768,remaining))
                        self.wfile.write(chunk)
                        remaining-=len(chunk)
            except (ConnectionError,OSError):
                pass

server=ThreadingHTTPServer(('127.0.0.1',args.port),Handler)
(args.output/'server.json').write_text(json.dumps({'port':server.server_port,'assets':str(args.assets)}))
print(json.dumps({'port':server.server_port}),flush=True)
try:
    server.serve_forever()
except KeyboardInterrupt:
    server.server_close()
