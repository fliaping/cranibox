"""Actual authenticated WebSockets and local/remote PTYs; no mocks or third-party WS client."""
import base64
import hashlib
import json
import os
import re
from pathlib import Path
import socket
import struct
import time
import unittest

from workspace_web_harness import CORE, NODE, Harness, sockets_allowed, until


class Socket:
    def __init__(self, harness, ticket, cookie=None, origin=None):
        self.socket = socket.create_connection(('127.0.0.1', harness.web_port), timeout=5)
        self.socket.settimeout(5)
        self.reader = self.socket.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        headers = [f'GET /api/terminal/ws?ticket={ticket} HTTP/1.1',
                   f'Host: 127.0.0.1:{harness.web_port}', 'Upgrade: websocket',
                   'Connection: Upgrade', 'Sec-WebSocket-Version: 13', f'Sec-WebSocket-Key: {key}',
                   'Cookie: ' + (harness.cookie if cookie is None else cookie)]
        if origin != 'omit':
            headers.append('Origin: ' + (origin or f'http://127.0.0.1:{harness.web_port}'))
        self.socket.sendall(('\r\n'.join(headers) + '\r\n\r\n').encode())
        self.status = int(self.reader.readline().split()[1])
        response = {}
        while True:
            line = self.reader.readline()
            if line == b'\r\n' or not line:
                break
            name, value = line.decode().split(':', 1)
            response[name.lower()] = value.strip()
        if self.status == 101:
            accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            assert response['sec-websocket-accept'] == accept

    def send(self, value, opcode=1):
        data = json.dumps(value).encode() if isinstance(value, dict) else value
        mask = os.urandom(4)
        size = len(data)
        prefix = bytes([0x80 | opcode, 0x80 | (size if size < 126 else 126)])
        if size >= 126:
            prefix += struct.pack('!H', size)
        self.socket.sendall(prefix + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def recv(self):
        while True:
            prefix = self.reader.read(2)
            if len(prefix) != 2:
                return 8, b''
            opcode, size = prefix[0] & 15, prefix[1] & 127
            if size == 126:
                size = struct.unpack('!H', self.reader.read(2))[0]
            elif size == 127:
                size = struct.unpack('!Q', self.reader.read(8))[0]
            data = self.reader.read(size)
            if opcode == 9:
                self.send(data, opcode=10)
                continue
            return opcode, data

    def command(self, line):
        self.send({'type': 'input', 'data': line + '\r'})

    def expect(self, text):
        data = b''
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            opcode, value = self.recv()
            if opcode == 8:
                raise AssertionError('Terminal closed: ' + repr(data))
            data += value
            plain = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', data)
            if text.encode() in data or text.encode() in plain:
                return data.decode(errors='replace')
        raise AssertionError('Missing terminal output: ' + text)

    def close(self):
        try:
            self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.reader.close()
        self.socket.close()


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(), 'build cranibox and cranibox-node; PTY tests require sockets and Unix')
class TerminalTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN': '/missing-node-herdr'})
        self.addCleanup(self.h.close)

    def ticket(self, node=None, **kwargs):
        if 'session_id' not in kwargs:
            kwargs['session_id'] = self.create(node, kwargs.pop('cwd', None))
        status, value = self.h.post('/api/terminal/ticket', {'node': node, 'cols': 80, 'rows': 24, **kwargs})
        self.assertEqual(status, 200, value)
        return value['ticket']

    def manage(self, node=None, **request):
        status, value = self.h.post('/api/terminal/sessions', {'node': node, **request})
        self.assertEqual(status, 200, value)
        return value

    def create(self, node=None, cwd=None):
        return self.manage(node, operation='create', name='PTY test', cwd=cwd)['session']['id']

    def open(self, node=None, **kwargs):
        ws = Socket(self.h, self.ticket(node, **kwargs))
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        # Opening the WebSocket precedes asynchronous PTY attachment.
        ws.expect('')
        time.sleep(.1)
        return ws

    def test_local_and_remote_real_tty_resize_unicode_ctrl_c_and_exit(self):
        for node in (None, 'test-node'):
            with self.subTest(node=node):
                ws = self.open(node, cwd=str(self.h.root))
                ws.command("test -t 0 && test -t 1 && printf '%s%s\\n' TTY _OK")
                ws.expect('TTY_OK')
                ws.send({'type': 'resize', 'cols': 101, 'rows': 31})
                time.sleep(.2)
                size_file = self.h.root / ('local.size' if node is None else 'remote.size')
                ws.command(f'stty size > {size_file}')
                until(size_file.exists)
                self.assertEqual(size_file.read_text().strip(), '31 101')
                ws.command("printf '\\033[32m%s%s\\033[0m\\n' 中文 测试")
                ws.expect('中文测试')
                ws.command('sleep 30')
                time.sleep(.1)
                ws.send({'type': 'input', 'data': '\x03'})
                ws.command("printf '%s%s\\n' INTERRUPT _OK")
                ws.expect('INTERRUPT_OK')
                ws.command('exit 7')
                while True:
                    opcode, data = ws.recv()
                    self.assertNotEqual(opcode, 8)
                    if opcode == 1:
                        self.assertEqual(json.loads(data)['type'], 'attachment_closed')
                        break
                ws.close()

    def test_detach_preserves_shell_and_jobs_reattach_rename_and_end(self):
        for node in (None, 'test-node'):
            with self.subTest(node=node):
                session = self.create(node)
                ws = self.open(node, session_id=session)
                prefix = self.h.root / ('local' if node is None else 'remote')
                ws.command(f'echo $$ > {prefix}.shell; sleep 60 & echo $! > {prefix}.background; sleep 61 & echo $! > {prefix}.foreground; wait')
                until(lambda: Path(str(prefix) + '.foreground').exists())
                pids = [int(Path(str(prefix) + suffix).read_text()) for suffix in ('.shell', '.background', '.foreground')]
                ws.close()
                time.sleep(.25)
                for pid in pids:
                    os.kill(pid, 0)
                    self.assertNotEqual(Path(f'/proc/{pid}/stat').read_text().split(') ')[1][0], 'Z')
                self.manage(node, operation='rename', id=session, name='Still running')
                catalog = self.manage(node, operation='list')['sessions']
                self.assertEqual(catalog[0]['name'], 'Still running')
                self.assertEqual(catalog[0]['status'], 'running')
                reattached = self.open(node, session_id=session)
                reattached.send({'type':'input','data':'\x03'})
                reattached.command("printf '%s%s\\n' REATTACHED _OK")
                reattached.expect('REATTACHED_OK')
                reattached.command(f'echo $$ > {prefix}.reattached')
                until(lambda: Path(str(prefix) + '.reattached').exists())
                self.assertEqual(int(Path(str(prefix) + '.reattached').read_text()), pids[0])
                self.manage(node, operation='end', id=session)
                def ended():
                    for pid in pids:
                        path = Path(f'/proc/{pid}/stat')
                        if path.exists() and path.read_text().split(') ')[1][0] != 'Z':
                            return False
                    return True
                until(ended)
                self.assertEqual(self.manage(node, operation='list')['sessions'], [])

    def test_authentication_origin_csrf_and_one_time_session_ticket(self):
        ticket = self.ticket()
        for cookie, origin, status in [('', None, 401), (None, 'https://evil.invalid', 403), (None, 'omit', 403)]:
            ws = Socket(self.h, ticket, cookie, origin)
            self.addCleanup(ws.close)
            self.assertEqual(ws.status, status)
            ws.close()
        ws = Socket(self.h, ticket)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        replay = Socket(self.h, ticket)
        self.addCleanup(replay.close)
        self.assertEqual(replay.status, 401)
        self.assertEqual(self.h.http('POST', '/api/terminal/ticket', {'cols':80,'rows':24}, headers={'Cookie':self.h.cookie,'Content-Type':'application/json'})[0], 401)
        self.assertEqual(self.h.post('/api/terminal/ticket', {'cols':0,'rows':24})[0], 400)

    def test_server_disconnect_and_restart_preserve_remote_shell_without_herdr(self):
        session = self.create('test-node')
        ws = self.open('test-node', session_id=session)
        shell_file, completed = self.h.root / 'remote-shell', self.h.root / 'offline-completed'
        ws.command(f"echo $$ > {shell_file}; sleep 1; printf '%s%s\\n' OFFLINE _OUTPUT; printf done > {completed}")
        until(shell_file.exists)
        shell_pid = int(shell_file.read_text())
        node_pid = self.h.node.pid
        self.assertEqual(self.manage('test-node', operation='list')['engine'], 'node-pty')
        self.h.stop(self.h.web, force=True)
        until(completed.exists)
        os.kill(shell_pid, 0)
        self.assertIsNone(self.h.node.poll())
        self.h.start_web()
        self.h.login()
        until(lambda: self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        catalog = self.manage('test-node', operation='list')
        self.assertEqual(catalog['sessions'][0]['id'], session)
        reattached = Socket(self.h, self.ticket('test-node', session_id=session))
        self.addCleanup(reattached.close)
        self.assertEqual(reattached.status, 101)
        reattached.expect('OFFLINE_OUTPUT')
        reattached.command("printf '%s%s\\n' SERVER_RESTART _OK")
        reattached.expect('SERVER_RESTART_OK')
        reattached.command(f'echo $$ > {self.h.root / "reattached-shell"}')
        until(lambda: (self.h.root / 'reattached-shell').exists())
        self.assertEqual(int((self.h.root / 'reattached-shell').read_text()), shell_pid)
        self.assertEqual(self.h.node.pid, node_pid)

    def test_client_restart_ends_owned_sessions(self):
        session = self.create('test-node')
        ws = self.open('test-node', session_id=session)
        shell_file = self.h.root / 'client-owned-shell'
        ws.command(f'echo $$ > {shell_file}')
        until(shell_file.exists)
        shell_pid = int(shell_file.read_text())
        self.h.stop(self.h.node)
        until(lambda: not self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        until(lambda: not Path(f'/proc/{shell_pid}/stat').exists() or Path(f'/proc/{shell_pid}/stat').read_text().split(') ')[1][0] == 'Z')
        self.h.node = self.h.start(NODE, '--id', 'test-node', '--server', f'ws://127.0.0.1:{self.h.gateway_port}', '--token-file', self.h.root / 'node.token')
        until(lambda: self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        self.assertEqual(self.manage('test-node', operation='list')['sessions'], [])
        self.assertEqual(self.h.post('/api/terminal/sessions', {'node':'test-node', 'operation':'end', 'id':session})[0], 400)

    def test_takeover_keeps_session_and_transfers_attachment(self):
        for node in (None, 'test-node'):
            with self.subTest(node=node):
                session = self.create(node)
                first = self.open(node, session_id=session)
                first.command("printf '%s%s\\n' FIRST _ATTACH")
                first.expect('FIRST_ATTACH')
                second = self.open(node, session_id=session, takeover=True)
                second.command("printf '%s%s\\n' TAKEOVER _OK")
                second.expect('TAKEOVER_OK')
                while first.recv()[0] != 8:
                    pass
                self.assertEqual(self.manage(node, operation='list')['sessions'][0]['status'], 'running')
                self.manage(node, operation='end', id=session)

    def test_local_session_survives_server_restart_and_invalid_directory(self):
        session = self.create()
        ws = self.open(session_id=session)
        marker = self.h.root / 'continued'
        ws.command(f'sleep 1; printf survived > {marker}')
        time.sleep(.1)
        self.h.stop(self.h.web, force=True)
        until(marker.exists)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.manage(operation='list')['sessions'][0]['status'], 'running')
        reattached = self.open(session_id=session)
        reattached.command("printf '%s%s\\n' LOCAL_RESTART _OK")
        reattached.expect('LOCAL_RESTART_OK')
        self.assertEqual(self.h.post('/api/terminal/sessions', {'operation':'create','name':'invalid','cwd':'/missing-cranibox-terminal-directory'})[0], 400)
        self.assertEqual(self.h.post('/api/terminal/sessions', {'operation':'end','id':'unowned-session'})[0], 400)

    def test_terminal_capacity_released_after_disconnect(self):
        sockets = [self.open() for _ in range(8)]
        extra = Socket(self.h, self.ticket())
        self.addCleanup(extra.close)
        self.assertEqual(extra.status, 429)
        sockets[0].close()
        time.sleep(.15)
        self.assertEqual(self.open().status, 101)


if __name__ == '__main__':
    unittest.main()
