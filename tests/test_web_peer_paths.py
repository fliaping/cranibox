"""Real path selection, authorization and PTY recovery without operation replay."""
import http.client
import http.server
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import subprocess
import sys
import threading
import time
import unittest

import test_web_direct_peers as direct
from test_cranibox_terminal import Socket
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


class Relay:
    """A disposable TCP path; stopping it also disconnects attached WebSockets."""
    def __init__(self, target, port=0):
        self.connections = set()
        self.lock = threading.Lock()
        self.stopped = False
        owner = self

        class Handler(socketserver.BaseRequestHandler):
            def handle(self):
                upstream = None
                try:
                    upstream = socket.create_connection(('127.0.0.1', target), timeout=2)
                    self.request.settimeout(2)
                    with owner.lock:
                        owner.connections.update((self.request, upstream))
                    while not owner.stopped:
                        readable, _, _ = select.select([self.request, upstream], [], [], .1)
                        for source in readable:
                            data = source.recv(65536)
                            if not data:
                                return
                            (upstream if source is self.request else self.request).sendall(data)
                except OSError:
                    pass
                finally:
                    with owner.lock:
                        owner.connections.discard(self.request)
                        owner.connections.discard(upstream)
                    if upstream:
                        upstream.close()

        class Server(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True

        self.server = Server(('127.0.0.1', port), Handler)
        self.port = self.server.server_address[1]
        self.url = f'http://127.0.0.1:{self.port}'
        self.thread = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=.05), daemon=True)
        self.thread.start()

    def close(self):
        if self.stopped:
            return
        self.stopped = True
        self.server.shutdown()
        self.server.server_close()
        with self.lock:
            for connection in self.connections:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        self.thread.join(timeout=2)


class HttpPath:
    def __init__(self, target, legacy=False):
        self.mode = 'forward'
        self.calls = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.forward()

            def do_POST(self):
                self.forward()

            def forward(self):
                if legacy and self.path.endswith('/api/v1/mesh/keys'):
                    # Method-based fault injection needs plaintext requests, so this path
                    # poses as an older target without end-to-end keys (bearer fallback).
                    self.send_response(404)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                owner.calls.append((self.command, self.path))
                failures = {
                    'denied': (401, b'{"error":"grant denied"}'),
                    'wrong_identity': (200, b'{"nodes":[{"id":"another-device"}]}'),
                    'unavailable': (503, b'{"error":"temporarily unavailable"}'),
                    'malformed': (200, b'not-json'),
                }
                if owner.mode in failures:
                    status, body = failures[owner.mode]
                else:
                    body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
                    connection = http.client.HTTPConnection('127.0.0.1', target, timeout=4)
                    try:
                        connection.request(self.command, self.path, body, {
                            key: value for key, value in self.headers.items()
                            if key.lower() in ('authorization', 'x-cranibox-target', 'content-type', 'x-cranibox-session')
                        })
                        response = connection.getresponse()
                        status, body = response.status, response.read()
                    finally:
                        connection.close()
                    if owner.mode == 'drop_response' and self.command == 'POST':
                        self.close_connection = True
                        return
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'
        self.thread = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=.05), daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class WebPeerPathsTests(unittest.TestCase):
    setUp = direct.WebDirectPeersTests.setUp
    api = direct.WebDirectPeersTests.api
    cli = direct.WebDirectPeersTests.cli
    execute = direct.WebDirectPeersTests.execute

    def add_path(self, url):
        return self.api(operation='add_path', node=self.target, url=url, token=self.token)

    def test_live_path_loss_reattaches_same_shell_and_prefers_recovered_path(self):
        self.h.stop(self.h.server)
        self.cli('revoke', '--node', self.caller)
        credential = self.h.root / 'paths.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', credential, '--exec', '--terminals')
        self.token = credential.read_text().strip()
        primary = Relay(self.port)
        self.addCleanup(primary.close)
        self.api(operation='connect', node=self.target, url=primary.url, token=self.token)
        catalog = self.add_path(self.url)
        self.assertEqual(len(catalog['routes'][0]['paths']), 2)
        probe = self.api(operation='probe', node=self.target)
        self.assertEqual(probe['connection'], {'url': primary.url, 'fallback': False, 'path_count': 2})
        shell = self.api(operation='sessions', node=self.target, command={
            'operation': 'create', 'name': 'Path recovery', 'cwd': str(self.h.root),
        })['session']

        def attach():
            status, ticket = self.h.post('/api/terminal/ticket', {
                'node': self.target, 'direct': True, 'session_id': shell['id'],
                'cols': 80, 'rows': 24, 'takeover': True,
            })
            self.assertEqual(status, 200, ticket)
            stream = Socket(self.h, ticket['ticket'])
            self.addCleanup(stream.close)
            self.assertEqual(stream.status, 101)
            stream.expect('ready')
            return stream

        first = attach()
        first.command("export PATH_RECOVERY=preserved; echo $$ > paths-shell.pid; printf '%s%s\\n' PATH _READY")
        first.expect('PATH_READY')
        pid = (self.h.root / 'paths-shell.pid').read_text().strip()
        primary.close()
        first.close()
        probe = self.api(operation='probe', node=self.target)
        self.assertEqual(probe['connection']['url'], self.url)
        self.assertTrue(probe['connection']['fallback'])
        second = attach()
        second.command("printf '%s:%s:%s\\n' RESTORED \"$PATH_RECOVERY\" \"$$\"")
        second.expect('RESTORED:preserved:' + pid)
        second.close()
        recovered = Relay(self.port, primary.port)
        self.addCleanup(recovered.close)
        self.assertEqual(self.api(operation='probe', node=self.target)['connection']['url'], primary.url)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(len(self.api(operation='list')['routes'][0]['paths']), 2)
        self.assertTrue(Path('/proc/' + pid).exists())

    def test_lost_exec_response_never_replays_on_an_alternative(self):
        primary = HttpPath(self.port, legacy=True)
        self.addCleanup(primary.close)
        self.api(operation='connect', node=self.target, url=primary.url, token=self.token)
        self.add_path(self.url)
        marker = self.h.root / 'exec-count'
        primary.mode = 'drop_response'
        status, _ = self.h.post('/api/peers', {'operation': 'exec', 'node': self.target, 'request': {
            'program': sys.executable, 'args': ['-c', f'with open({str(marker)!r}, "a") as file: file.write("once\\n")'], 'timeout_secs': 5,
        }})
        self.assertEqual(status, 400)
        until(marker.exists)
        time.sleep(.3)
        self.assertEqual(marker.read_text(), 'once\n')
        self.assertEqual(sum(method == 'POST' for method, _ in primary.calls), 1)

    def test_authorization_identity_and_protocol_errors_do_not_select_other_paths(self):
        primary = HttpPath(self.port)
        self.addCleanup(primary.close)
        self.api(operation='connect', node=self.target, url=primary.url, token=self.token)
        self.add_path(self.url)
        marker = self.h.root / 'forbidden-exec'
        for mode in ('denied', 'wrong_identity', 'malformed'):
            primary.mode = mode
            status, _ = self.h.post('/api/peers', {'operation': 'exec', 'node': self.target, 'request': {
                'program': sys.executable, 'args': ['-c', f'open({str(marker)!r}, "w").close()'], 'timeout_secs': 5,
            }})
            self.assertEqual(status, 400, mode)
            self.assertFalse(marker.exists(), mode)
        self.assertFalse(any(method == 'POST' for method, _ in primary.calls))
        primary.mode = 'unavailable'
        self.assertEqual(self.api(operation='probe', node=self.target)['connection']['url'], self.url)

    def test_path_changes_preserve_grants_and_failed_additions_preserve_config(self):
        primary = Relay(self.port)
        self.addCleanup(primary.close)
        before = self.path.read_bytes()
        for url, token in [(primary.url, 'f' * 64), (self.url, self.token), ('http://127.0.0.1:1', self.token)]:
            status, _ = self.h.post('/api/peers', {'operation': 'add_path', 'node': self.target, 'url': url, 'token': token})
            self.assertEqual(status, 400)
            self.assertEqual(self.path.read_bytes(), before)
        def paths(*args):
            result = subprocess.run([
                str(NODE), '--config', str(self.path.parent / 'settings.json'),
                'peer', 'paths', *map(str, args),
            ], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)

        paths('add', '--node', self.target, '--url', primary.url, '--credential-file', self.token_path)
        view = paths('prefer', '--node', self.target, '--url', primary.url)
        self.assertEqual(view['routes'][0]['url'], primary.url)
        paths('remove', '--node', self.target, '--url', self.url)
        status, _ = self.h.post('/api/peers', {'operation': 'remove_path', 'node': self.target, 'url': primary.url})
        self.assertEqual(status, 400)
        self.assertTrue(self.api(operation='probe', node=self.target)['nodes'][0]['exec'])
