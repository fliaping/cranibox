"""Real browser -> Rust Web -> independent node -> TCP desktop forwarding."""
import json
import base64
import hashlib
import http.client
import os
import socket
import struct
import subprocess
import threading
import time
import unittest
import urllib.parse

from workspace_web_harness import Harness, CORE, NODE, WEB, free_port, sockets_allowed, until
from test_cranibox_terminal import Socket


class DesktopServer:
    """Opaque RFB bytes: transport must preserve binary, including non-UTF8."""
    banner = b'RFB 003.008\n'

    def __init__(self, host='127.0.0.1', port=0):
        ipv6 = ':' in host
        self.listener = socket.socket(socket.AF_INET6 if ipv6 else socket.AF_INET)
        if ipv6:
            self.listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
        self.listener.bind((host, port))
        self.listener.listen()
        self.address = ('[' + host + ']' if ipv6 else host) + ':' + str(self.listener.getsockname()[1])
        self.closed = threading.Event()
        self.connections = []
        self.received = []
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
                self.received.append(data)
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


def framing(connection):
    """Exact reads and unmasked WebSocket frames on a raw server connection."""
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
    return exact, send


class SelkiesServer(DesktopServer):
    payload = bytes(range(256)) * 512

    def echo(self, connection):
        exact, send = framing(connection)
        try:
            request = b''
            while not request.endswith(b'\r\n\r\n'):
                request += exact(1)
                if len(request) > 8192:
                    return
            self.stream(connection, request, exact, send)
        except (OSError, EOFError, StopIteration):
            pass
        finally:
            self.active -= 1
            connection.close()

    def stream(self, connection, request, exact, send):
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
            if size > 128 * 1024:
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
                self.received.append(payload)
                send(b'input:' + payload, 1)


class SelkiesWebServer(SelkiesServer):
    """Selkies 2.x: its page, API and stream share one port (HTTP/1.1 keep-alive)."""
    def __init__(self):
        self.requests = []
        super().__init__()

    def echo(self, connection):
        exact, send = framing(connection)
        try:
            while True:
                request = b''
                while not request.endswith(b'\r\n\r\n'):
                    request += exact(1)
                    if len(request) > 8192:
                        return
                line, *fields = request.decode().split('\r\n')
                method, target, _ = line.split(' ')
                headers = {name.strip().lower(): value.strip() for name, value in (f.split(':', 1) for f in fields if ':' in f)}
                self.requests.append((method, target, headers))
                if headers.get('upgrade', '').lower() == 'websocket':
                    return self.stream(connection, request, exact, send)
                body = exact(int(headers.get('content-length', 0)))
                status, kind, payload, extra = '200 OK', 'text/plain', b'', ''
                if target == '/':
                    kind, payload = 'text/html', b'<!doctype html><title>Selkies</title>'
                elif target.startswith('/api/status'):
                    kind, payload = 'application/json', json.dumps({'target': target, 'headers': headers}).encode()
                elif target == '/redirect':
                    status, extra = '302 Found', 'Location: /api/status\r\n'
                elif method == 'POST':
                    payload = b'posted:' + body
                else:
                    status = '404 Not Found'
                connection.sendall(f'HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {len(payload)}\r\n{extra}\r\n'.encode() + payload)
        except (OSError, EOFError, ValueError):
            pass
        finally:
            self.active -= 1
            connection.close()


APPLICATIONS = 'localhost:3002'
APPLICATIONS_ORIGIN = 'https://' + APPLICATIONS


class PageSocket(Socket):
    """A WebSocket to a desktop page on the application origin."""
    def __init__(self, harness, path, cookie, protocol=None):
        self.socket = socket.create_connection(('127.0.0.1', harness.web_port), timeout=5)
        self.socket.settimeout(5)
        self.reader = self.socket.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        headers = [f'GET {path} HTTP/1.1', f'Host: {APPLICATIONS}', 'Upgrade: websocket',
                   'Connection: Upgrade', 'Sec-WebSocket-Version: 13', f'Sec-WebSocket-Key: {key}',
                   'Cookie: ' + cookie, 'Origin: ' + APPLICATIONS_ORIGIN]
        if protocol:
            headers.append('Sec-WebSocket-Protocol: ' + protocol)
        self.socket.sendall(('\r\n'.join(headers) + '\r\n\r\n').encode())
        self.status = int(self.reader.readline().split()[1])
        self.headers = {}
        while (line := self.reader.readline()) not in (b'\r\n', b''):
            name, value = line.decode().split(':', 1)
            self.headers[name.lower()] = value.strip()


@unittest.skipUnless(all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
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
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', token_file, '--desktop')
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

    def page(self, **body):
        status, value = self.h.post('/api/desktop/page', {'node': self.target, **body})
        self.assertEqual(status, 200, value)
        return value

    def app(self, method, url, cookie='', origin=None, data=None):
        parts = urllib.parse.urlsplit(url)
        path = parts.path + ('?' + parts.query if parts.query else '')
        headers = {'Host': APPLICATIONS}
        if cookie:
            headers['Cookie'] = cookie
        if origin:
            headers['Origin'] = origin
        connection = http.client.HTTPConnection('127.0.0.1', self.h.web_port, timeout=8)
        try:
            connection.request(method, path, data, headers)
            response = connection.getresponse()
            return response.status, {k.lower(): v for k, v in response.getheaders()}, response.read()
        finally:
            connection.close()

    def claim(self, url):
        status, headers, body = self.app('GET', url)
        self.assertEqual(status, 303, body)
        return headers['location'], headers['set-cookie'].split(';')[0]

    def socket(self, path, cookie, protocol=None):
        connection = PageSocket(self.h, path, cookie, protocol)
        self.addCleanup(connection.socket.close)
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
        for command in (b'cr', b'cw,' + base64.b64encode(('中文 👋\n' + 'a' * 65522).encode())):
            data = b'\0' + struct.pack('!I', len(command)) + command
            for offset in range(0, len(data), 65536):
                connection.send(data[offset:offset + 65536], opcode=2)
            self.assertEqual(message(), (0, b'input:' + command))
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(connection.recv()[0], 8)
        until(lambda: server.active == 0)
        self.assertEqual(self.h.post('/api/desktop/ticket', {'node':self.target})[0], 400)

    def test_selkies_page_is_forwarded_unchanged_on_the_application_origin(self):
        server = SelkiesWebServer()
        self.addCleanup(server.close)
        self.cli('desktop', '--address', server.address, '--protocol', 'selkies')
        value = self.page()
        self.assertEqual(value['protocol'], 'selkies')
        self.assertTrue(value['url'].startswith(APPLICATIONS_ORIGIN + '/cranibox-app-'), value)
        prefix, cookie = self.claim(value['url'])
        self.assertEqual(self.app('GET', value['url'])[0], 403)
        self.assertEqual(self.app('GET', prefix)[0], 403)
        status, headers, body = self.app('GET', prefix, cookie)
        self.assertEqual((status, body), (200, b'<!doctype html><title>Selkies</title>'))
        self.assertIn('frame-ancestors http://127.0.0.1:', headers['content-security-policy'])
        status, _, body = self.app('GET', prefix + 'api/status?q=1', cookie)
        seen = json.loads(body)
        self.assertEqual(seen['target'], '/api/status?q=1')
        self.assertEqual(seen['headers']['host'], 'localhost')
        self.assertNotIn('cookie', seen['headers'])
        self.assertEqual(self.app('POST', prefix + 'api/switch', cookie, data=b'{"a":1}')[0], 403)
        status, _, body = self.app('POST', prefix + 'api/switch', cookie, APPLICATIONS_ORIGIN, b'{"a":1}')
        self.assertEqual((status, body), (200, b'posted:{"a":1}'))
        status, headers, _ = self.app('GET', prefix + 'redirect', cookie)
        self.assertEqual((status, headers['location']), (302, prefix + 'api/status'))
        self.assertEqual(self.app('GET', prefix + 'missing', cookie)[0], 404)
        # Sequential requests reuse the tunnelled HTTP connection.
        self.assertLess(len(server.connections), 3)
        stream = self.socket(prefix + 'api/websockets', cookie)
        self.assertEqual(stream.status, 101)
        self.assertEqual(stream.recv(), (1, b'MODE websockets'))
        self.assertEqual(stream.recv(), (2, server.payload))
        stream.send(b'kd,97')
        self.assertEqual(stream.recv(), (1, b'input:kd,97'))
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(stream.recv()[0], 8)
        until(lambda: server.active == 0)
        self.assertEqual(self.app('GET', prefix, cookie)[0], 403)
        self.assertEqual(self.h.post('/api/desktop/page', {'node': self.target})[0], 400)

    def test_vnc_page_is_novnc_and_its_socket_carries_rfb(self):
        value = self.page()
        self.assertEqual(value['protocol'], 'vnc')
        prefix, cookie = self.claim(value['url'])
        status, headers, body = self.app('GET', prefix, cookie)
        self.assertEqual(status, 200)
        self.assertTrue(headers['content-type'].startswith('text/html'))
        self.assertIn(b'noVNC', body)
        status, headers, _ = self.app('GET', prefix + 'app/ui.js', cookie)
        self.assertEqual((status, headers['content-type']), (200, 'text/javascript; charset=utf-8'))
        self.assertEqual(json.loads(self.app('GET', prefix + 'mandatory.json', cookie)[2]), {'host': '', 'port': 0, 'path': 'websockify'})
        self.assertTrue(json.loads(self.app('GET', prefix + 'defaults.json', cookie)[2])['autoconnect'])
        self.assertEqual(self.app('GET', prefix + 'app/missing.js', cookie)[0], 404)
        # The stream opened to identify the desktop serves noVNC's first connection.
        self.assertEqual(self.vnc.active, 1)
        stream = self.socket(prefix + 'websockify', cookie, 'binary')
        self.assertEqual((stream.status, stream.headers['sec-websocket-protocol']), (101, 'binary'))
        self.assertEqual(stream.recv(), (2, self.vnc.banner))
        stream.send(b'\x00\x01\xff', opcode=2)
        self.assertEqual(stream.recv(), (2, b'\x00\x01\xff'))
        # One controller: another page waits until this one closes.
        self.assertEqual(self.h.post('/api/desktop/page', {'node': self.target})[0], 400)
        stream.send(b'', opcode=8)
        until(lambda: self.vnc.active == 0)
        self.assertEqual(self.page()['protocol'], 'vnc')

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
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', token_file, '--exec')
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
