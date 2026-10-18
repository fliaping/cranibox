"""Real nodes, HTTP/CDP WebSockets and process-scoped proxy; scripted browser only."""
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import select
import signal
import socket
import struct
import subprocess
import threading
import unittest
from urllib.parse import urlsplit

import test_node_peers as peers
from test_cranibox_terminal import Socket
from workspace_web_harness import NODE, sockets_allowed, until


class BrowserSocket(Socket):
    def __init__(self, port, path, headers=None):
        self.socket = socket.create_connection(('127.0.0.1', port), timeout=5)
        self.socket.settimeout(5)
        self.reader = self.socket.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        fields = {'Host': f'127.0.0.1:{port}', 'Upgrade': 'websocket', 'Connection': 'Upgrade',
                  'Sec-WebSocket-Version': '13', 'Sec-WebSocket-Key': key, **(headers or {})}
        self.socket.sendall((f'GET {path} HTTP/1.1\r\n' + ''.join(f'{k}: {v}\r\n' for k, v in fields.items()) + '\r\n').encode())
        self.status = int(self.reader.readline().split()[1])
        while line := self.reader.readline():
            if line == b'\r\n':
                break


@unittest.skipUnless(os.name == 'posix' and NODE.exists() and sockets_allowed(), 'requires built nodes and real sockets')
class BrowserUseTests(unittest.TestCase):
    cli, http, start = peers.PeerTests.cli, peers.PeerTests.http, peers.PeerTests.start
    close, stop = peers.PeerTests.close, peers.PeerTests.stop

    def setUp(self):
        peers.PeerTests.setUp(self)
        self.mode, self.commands = 'normal', []
        self.probing, self.release = threading.Event(), threading.Event()
        owner = self

        class CDP(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'
            def log_message(self, *args):
                pass

            def do_GET(self):
                if self.path == '/json/version':
                    owner.probing.set()
                    if owner.mode == 'slow':
                        owner.release.wait(4)
                    if owner.mode == 'redirect':
                        self.send_response(302); self.send_header('Location', 'http://192.0.2.1/secret'); self.send_header('Content-Length', '0'); self.end_headers()
                        return
                    endpoint = '/devtools/page/not-a-browser' if owner.mode == 'invalid' else '/devtools/browser/test-browser'
                    data = json.dumps({'Browser': 'TestChromium/1', 'webSocketDebuggerUrl': f'ws://192.0.2.1:1{endpoint}'}).encode()
                    if owner.mode == 'oversize':
                        data = b' ' * (64 * 1024 + 1)
                    self.send_response(200); self.send_header('Content-Length', str(len(data))); self.end_headers()
                    try:
                        self.wfile.write(data)
                    except OSError:
                        pass
                    return
                if self.path != '/devtools/browser/test-browser':
                    self.send_error(404); return
                key = self.headers['Sec-WebSocket-Key']
                accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                self.send_response(101); self.send_header('Upgrade', 'websocket'); self.send_header('Connection', 'Upgrade')
                self.send_header('Sec-WebSocket-Accept', accept); self.end_headers()
                self.connection.settimeout(6)
                try:
                    while prefix := self.rfile.read(2):
                        opcode, size = prefix[0] & 15, prefix[1] & 127
                        if size == 126:
                            size = struct.unpack('!H', self.rfile.read(2))[0]
                        elif size == 127:
                            size = struct.unpack('!Q', self.rfile.read(8))[0]
                        mask = self.rfile.read(4) if prefix[1] & 128 else b''
                        data = self.rfile.read(size)
                        if mask:
                            data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
                        if opcode == 8:
                            break
                        if opcode == 9:
                            self.wfile.write(bytes([0x8a, len(data)]) + data); self.wfile.flush(); continue
                        if opcode != 1:
                            continue
                        command = json.loads(data); owner.commands.append(command)
                        data = json.dumps({'id': command['id'], 'result': {'method': command['method']}}).encode()
                        self.wfile.write(bytes([0x81, len(data)]) + data); self.wfile.flush()
                except (OSError, ValueError):
                    pass

        self.cdp = ThreadingHTTPServer(('127.0.0.1', 0), CDP)
        self.addCleanup(self.cdp.server_close); self.addCleanup(self.cdp.shutdown)
        self.addCleanup(self.release.set)
        threading.Thread(target=self.cdp.serve_forever, daemon=True).start()

    def configure(self):
        self.cli(self.b, 'browser', '--address', f'127.0.0.1:{self.cdp.server_port}')

    def enable(self):
        self.cli(self.b, 'revoke', '--node', self.caller); self.token_path.unlink()
        self.cli(self.b, 'grant', '--node', self.caller, '--output-token', self.token_path, '--browser')
        self.token = self.token_path.read_text().strip()
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.port}', '--credential-file', self.token_path)
        self.configure()

    def request(self, operation='status', **fields):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=6)
        try:
            connection.request('POST', f'/api/v1/mesh/nodes/{self.target}/browser', json.dumps({'operation': operation, **fields}),
                               {'Authorization': f'Bearer {self.token}', 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'})
            response = connection.getresponse(); data = response.read()
            return response.status, json.loads(data) if response.getheader('Content-Type', '').startswith('application/json') else data.decode()
        finally:
            connection.close()

    def owner(self, action):
        return self.cli(self.b, 'browser-owner', '--command', json.dumps({'action': action}))

    def connect(self, **headers):
        ws = BrowserSocket(self.port, f'/api/v1/mesh/nodes/{self.target}/browser/ws',
                           {'Authorization': f'Bearer {self.token}', 'X-Cranibox-Target': self.target, **headers})
        self.addCleanup(ws.close)
        return ws

    def test_legacy_grants_and_disabled_provider(self):
        self.configure(); self.start()
        self.assertFalse(self.http('GET', '/nodes')[1]['nodes'][0]['browser'])
        self.assertEqual(self.request()[0], 403)
        self.assertEqual(self.connect().status, 403)
        self.assertFalse(self.cli(self.b, 'identity')['grants'][0]['permissions']['browser'])
        self.enable(); self.cli(self.b, 'browser')
        self.assertEqual(self.request()[0], 409)

    def test_loopback_validation_and_no_redirect_or_oversize_metadata(self):
        for address in ['192.0.2.1:9222', '0.0.0.0:9222', '127.0.0.1:0']:
            self.assertNotEqual(self.cli(self.b, 'browser', '--address', address, check=False).returncode, 0)
        self.enable(); self.start()
        for self.mode in ['redirect', 'invalid', 'oversize']:
            status, value = self.request()
            self.assertEqual(status, 200, value)
            self.assertFalse(value['result']['backend_available'])
            self.assertEqual(self.connect().status, 502)

    def test_cdp_frames_metadata_sanitization_and_single_controller(self):
        self.enable(); self.start()
        status, value = self.request()
        self.assertEqual(status, 200, value)
        self.assertEqual(value['result']['browser'], {'version': 'TestChromium/1'})
        self.assertNotIn('192.0.2.1', json.dumps(value))
        ws = self.connect(); self.assertEqual(ws.status, 101)
        for i in range(3):
            ws.send({'id': i, 'method': 'Browser.getVersion'})
            self.assertEqual(json.loads(ws.recv()[1]), {'id': i, 'result': {'method': 'Browser.getVersion'}})
        self.assertTrue(self.request()[1]['result']['control_busy'])
        self.assertEqual(self.connect().status, 409)
        self.assertEqual(len(self.commands), 3)

    def test_emergency_stop_persists_across_restart_and_only_owner_resumes(self):
        self.enable(); process = self.start(); ws = self.connect()
        self.assertEqual(ws.status, 101)
        self.assertTrue(self.request('emergency_stop')[1]['result']['paused'])
        self.assertEqual(ws.recv()[0], 8)
        self.assertEqual(self.connect().status, 409)
        self.assertEqual(self.request('resume')[0], 422)
        self.stop(process); self.start()
        self.assertTrue(self.request()[1]['result']['paused'])
        self.assertFalse(self.owner('resume')['paused'])
        self.assertEqual(self.connect().status, 101)

    def test_revocation_disconnects_idle_controller(self):
        self.enable(); self.start(); ws = self.connect(); self.assertEqual(ws.status, 101)
        self.cli(self.b, 'revoke', '--node', self.caller)
        self.assertEqual(ws.recv()[0], 8)
        self.assertEqual(self.request()[0], 401)

    def test_reconfiguring_same_provider_disconnects_old_controller(self):
        self.enable(); self.start(); ws = self.connect(); self.assertEqual(ws.status, 101)
        self.configure()
        self.assertEqual(ws.recv()[0], 8)
        until(lambda: not self.request()[1]['result']['control_busy'])
        self.assertEqual(self.connect().status, 101)

    def test_stop_during_slow_probe_does_not_wait_or_lose_pause(self):
        self.enable(); self.start(); self.mode = 'slow'; self.probing.clear()
        with ThreadPoolExecutor() as pool:
            future = pool.submit(self.request)
            self.assertTrue(self.probing.wait(2))
            self.assertTrue(self.owner('emergency_stop')['paused'])
            self.release.set()
            self.assertTrue(future.result(timeout=5)[1]['result']['paused'])
        self.assertEqual(self.connect().status, 409)

    def test_origin_and_target_binding(self):
        self.enable(); self.start()
        self.assertEqual(self.connect(Origin='https://untrusted.example').status, 403)
        self.assertEqual(self.connect(**{'X-Cranibox-Target': 'another-node'}).status, 409)
        self.assertEqual(self.request(extra='not-allowed')[0], 422)

    def test_process_scoped_proxy_has_nonce_and_rejects_browser_origins(self):
        self.enable(); self.start()
        log = (self.root / 'proxy.log').open('wb'); self.logs.append(log)
        process = subprocess.Popen([str(NODE), '--config', str(self.a), 'peer', 'call', '--node', self.target, 'browser-proxy'], stdout=subprocess.PIPE, stderr=log, text=True)
        self.processes.append(process); self.addCleanup(process.stdout.close)
        self.assertTrue(select.select([process.stdout], [], [], 10)[0], 'proxy failed to publish endpoint')
        value = json.loads(process.stdout.readline()); self.assertEqual(value['node'], self.target)
        url = urlsplit(value['websocket']); self.assertEqual(url.hostname, '127.0.0.1')
        for headers in [{'Origin': 'http://example.com'}, {'Host': 'attacker.example'}, {'Sec-Fetch-Site': 'cross-site'}]:
            rejected = BrowserSocket(url.port, url.path, headers); self.addCleanup(rejected.close)
            self.assertEqual(rejected.status, 403)
        missing = BrowserSocket(url.port, '/wrong-path'); self.addCleanup(missing.close); self.assertEqual(missing.status, 404)
        ws = BrowserSocket(url.port, url.path); self.addCleanup(ws.close); self.assertEqual(ws.status, 101)
        ws.send({'id': 42, 'method': 'Browser.getVersion'})
        self.assertEqual(json.loads(ws.recv()[1])['id'], 42)
        process.send_signal(signal.SIGINT); process.wait(timeout=6)
        self.assertEqual(process.returncode, 0)
        self.assertEqual(ws.recv()[0], 8)
        with self.assertRaises(OSError):
            socket.create_connection(('127.0.0.1', url.port), timeout=.5)
