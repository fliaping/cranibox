"""Real outbound-only nodes, Rust rendezvous and authenticated browser/native RFB."""
import json
import subprocess
import time
import unittest
from types import SimpleNamespace

from workspace_web_harness import Harness, CORE, NODE, WEB, free_port, sockets_allowed, until
from test_cranibox_terminal import Socket
from test_remote_desktops import DesktopServer


@unittest.skipUnless(all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; sockets required')
class DesktopRelayTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True)
        self.addCleanup(self.h.close)
        self.vnc = DesktopServer()
        self.addCleanup(self.vnc.close)
        self.configuration = self.h.root / 'relay-target/settings.json'
        self.target = self.cli('identity')['node_id']
        self.caller = self.h.post('/api/peers', {'operation': 'identity'})[1]['node_id']
        self.cli('desktop', '--address', self.vnc.address)
        self.grant(self.caller)
        self.grant('test-node')
        self.h.cli('enroll', self.target, '--credentials', self.h.credentials, '--token-file', self.h.root / 'target.token')
        for node in ('test-node', self.target):
            self.h.cli('group', node, '--group', 'personal', '--credentials', self.h.credentials)
        self.start_target()

    def cli(self, *args):
        process = subprocess.run([str(NODE), '--config', str(self.configuration), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(process.returncode, 0, process.stderr)
        return json.loads(process.stdout)

    def grant(self, caller, scope='--desktop'):
        token = self.h.root / (caller + '.grant')
        if token.exists():
            token = self.h.root / f'{caller}-{time.time_ns()}.grant'
        self.cli('grant', '--allow-unverified', '--node', caller, '--output-token', token, scope)
        return token.read_text().strip()

    def start_target(self):
        self.process = self.h.start(NODE, '--config', self.configuration, '--id', self.target,
                                    '--server', f'ws://127.0.0.1:{self.h.gateway_port}',
                                    '--token-file', self.h.root / 'target.token')
        until(lambda: any(n['id'] == self.target and n['online'] and n.get('desktop_relay') for n in self.h.gateway('/api/v1/nodes')['nodes']))

    def ticket(self):
        status, result = self.h.post('/api/desktop/ticket', {'node': self.target, 'relay': True})
        self.assertEqual(status, 200, result)
        return result['ticket']

    def browser(self, ticket=None):
        viewer = Socket(self.h, ticket or self.ticket(), path='/api/desktop/ws')
        self.addCleanup(viewer.close)
        return viewer

    def native(self, token=None, ticket='unused'):
        endpoint = SimpleNamespace(web_port=self.h.gateway_port, cookie='')
        viewer = Socket(endpoint, ticket, origin='omit', path=f'/api/v1/mesh/nodes/{self.target}/desktop/ws',
                        extra_headers={'Authorization': 'Bearer ' + (token or (self.h.root / 'node.token').read_text().strip())})
        self.addCleanup(viewer.close)
        return viewer

    def connected(self, native=False):
        viewer = self.native() if native else self.browser()
        self.assertEqual(viewer.status, 101)
        self.assertEqual(viewer.recv(), (2, self.vnc.banner))
        return viewer

    def denied_ticket(self, text):
        status, result = self.h.post('/api/desktop/ticket', {'node': self.target, 'relay': True})
        self.assertEqual(status, 400, result)
        self.assertIn(text, result['error'])

    def test_outbound_only_binary_transport_and_reconnect(self):
        self.assertIsNone(self.cli('identity')['listener'])
        self.assertEqual(self.h.post('/api/peers', {'operation': 'list'})[1]['routes'], [])
        viewer = self.connected()
        data = bytes(range(256)) * 220
        viewer.send(data, opcode=2)
        result = b''
        while len(result) < len(data):
            opcode, chunk = viewer.recv()
            self.assertEqual(opcode, 2)
            result += chunk
        self.assertEqual(result, data)
        viewer.close()
        until(lambda: self.vnc.active == 0)
        self.connected(native=True)
        self.assertIsNone(self.process.poll())

    def test_enrollment_cannot_replace_target_desktop_grant_or_assert_caller(self):
        self.cli('revoke', '--node', self.caller)
        self.denied_ticket('desktop capability not granted')
        self.grant(self.caller, '--exec')
        self.denied_ticket('desktop capability not granted')
        self.cli('revoke', '--node', self.caller)
        self.grant(self.caller)
        self.cli('revoke', '--node', 'test-node')
        self.assertEqual(self.native(ticket='unused&caller=' + self.caller).status, 409)
        status, _ = self.h.post('/api/desktop/ticket', {'node':self.target, 'relay':True, 'caller':self.caller})
        self.assertEqual(status, 400)
        viewer = self.connected()
        self.cli('revoke', '--node', self.caller)
        start = time.monotonic()
        self.assertEqual(viewer.recv()[0], 8)
        self.assertLess(time.monotonic() - start, 3)
        until(lambda: self.vnc.active == 0)

    def test_native_group_and_credential_revocation_disconnect_idle_viewer(self):
        viewer = self.connected(native=True)
        self.h.cli('group', 'test-node', '--credentials', self.h.credentials)
        self.assertEqual(viewer.recv()[0], 8)
        until(lambda: self.vnc.active == 0)
        self.assertEqual(self.native().status, 409)
        self.h.cli('group', 'test-node', '--group', 'personal', '--credentials', self.h.credentials)
        viewer = self.connected(native=True)
        self.h.cli('enroll', self.target, '--credentials', self.h.credentials, '--token-file', self.h.root / 'rotated.token')
        start = time.monotonic()
        self.assertEqual(viewer.recv()[0], 8)
        self.assertLess(time.monotonic() - start, 3)
        until(lambda: self.vnc.active == 0)

    def test_one_controller_shared_by_direct_and_relay_routes(self):
        self.h.stop(self.process)
        port = free_port()
        self.cli('listen', '--address', f'127.0.0.1:{port}')
        self.start_target()
        token = (self.h.root / (self.caller + '.grant')).read_text().strip()
        status, result = self.h.post('/api/peers', {'operation':'connect', 'node':self.target, 'url':f'http://127.0.0.1:{port}', 'token':token})
        self.assertEqual(status, 200, result)
        viewer = self.connected()
        self.assertEqual(self.native().status, 409)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node': self.target})[0], 400)
        self.assertEqual(self.vnc.active, 1)
        viewer.close()
        until(lambda: self.vnc.active == 0)
        status, ticket = self.h.post('/api/desktop/ticket', {'node':self.target})
        self.assertEqual(status, 200, ticket)
        direct = self.browser(ticket['ticket'])
        self.assertEqual(direct.recv(), (2, self.vnc.banner))
        self.denied_ticket('active controller')

    def test_target_disconnect_and_restart_never_replays_viewer(self):
        viewer = self.connected()
        self.h.stop(self.process)
        self.assertEqual(viewer.recv()[0], 8)
        until(lambda: self.vnc.active == 0)
        self.start_target()
        self.assertEqual(self.vnc.active, 0)
        self.connected()

    def test_unused_browser_ticket_expires_and_releases_target_lease(self):
        ticket = self.ticket()
        until(lambda: self.vnc.active == 1)
        until(lambda: self.vnc.active == 0, seconds=34)
        self.assertEqual(self.browser(ticket).status, 401)
        self.connected()

    def test_pending_ticket_cancellation_is_session_bound_and_never_ends_an_active_viewer(self):
        ticket = self.ticket()
        until(lambda: self.vnc.active == 1)
        owner = self.h.cookie, self.h.csrf
        self.h.login()
        status, result = self.h.post('/api/desktop/cancel', {'ticket': ticket})
        self.assertEqual((status, result), (200, {'cancelled': False}))
        self.assertEqual(self.vnc.active, 1)
        self.h.cookie, self.h.csrf = owner
        self.assertEqual(self.h.post('/api/desktop/cancel', {'ticket': ticket, 'node': self.target})[0], 400)
        headers = {**self.h.headers(), 'X-Workspace-CSRF': 'invalid'}
        self.assertEqual(self.h.http('POST', '/api/desktop/cancel', {'ticket': ticket}, headers=headers)[0], 401)
        status, result = self.h.post('/api/desktop/cancel', {'ticket': ticket})
        self.assertEqual((status, result), (200, {'cancelled': True}))
        until(lambda: self.vnc.active == 0, seconds=3)
        self.assertEqual(self.browser(ticket).status, 401)
        self.assertEqual(self.h.post('/api/desktop/cancel', {'ticket': ticket})[1], {'cancelled': False})
        ticket = self.ticket()
        viewer = self.browser(ticket)
        self.assertEqual(viewer.recv(), (2, self.vnc.banner))
        self.assertEqual(self.h.post('/api/desktop/cancel', {'ticket': ticket})[1], {'cancelled': False})
        viewer.send(b'still connected', opcode=2)
        self.assertEqual(viewer.recv(), (2, b'still connected'))


if __name__ == '__main__':
    unittest.main()
