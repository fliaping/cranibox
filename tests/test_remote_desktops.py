"""Real browser -> Rust Web -> independent node -> TCP desktop forwarding."""
import json
import base64
import hashlib
import socket
import struct
import subprocess
import threading
import time
import unittest

from workspace_web_harness import Harness, CORE, NODE, RUNTIME, WEB, free_port, sockets_allowed, until
from test_cranibox_terminal import Socket


class DesktopServer:
    """Opaque RFB bytes: transport must preserve binary, including non-UTF8."""
    banner = b'RFB 003.008\n'

    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen()
        self.address = '127.0.0.1:' + str(self.listener.getsockname()[1])
        self.closed = threading.Event()
        self.connections = []
        self.active = 0
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while not self.closed.is_set():
            try:
                connection, _ = self.listener.accept()
            except OSError:
                return
            self.connections.append(connection)
            self.active += 1
            threading.Thread(target=self.echo, args=(connection,), daemon=True).start()

    def echo(self, connection):
        try:
            connection.sendall(self.banner)
            while data := connection.recv(65536):
                connection.sendall(data)
        except OSError:
            pass
        finally:
            self.active -= 1
            connection.close()

    def close(self):
        self.closed.set()
        self.listener.close()
        for connection in self.connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            connection.close()


class SelkiesServer(DesktopServer):
    payload = bytes(range(256)) * 512

    def echo(self, connection):
        def exact(count):
            value = b''
            while len(value) < count:
                chunk = connection.recv(count - len(value))
                if not chunk:
                    raise EOFError()
                value += chunk
            return value

        def send(payload, opcode):
            size = len(payload)
            header = bytes([128 | opcode, size if size < 126 else 126 if size <= 65535 else 127])
            if size >= 126:
                header += struct.pack('!H' if size <= 65535 else '!Q', size)
            connection.sendall(header + payload)

        try:
            request = b''
            while not request.endswith(b'\r\n\r\n'):
                request += exact(1)
                if len(request) > 8192:
                    return
            key = next(line.split(b':', 1)[1].strip() for line in request.split(b'\r\n') if line.lower().startswith(b'sec-websocket-key:'))
            accept = base64.b64encode(hashlib.sha1(key + b'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest())
            connection.sendall(b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + b'\r\n\r\n')
            send(b'MODE websockets', 1)
            send(self.payload, 2)
            while True:
                a, b = exact(2)
                size = b & 127
                if size == 126:
                    size = struct.unpack('!H', exact(2))[0]
                elif size == 127:
                    size = struct.unpack('!Q', exact(8))[0]
                if size > 65536:
                    return
                mask = exact(4) if b & 128 else None
                payload = exact(size)
                if mask:
                    payload = bytes(value ^ mask[i % 4] for i, value in enumerate(payload))
                opcode = a & 15
                if opcode == 8:
                    send(b'', 8)
                    return
                if opcode == 9:
                    send(payload, 10)
                elif opcode == 1:
                    send(b'input:' + payload, 1)
        except (OSError, EOFError, StopIteration):
            pass
        finally:
            self.active -= 1
            connection.close()


@unittest.skipUnless(all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class RemoteDesktopTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.vnc = DesktopServer()
        self.addCleanup(self.vnc.close)
        self.caller = self.api(operation='identity')['node_id']
        self.target_config = self.h.root / 'desktop-target/settings.json'
        self.target = self.cli('identity')['node_id']
        token_file = self.h.root / 'desktop.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_file, '--desktop')
        self.token = token_file.read_text().strip()
        self.port = free_port()
        self.cli('desktop', '--address', self.vnc.address)
        self.cli('listen', '--address', f'127.0.0.1:{self.port}')
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{self.port}', token=self.token)
        until(lambda: self.h.post('/api/peers', {'operation':'probe', 'node':self.target})[0] == 200)

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def api(self, **command):
        status, body = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, body)
        return body

    def ticket(self, node=None, local=False):
        status, body = self.h.post('/api/desktop/ticket', {'node':node or self.target, 'local':local})
        self.assertEqual(status, 200, body)
        return body['ticket']

    def open(self, ticket=None, **kwargs):
        connection = Socket(self.h, ticket or self.ticket(), path='/api/desktop/ws', **kwargs)
        self.addCleanup(connection.close)
        return connection

    def connected(self, **kwargs):
        connection = self.open(**kwargs)
        self.assertEqual(connection.status, 101)
        self.assertEqual(connection.recv(), (2, self.vnc.banner))
        return connection

    def test_binary_data_and_reconnect_without_gateway_or_desktop_restart(self):
        self.h.stop(self.h.server)
        self.assertTrue(self.api(operation='probe', node=self.target)['nodes'][0]['desktop'])
        first = self.connected()
        data = bytes(range(256)) * 200
        first.send(data, opcode=2)
        received = b''
        while len(received) < len(data):
            opcode, chunk = first.recv()
            self.assertEqual(opcode, 2)
            received += chunk
        self.assertEqual(received, data)
        first.close()
        until(lambda: self.vnc.active == 0)
        second = self.connected()
        second.send(b'keyboard-and-pointer', opcode=2)
        self.assertEqual(second.recv(), (2, b'keyboard-and-pointer'))
        self.assertIsNone(self.target_process.poll())

    def test_selkies_preserves_messages_large_frames_and_target_revocation(self):
        server = SelkiesServer()
        self.addCleanup(server.close)
        self.cli('desktop', '--address', server.address, '--protocol', 'selkies')
        self.assertEqual(self.api(operation='probe', node=self.target)['nodes'][0]['desktop'], True)
        connection = self.open()
        self.assertEqual(connection.status, 101)
        self.assertEqual(connection.recv(), (2, b'CRANIBOX-SELKIES/1\n'))
        buffer = b''

        def message():
            nonlocal buffer
            while len(buffer) < 5 or len(buffer) < 5 + int.from_bytes(buffer[1:5], 'big'):
                opcode, data = connection.recv()
                self.assertEqual(opcode, 2)
                self.assertLessEqual(len(data), 65536)
                buffer += data
            kind, size = buffer[0], int.from_bytes(buffer[1:5], 'big')
            payload, buffer = buffer[5:5 + size], buffer[5 + size:]
            return kind, payload

        self.assertEqual(message(), (0, b'MODE websockets'))
        self.assertEqual(message(), (1, server.payload))
        data = b'\0' + struct.pack('!I', 5) + b'kd,97'
        connection.send(data[:3], opcode=2)
        connection.send(data[3:], opcode=2)
        self.assertEqual(message(), (0, b'input:kd,97'))
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(connection.recv()[0], 8)
        until(lambda: server.active == 0)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)

    def test_single_controller_and_sharing_stop_release_the_lease(self):
        first = self.connected()
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)
        self.assertEqual(self.vnc.active, 1)
        self.cli('desktop')
        self.assertEqual(first.recv()[0], 8)
        until(lambda: self.vnc.active == 0)
        self.assertFalse(self.api(operation='probe', node=self.target)['nodes'][0]['desktop'])
        self.cli('desktop', '--address', self.vnc.address)
        self.connected()

    def test_revocation_disconnects_live_viewer_without_input(self):
        first = self.connected()
        start = time.monotonic()
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(first.recv()[0], 8)
        self.assertLess(time.monotonic() - start, 3)
        until(lambda: self.vnc.active == 0)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)

    def test_explicit_scope_target_binding_and_loopback_only(self):
        self.api(operation='connect', node='wrong-target', url=f'http://127.0.0.1:{self.port}', token=self.token)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':'wrong-target'})[0], 400)
        self.cli('revoke', '--node', self.caller)
        token_file = self.h.root / 'exec-only.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_file, '--exec')
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{self.port}', token=token_file.read_text().strip())
        self.assertFalse(self.api(operation='probe', node=self.target)['nodes'][0]['desktop'])
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)
        for address in ('0.0.0.0:5900', '192.168.1.2:5900', '127.0.0.1:0'):
            self.assertEqual(self.h.post('/api/peers', {'operation':'desktop', 'desktop':{'address':address}})[0], 400)
        self.assertIsNone(self.api(operation='list')['desktop'])

    def test_browser_ticket_origin_csrf_replay_and_session_binding(self):
        ticket = self.ticket()
        for cookie, origin, expected in [('', None, 401), (None, 'https://evil.invalid', 403), (None, 'omit', 403)]:
            self.assertEqual(self.open(ticket=ticket, cookie=cookie, origin=origin).status, expected)
        previous_cookie, previous_csrf = self.h.cookie, self.h.csrf
        self.h.login()
        self.assertEqual(self.open(ticket=ticket).status, 401)
        self.h.cookie, self.h.csrf = previous_cookie, previous_csrf
        self.connected(ticket=ticket)
        self.assertEqual(self.open(ticket=ticket).status, 401)
        self.assertEqual(self.h.http('POST', '/api/desktop/ticket', {'node':self.target}, headers={'Cookie':self.h.cookie, 'Content-Type':'application/json'})[0], 401)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':'unknown-target'})[0], 400)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target, 'local':True})[0], 400)

    def test_local_owner_uses_same_pipeline_and_invalid_frames_detach(self):
        self.api(operation='desktop', desktop={'address':self.vnc.address})
        first = self.connected(ticket=self.ticket(self.caller, local=True))
        first.send(b'not-rfb-json', opcode=1)
        self.assertEqual(first.recv()[0], 8)
        until(lambda: self.vnc.active == 0)
        second = self.connected(ticket=self.ticket(self.caller, local=True))
        self.api(operation='desktop', desktop=None)
        self.assertEqual(second.recv()[0], 8)
        until(lambda: self.vnc.active == 0)


if __name__ == '__main__':
    unittest.main()
