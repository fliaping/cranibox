"""Native desktop transport against a real gateway and node-owned PTY."""
import base64
import hashlib
import json
import os
import socket
import time
import unittest
import uuid

from test_cranibox_terminal import Socket
from test_node_peers import DirectSocket
from workspace_web_harness import CORE, NODE, Harness, sockets_allowed, until


class GatewaySocket(DirectSocket):
    def __init__(self, harness, session, token, *, controller=False, query='', headers=None):
        self.socket = socket.create_connection(('127.0.0.1', harness.gateway_port), timeout=5)
        self.socket.settimeout(5)
        self.reader = self.socket.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        prefix = '/api/v1' if controller else '/api/v1/mesh'
        request = [f'GET {prefix}/nodes/test-node/terminal/ws?session={session}{query} HTTP/1.1',
                   f'Host: 127.0.0.1:{harness.gateway_port}', 'Upgrade: websocket',
                   'Connection: Upgrade', 'Sec-WebSocket-Version: 13',
                   f'Sec-WebSocket-Key: {key}', f'Authorization: Bearer {token}']
        request.extend(f'{name}: {value}' for name, value in (headers or {}).items())
        self.socket.sendall(('\r\n'.join(request) + '\r\n\r\n').encode())
        self.status = int(self.reader.readline().split()[1])
        response = {}
        while line := self.reader.readline():
            if line == b'\r\n':
                break
            name, value = line.decode().split(':', 1)
            response[name.lower()] = value.strip()
        if self.status == 101:
            expected = base64.b64encode(hashlib.sha1(
                (key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            assert response['sec-websocket-accept'] == expected


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(),
                     'build cranibox and cranibox-node; real sockets and Unix PTY required')
class GatewayTerminalTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN': '/missing-node-herdr'})
        self.addCleanup(self.h.close)
        self.token = (self.h.root / 'node.token').read_text().strip()

    def open(self, session, token=None, status=101, **options):
        ws = GatewaySocket(self.h, session, token or self.token, **options)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, status)
        return ws

    def manage(self, **command):
        status, value = self.h.http('POST', '/api/v1/mesh/nodes/test-node/terminal/sessions',
                                   command, self.h.gateway_port,
                                   {'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'})
        self.assertEqual(status, 200, value)
        return value

    def create(self):
        return self.manage(operation='create', name='Desktop transport', cwd=str(self.h.root))['session']['id']

    def assert_detached(self, ws):
        end = time.monotonic() + 3
        while time.monotonic() < end:
            opcode, value = ws.recv()
            if opcode == 8:
                ws.close()
                return
            # Revocation and malformed input detach; they must never end the Shell.
            self.assertNotEqual(json.loads(value).get('type'), 'exit')
        self.fail('Gateway did not detach the terminal')

    def test_desktop_and_browser_attach_same_shell_across_gateway_restart(self):
        session = self.create()
        first = self.open(session)
        pid_path = self.h.root / 'shell.pid'
        first.command(f'export CRANIBOX_DESKTOP=retained; echo $$ > {pid_path}; printf "%s%s" DESKTOP _READY')
        first.expect('DESKTOP_READY')
        until(pid_path.exists)
        pid = int(pid_path.read_text())
        first.close()

        status, ticket = self.h.post('/api/terminal/ticket', {
            'node': 'test-node', 'session_id': session, 'cols': 80, 'rows': 24})
        self.assertEqual(status, 200, ticket)
        browser = Socket(self.h, ticket['ticket'])
        self.addCleanup(browser.close)
        self.assertEqual(browser.status, 101)
        browser.command('printf "%s%s" BROWSER_ "$CRANIBOX_DESKTOP"')
        browser.expect('BROWSER_retained')
        browser.close()

        self.h.stop(self.h.server)
        os.kill(pid, 0)
        self.h.start_web()
        self.h.login()
        until(lambda: self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        again = self.open(session)
        again.send({'type': 'resize', 'cols': 101, 'rows': 31})
        size_path, reattached_path = self.h.root / 'size', self.h.root / 'reattached.pid'
        again.command(f'stty size > {size_path}; echo $$ > {reattached_path}; printf "%s%s" RESTORED_ "$CRANIBOX_DESKTOP"')
        again.expect('RESTORED_retained')
        until(size_path.exists)
        self.assertEqual(size_path.read_text().strip(), '31 101')
        self.assertEqual(int(reattached_path.read_text()), pid)
        again.command("printf '%s%s' 中文 测试")
        again.expect('中文测试')
        again.command('sleep 30')
        time.sleep(.1)
        again.send({'type': 'input', 'data': base64.b64encode(b'\x03').decode()})
        again.command("printf '%s%s' INTERRUPT _OK")
        again.expect('INTERRUPT_OK')
        self.assertEqual([s['id'] for s in self.manage(operation='list')['sessions']], [session])
        again.command('exit 7')
        while True:
            opcode, payload = again.recv()
            self.assertNotEqual(opcode, 8)
            value = json.loads(payload)
            if value['type'] == 'exit':
                self.assertEqual(value['code'], 7)
                break

    def test_authentication_and_live_group_or_token_revocation_preserve_shell(self):
        session = self.create()
        controller = json.loads(self.h.credentials.read_text())['controller_token']
        self.open(session, token='wrong', status=401)
        self.open(session, token=controller, status=401)
        self.open(session, status=401, controller=True)
        self.open(session, status=403, headers={'Origin': 'https://untrusted.invalid'})
        self.open(session, status=403, headers={'Sec-Fetch-Site': 'cross-site'})
        for query in ('&cols=1', '&rows=301', '&unknown=true'):
            self.open(session, status=400, query=query)
        self.open('not-a-uuid', status=400)

        caller_path = self.h.root / 'caller.token'
        self.h.cli('enroll', 'caller', '--credentials', self.h.credentials, '--token-file', caller_path)
        caller_token = caller_path.read_text().strip()
        self.open(session, token=caller_token, status=403)
        for node in ('caller', 'test-node'):
            self.h.cli('group', node, '--group', 'personal', '--credentials', self.h.credentials)
        active = self.open(session, token=caller_token)
        active.command("export ACCESS_STATE=kept; printf '%s%s' ACCESS _READY")
        active.expect('ACCESS_READY')
        self.h.cli('group', 'caller', '--credentials', self.h.credentials)
        self.assert_detached(active)
        self.open(session, token=caller_token, status=403)
        self.h.cli('group', 'caller', '--group', 'personal', '--credentials', self.h.credentials)
        active = self.open(session, token=caller_token)
        active.command('printf "%s%s" GROUP_ "$ACCESS_STATE"')
        active.expect('GROUP_kept')
        rotated_path = self.h.root / 'rotated.token'
        self.h.cli('enroll', 'caller', '--credentials', self.h.credentials, '--token-file', rotated_path)
        self.assert_detached(active)
        self.open(session, token=caller_token, status=401)
        active = self.open(session, token=rotated_path.read_text().strip())
        active.command('printf "%s%s" ROTATED_ "$ACCESS_STATE"')
        active.expect('ROTATED_kept')
        active.close()
        admin = self.open(session, token=controller, controller=True)
        admin.command('printf "%s%s" ADMIN_ "$ACCESS_STATE"')
        admin.expect('ADMIN_kept')
        self.assertEqual(self.manage(operation='list')['sessions'][0]['status'], 'running')

    def test_bad_frames_detach_without_replacing_the_shell(self):
        session = self.create()
        invalid = [({'type': 'input', 'data': 'not base64!'}, 1),
                   ({'type': 'resize', 'cols': 0, 'rows': 24}, 1),
                   ({'type': 'input', 'data': '', 'unexpected': True}, 1),
                   ({'type': 'input', 'data': base64.b64encode(b'x' * (16 * 1024 + 1)).decode()}, 1),
                   (b'binary frame', 2)]
        for index, (frame, opcode) in enumerate(invalid):
            with self.subTest(index=index):
                ws = self.open(session)
                ws.command(f'export ORIGINAL_SHELL=retained; printf "%s%s" READY _{index}')
                ws.expect(f'READY_{index}')
                ws.send(frame, opcode=opcode)
                self.assert_detached(ws)
                self.assertEqual(self.manage(operation='list')['sessions'][0]['status'], 'running')
        again = self.open(session)
        again.command('printf "%s%s" ORIGINAL_ "$ORIGINAL_SHELL"')
        again.expect('ORIGINAL_retained')


if __name__ == '__main__':
    unittest.main()
