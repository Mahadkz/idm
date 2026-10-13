"""Logging forward HTTP proxy and SOCKS5 proxy (optional user/password) for engine routing tests."""
import argparse
import http.client
import json
import socket
import socketserver
import struct
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument('log', type=Path)
parser.add_argument('--user', default='user')
parser.add_argument('--password', default='secret')
args = parser.parse_args()
lock = threading.Lock()


def record(**fields):
    with lock, args.log.open('a') as file:
        file.write(json.dumps(fields) + '\n')


class Forward(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *unused):
        pass

    def do_GET(self):
        target = urlsplit(self.path)
        record(kind='http', url=self.path, range=self.headers.get('Range'))
        headers = {k: v for k, v in self.headers.items() if k.lower() not in ['proxy-connection', 'connection']}
        upstream = http.client.HTTPConnection(target.hostname, target.port, timeout=30)
        upstream.request('GET', target.path + (f'?{target.query}' if target.query else ''), headers=headers)
        response = upstream.getresponse()
        self.send_response(response.status)
        for name, value in response.getheaders():
            if name.lower() not in ['connection', 'transfer-encoding']:
                self.send_header(name, value)
        self.end_headers()
        try:
            while chunk := response.read(64 * 1024):
                self.wfile.write(chunk)
        except (ConnectionError, OSError):
            pass
        upstream.close()


def exact(sock, count):
    data = b''
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise ConnectionError('closed')
        data += chunk
    return data


class Socks(socketserver.BaseRequestHandler):
    def handle(self):
        client = self.request
        version, count = exact(client, 2)
        methods = exact(client, count)
        if version != 5 or 2 not in methods:
            client.sendall(b'\x05\xff')
            return
        client.sendall(b'\x05\x02')
        exact(client, 1)
        user = exact(client, exact(client, 1)[0]).decode()
        password = exact(client, exact(client, 1)[0]).decode()
        if (user, password) != (args.user, args.password):
            client.sendall(b'\x01\x01')
            return
        client.sendall(b'\x01\x00')
        _, command, _, kind = exact(client, 4)
        if kind == 3:
            host = exact(client, exact(client, 1)[0]).decode()
        elif kind == 1:
            host = socket.inet_ntoa(exact(client, 4))
        else:
            client.sendall(b'\x05\x08\x00\x01' + bytes(6))
            return
        port = struct.unpack('!H', exact(client, 2))[0]
        record(kind='socks', host=host, address_type='domain' if kind == 3 else 'ipv4', port=port)
        upstream = socket.create_connection((host, port), timeout=30)
        client.sendall(b'\x05\x00\x00\x01' + bytes(6))

        def pipe(source, sink):
            try:
                while data := source.recv(65536):
                    sink.sendall(data)
            except OSError:
                pass
            finally:
                try:
                    sink.shutdown(socket.SHUT_WR)
                except OSError:
                    pass

        thread = threading.Thread(target=pipe, args=(upstream, client), daemon=True)
        thread.start()
        pipe(client, upstream)
        thread.join()


class ThreadedTCP(socketserver.ThreadingMixIn, socketserver.TCPServer):
    daemon_threads = True
    allow_reuse_address = True


forward = ThreadingHTTPServer(('127.0.0.1', 0), Forward)
forward.daemon_threads = True
socks = ThreadedTCP(('127.0.0.1', 0), Socks)
for server in [forward, socks]:
    threading.Thread(target=server.serve_forever, daemon=True).start()
print(json.dumps({'http': forward.server_port, 'socks': socks.server_address[1]}), flush=True)
threading.Event().wait()
