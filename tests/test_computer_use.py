"""Real node + shipped bridge; scripted native backend, never the user's desktop."""
import base64
from concurrent.futures import ThreadPoolExecutor
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import unittest
import uuid

import test_node_peers as peers
from test_remote_desktops import DesktopServer
from workspace_web_harness import CORE, NODE, Harness, free_port, sockets_allowed, until

PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6Bz8AAAAASUVORK5CYII='
BRIDGE = Path(__file__).resolve().parents[1] / 'addons/desktop-bridge/bin/agent-desktop-bridge'


@unittest.skipUnless(os.name == 'posix' and NODE.exists() and sockets_allowed(), 'requires built node and real sockets')
class ComputerUseTests(unittest.TestCase):
    cli, http, start = peers.PeerTests.cli, peers.PeerTests.http, peers.PeerTests.start
    close, stop = peers.PeerTests.close, peers.PeerTests.stop

    def setUp(self):
        peers.PeerTests.setUp(self)
        self.actions, self.bad_image = [], False
        owner = self

        class Native(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                action = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                owner.actions.append(action)
                if action.get('text') == 'ambiguous-input':
                    self.close_connection = True
                    return
                value = {'result': 'ok'}
                if action['action'] in ('screenshot', 'zoom'):
                    value = {'data': base64.b64encode(b'not-png').decode() if owner.bad_image else PNG}
                data = json.dumps(value).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.native = ThreadingHTTPServer(('127.0.0.1', 0), Native)
        self.addCleanup(self.native.server_close)
        self.addCleanup(self.native.shutdown)
        threading.Thread(target=self.native.serve_forever, daemon=True).start()
        self.bridge_port = free_port()
        native_port = self.native.server_address[1]
        self.bridge_env = {**os.environ, 'HOME': str(self.root), 'AGENT_WORKSPACE_CONFIG_ROOT': str(self.root),
                           'AGENT_DESKTOP_BRIDGE_PORT': str(self.bridge_port), 'PIXELFLUX_CU': str(native_port),
                           'AGENT_DESKTOP_NATIVE_URL': f'http://127.0.0.1:{native_port}/computer-use',
                           'AGENT_DESKTOP_STATE_DIR': str(self.root / 'bridge-state')}
        self.bridge_process = self.start_bridge()

    def start_bridge(self):
        log = (self.root / f'bridge-{len(self.logs)}.log').open('wb')
        self.logs.append(log)
        process = subprocess.Popen([sys.executable, str(BRIDGE), 'serve'], env=self.bridge_env, stdout=log, stderr=log)
        self.processes.append(process)

        def ready():
            with socket.create_connection(('127.0.0.1', self.bridge_port), timeout=.2):
                return True
        until(ready)
        return process

    def enable(self):
        self.cli(self.b, 'revoke', '--node', self.caller)
        self.token_path.unlink()
        self.cli(self.b, 'grant', '--allow-unverified', '--node', self.caller, '--output-token', self.token_path, '--computer', '--desktop')
        self.token = self.token_path.read_text().strip()
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.port}', '--credential-file', self.token_path)
        self.configure()

    def configure(self):
        self.cli(self.b, 'computer', '--address', f'127.0.0.1:{self.bridge_port}')

    def request(self, operation, **fields):
        return self.http('POST', f'/nodes/{self.target}/computer', {'operation': operation, **fields})

    def acquire(self, ttl=60):
        status, value = self.request('acquire', ttl_secs=ttl)
        self.assertEqual(status, 200, value)
        return value['result']['lease']

    def owner(self, action):
        return self.cli(self.b, 'computer-owner', '--command', json.dumps({'action': action}))

    def test_legacy_grants_do_not_gain_computer_use(self):
        self.configure(); self.start()
        self.assertFalse(self.http('GET', '/nodes')[1]['nodes'][0]['computer'])
        self.assertEqual(self.request('status')[0], 403)
        self.assertFalse(self.cli(self.b, 'identity')['grants'][0]['permissions']['computer'])

    def test_opt_in_and_loopback_provider_required(self):
        self.enable(); self.cli(self.b, 'computer'); self.start()
        self.assertFalse(self.http('GET', '/nodes')[1]['nodes'][0]['computer'])
        self.assertEqual(self.request('status')[0], 409)
        self.assertNotEqual(self.cli(self.b, 'computer', '--address', '192.0.2.1:8765', check=False).returncode, 0)

    def test_screenshot_and_caller_owned_input_lease(self):
        self.enable(); self.start()
        status, value = self.request('status')
        self.assertEqual(status, 200)
        self.assertNotIn('bridge_url', value['result'])
        self.assertNotIn('native_url', value['result'])
        self.assertEqual(self.request('action', action={'action': 'screenshot'})[1]['result']['data'], PNG)
        action = {'action': 'type', 'text': 'hello'}
        self.assertEqual(self.request('action', action=action)[0], 409)
        lease = self.acquire()
        self.assertEqual(self.request('action', lease=str(uuid.uuid4()), action=action)[0], 409)
        self.assertEqual(self.request('action', lease=lease, action=action)[0], 200)
        self.assertEqual(self.request('renew', lease=lease, ttl_secs=30)[0], 200)
        self.assertEqual(self.request('release', lease=lease)[0], 200)
        self.assertEqual(self.request('action', lease=lease, action=action)[0], 409)
        self.assertEqual(sum(a.get('text') == 'hello' for a in self.actions), 1)
        self.assertEqual(self.request('acquire', ttl_secs=301)[0], 400)

    def test_expiry_reconfiguration_and_revocation_release_control(self):
        self.enable(); self.start()
        old = self.acquire(1)
        until(lambda: self.request('status')[1]['result']['control_busy'] is False)
        self.assertEqual(self.request('renew', lease=old)[0], 409)
        old = self.acquire(); self.configure()
        self.assertEqual(self.request('renew', lease=old)[0], 409)
        self.acquire(); self.cli(self.b, 'revoke', '--node', self.caller)
        self.assertEqual(self.request('status')[0], 401)
        token = self.root / 'other.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', 'other-node', '--output-token', token, '--computer')
        self.token = token.read_text().strip()
        until(lambda: self.request('status')[1]['result']['control_busy'] is False)
        self.acquire()

    def test_lease_cannot_be_used_by_another_granted_caller(self):
        self.enable(); self.start()
        lease = self.acquire()
        token = self.root / 'other.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', 'other-node', '--output-token', token, '--computer')
        self.token = token.read_text().strip()
        for operation in ('renew', 'release'):
            self.assertEqual(self.request(operation, lease=lease)[0], 409)
        self.assertEqual(self.request('action', lease=lease, action={'action': 'key', 'text': 'Return'})[0], 409)

    def test_emergency_stop_survives_node_and_bridge_restart(self):
        self.enable(); node = self.start()
        lease = self.acquire()
        self.assertTrue(self.request('emergency_stop')[1]['result']['paused'])
        self.assertTrue(self.cli(self.b, 'identity')['computer_paused'])
        self.stop(node); self.stop(self.bridge_process)
        self.start_bridge(); self.start()
        self.assertEqual(self.request('acquire')[0], 409)
        self.assertEqual(self.request('renew', lease=lease)[0], 409)
        self.assertFalse(self.owner('resume')['paused'])
        self.acquire()

    def test_emergency_stop_fences_node_even_if_bridge_is_down(self):
        self.enable(); self.start(); self.acquire(); self.stop(self.bridge_process)
        self.assertTrue(self.request('emergency_stop')[1]['result']['paused'])
        self.start_bridge()
        self.assertEqual(self.request('acquire')[0], 409)
        self.owner('resume'); self.acquire()

    def test_no_automatic_replay_after_ambiguous_input(self):
        self.enable(); self.start()
        lease = self.acquire()
        self.assertEqual(self.request('action', lease=lease, action={'action': 'type', 'text': 'ambiguous-input'})[0], 502)
        self.assertEqual(sum(a.get('text') == 'ambiguous-input' for a in self.actions), 1)

    def test_screenshot_validation_and_bound_mcp_image_content(self):
        self.enable(); self.start()
        requests = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': 'device_computer', 'arguments': {'operation': 'action', 'action': {'action': 'screenshot'}}}},
        ]
        result = self.cli(self.a, 'call', '--node', self.target, 'computer-mcp', input=''.join(json.dumps(r) + '\n' for r in requests))
        self.assertEqual([t['name'] for t in result[0]['result']['tools']], ['device_computer'])
        content = result[1]['result']['content']
        self.assertEqual(json.loads(content[0]['text'])['node'], self.target)
        self.assertEqual(content[1], {'type': 'image', 'mimeType': 'image/png', 'data': PNG})
        self.bad_image = True
        self.assertEqual(self.request('action', action={'action': 'screenshot'})[0], 502)
        self.cli(self.b, 'revoke', '--node', self.caller)
        result = self.cli(self.a, 'call', '--node', self.target, 'computer-mcp', input=json.dumps(requests[0]) + '\n')
        self.assertEqual(result[0]['result']['tools'], [])

    def test_origin_target_and_unsafe_actions_rejected(self):
        self.enable(); self.start()
        for headers, expected in [({'Origin': 'https://example.com'}, 403), ({'X-Cranibox-Target': 'other-node'}, 409)]:
            self.assertEqual(self.http('POST', f'/nodes/{self.target}/computer', {'operation': 'status'}, headers)[0], expected)
        for command in [{'operation': 'resume'}, {'operation': 'action', 'action': {'action': 'hold_key', 'text': 'ctrl'}}, {'operation': 'status', 'url': 'http://other'}]:
            self.assertNotEqual(self.cli(self.a, 'call', '--node', self.target, 'computer', '--command', json.dumps(command), check=False).returncode, 0)

    def test_desktop_viewer_and_computer_lease_share_controller_slot(self):
        self.enable(); self.start()
        desktop = DesktopServer(); self.addCleanup(desktop.close)
        self.cli(self.b, 'desktop', '--address', desktop.address)
        lease = self.acquire()

        def viewer():
            connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
            headers = {'Authorization': 'Bearer ' + self.token, 'X-Cranibox-Target': self.target,
                       'Connection': 'Upgrade', 'Upgrade': 'websocket', 'Sec-WebSocket-Version': '13',
                       'Sec-WebSocket-Key': base64.b64encode(os.urandom(16)).decode()}
            connection.request('GET', f'/api/v1/mesh/nodes/{self.target}/desktop/ws', headers=headers)
            return connection, connection.getresponse()
        connection, response = viewer()
        self.assertEqual(response.status, 409); connection.close()
        self.request('release', lease=lease)
        connection, response = viewer(); self.addCleanup(connection.close)
        self.assertEqual(response.status, 101)
        self.assertEqual(self.request('acquire')[0], 409)


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE)) and sockets_allowed(), 'requires built server and real sockets')
class ComputerUseOwnerWebTests(unittest.TestCase):
    def test_owner_emergency_stop_does_not_wait_for_slow_provider_status(self):
        h = Harness(unified=True)
        self.addCleanup(h.close)
        entered, release = threading.Event(), threading.Event()

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                command = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if command['action'] == 'status':
                    entered.set()
                    release.wait(5)
                data = json.dumps({'paused': command['action'] == 'emergency_stop', 'backend_available': True}).encode()
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        self.addCleanup(provider.server_close)
        self.addCleanup(provider.shutdown)
        self.addCleanup(release.set)
        threading.Thread(target=provider.serve_forever, daemon=True).start()
        self.assertEqual(h.post('/api/peers', {'operation': 'computer_sharing', 'computer': {'address': f'127.0.0.1:{provider.server_port}'}})[0], 200)
        with ThreadPoolExecutor(max_workers=2) as pool:
            pending = pool.submit(h.post, '/api/peers', {'operation': 'computer_owner', 'command': {'action': 'status'}})
            try:
                self.assertTrue(entered.wait(3))
                stopped = pool.submit(h.post, '/api/peers', {'operation': 'computer_owner', 'command': {'action': 'emergency_stop'}}).result(timeout=2)
                self.assertEqual(stopped[0], 200, stopped[1])
                self.assertTrue(stopped[1]['paused'])
                self.assertTrue(h.post('/api/peers', {'operation': 'list'})[1]['computer_paused'])
            finally:
                release.set()
            self.assertEqual(pending.result(timeout=3)[0], 200)


if __name__ == '__main__':
    unittest.main()
