"""Opt-in integration with real x11vnc and an isolated Xvfb desktop.

CRANIBOX_TEST_REAL_VNC=1 python3 -m unittest discover -s tests -p test_desktop_real_vnc.py -v
Requires x11vnc, Xvfb, xev, xsetroot, xdotool, Node and the Web UI's npm dependencies.
No package installation, gateway dependency or access to the current desktop.
"""
import json
import os
from pathlib import Path
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from test_cranibox_terminal import Socket
from workspace_web_harness import CORE, NODE, ROOT, WEB, Harness, free_port, sockets_allowed, until


DES = ROOT / 'addons/novnc/core/crypto/des.js'
TOOLS = ('x11vnc', 'Xvfb', 'xev', 'xsetroot', 'xdotool', 'node', 'stdbuf')


class Rfb:
    """Small, bounded RFB client; authentication uses the product's noVNC DES."""
    def __init__(self, test, connection):
        self.test, self.connection = test, connection
        self.pending = b''

    def read(self, size):
        self.test.assertLessEqual(size, 320 * 240 * 4)
        while len(self.pending) < size:
            opcode, data = self.connection.recv()
            self.test.assertEqual(opcode, 2, 'RFB transport closed unexpectedly')
            self.test.assertTrue(data)
            self.pending += data
            self.test.assertLessEqual(len(self.pending), 320 * 240 * 4 + 65536)
        value, self.pending = self.pending[:size], self.pending[size:]
        return value

    def send(self, data):
        self.connection.send(data, opcode=2)

    def authenticate(self, password):
        self.test.assertIn(self.read(12), (b'RFB 003.007\n', b'RFB 003.008\n'))
        self.send(b'RFB 003.008\n')
        methods = self.read(self.read(1)[0])
        self.test.assertIn(2, methods, 'Managed VNC must require a password')
        self.test.assertNotIn(1, methods, 'Managed VNC must not offer unauthenticated access')
        self.send(b'\x02')
        challenge = self.read(16)
        # Secrets use stdin, never command arguments or test failure output.
        script = '''
import { readFileSync } from 'node:fs';
const { DESECBCipher } = await import(process.argv[1]);
const input = JSON.parse(readFileSync(0, 'utf8'));
const key = DESECBCipher.importKey([...input.password].map(c => c.charCodeAt(0)));
const response = key.encrypt({}, Buffer.from(input.challenge, 'hex'));
process.stdout.write(Buffer.from(response).toString('hex'));
'''
        result = subprocess.run(['node', '--input-type=module', '-e', script, DES.as_uri()],
                                input=json.dumps({'password': password, 'challenge': challenge.hex()}),
                                text=True, capture_output=True, timeout=10)
        self.test.assertEqual(result.returncode, 0, result.stderr)
        self.send(bytes.fromhex(result.stdout))
        accepted = self.read(4) == b'\0\0\0\0'
        if not accepted:
            return False
        self.send(b'\x01')  # Shared session; the node still enforces one controller.
        self.width, self.height = struct.unpack('!HH', self.read(4))
        self.test.assertEqual((self.width, self.height), (320, 240))
        self.read(16)
        name_size = struct.unpack('!I', self.read(4))[0]
        self.test.assertLessEqual(name_size, 4096)
        self.read(name_size)
        return True

    def frame(self, attempts=10):
        pixel_format = struct.pack('!BBBBHHHBBB3x', 32, 24, 0, 1, 255, 255, 255, 16, 8, 0)
        self.send(b'\0\0\0\0' + pixel_format)
        self.send(struct.pack('!BBHi', 2, 0, 1, 0))  # Only raw rectangles.
        # A freshly (re)started server can send its first full frame before the
        # compositor has drawn the background; ask again before failing.
        for attempt in range(attempts):
            if self.background_frame(final=attempt == attempts - 1):
                return
            time.sleep(0.3)

    def background_frame(self, final):
        self.send(struct.pack('!BBHHHH', 3, 0, 0, 0, self.width, self.height))
        self.test.assertEqual(self.read(2), b'\0\0')
        count = struct.unpack('!H', self.read(2))[0]
        self.test.assertTrue(0 < count <= 256)
        pixels = b''
        for _ in range(count):
            x, y, width, height, encoding = struct.unpack('!HHHHi', self.read(12))
            self.test.assertEqual(encoding, 0)
            self.test.assertLessEqual(x + width, self.width)
            self.test.assertLessEqual(y + height, self.height)
            pixels += self.read(width * height * 4)
        # The isolated desktop's background is #2468ac (little-endian BGRX).
        shown = pixels.count(bytes.fromhex('ac682400'))
        if final:
            self.test.assertGreater(shown, 320 * 240 // 2)
        return shown > 320 * 240 // 2

    def input(self):
        self.send(struct.pack('!BBHH', 5, 1, 87, 69))
        self.send(struct.pack('!BBHH', 5, 0, 87, 69))
        self.send(struct.pack('!BB2xI', 4, 1, ord('a')))
        self.send(struct.pack('!BB2xI', 4, 0, ord('a')))


@unittest.skipUnless(os.environ.get('CRANIBOX_TEST_REAL_VNC') == '1', 'opt in to real VNC integration')
class RealVncTests(unittest.TestCase):
    def setUp(self):
        for tool in TOOLS:
            self.assertIsNotNone(shutil.which(tool), f'Missing integration tool: {tool}')
        for path in (CORE, NODE, WEB, DES):
            self.assertTrue(path.is_file(), f'Missing integration dependency: {path}')
        self.assertTrue(sockets_allowed(), 'Real sockets are required')
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-real-vnc-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.processes = []
        self.logs = []
        self.addCleanup(self.close_processes)
        read_fd, write_fd = os.pipe()
        try:
            self.start('Xvfb', '-displayfd', str(write_fd), '-screen', '0', '320x240x24',
                       '-nolisten', 'tcp', '-noreset', pass_fds=(write_fd,))
        finally:
            os.close(write_fd)
        try:
            self.assertTrue(select.select([read_fd], [], [], 10)[0], 'Xvfb did not start')
            number = os.read(read_fd, 16).decode().strip()
            self.assertTrue(number.isdigit(), 'Invalid Xvfb display')
        finally:
            os.close(read_fd)
        self.display = ':' + number
        # Keep upstream diagnostics for failed integration runs; Rust still owns
        # the real process (exec, no extra supervisor or simulated VNC server).
        binary = shutil.which('x11vnc')
        wrapper = self.root / 'x11vnc'
        self.vnc_log = self.root / 'vnc.log'
        wrapper.write_text('#!' + sys.executable + '\nimport os,sys\n'
                           + f'fd=os.open({str(self.vnc_log)!r},os.O_CREAT|os.O_WRONLY|os.O_APPEND,0o600)\n'
                           + 'os.dup2(fd,2)\nos.close(fd)\n'
                           + f'os.execv({binary!r},[{binary!r},*sys.argv[1:]])\n')
        wrapper.chmod(0o700)
        environment = patch.dict(os.environ, {'DISPLAY': self.display, 'XDG_SESSION_TYPE': 'x11',
                                              'PATH': str(self.root) + os.pathsep + os.environ['PATH']})
        environment.start()
        self.addCleanup(environment.stop)
        self.xenv = {**os.environ, 'DISPLAY': self.display}
        self.xcommand('xsetroot', '-solid', '#2468ac')
        self.events = self.root / 'input.log'
        self.start('stdbuf', '-oL', 'xev', '-root', '-event', 'keyboard', '-event', 'mouse', log=self.events)
        self.h = Harness(unified=True)
        self.addCleanup(self.h.close)
        self.config = self.h.root / '.config/cranibox/peers.json'

    def start(self, *args, log=None, **kwargs):
        stream = (log or self.root / f'process-{len(self.logs)}.log').open('wb')
        self.logs.append(stream)
        child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=stream, stderr=stream, **kwargs)
        self.processes.append(child)
        return child

    def close_processes(self):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        for log in self.logs:
            log.close()

    def xcommand(self, *args):
        result = subprocess.run(args, env=self.xenv, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def service(self, **command):
        status, value = self.h.post('/api/desktop/service', command)
        self.assertEqual(status, 200, value)
        return value

    def open(self, harness, node, local=False):
        code, ticket = harness.post('/api/desktop/ticket', {'node': node, 'local': local})
        self.assertEqual(code, 200, ticket)
        viewer = Socket(harness, ticket['ticket'], path='/api/desktop/ws')
        self.addCleanup(viewer.close)
        self.assertEqual(viewer.status, 101)
        return viewer, Rfb(self, viewer)

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.config.with_name('settings.json')),
                                 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_real_auth_pixels_input_persistence_and_authorized_remote_access(self):
        port = free_port()
        started = self.service(operation='start', display=self.display, port=port)
        self.assertTrue(started['enabled'] and started['running'])
        password = self.service(operation='credentials')['password']
        identity = json.loads(self.config.read_text())['node_id']
        viewer, rfb = self.open(self.h, identity, local=True)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        rfb.input()
        until(lambda: 'KeyPress' in self.events.read_text())
        events = self.events.read_text()
        self.assertIn('ButtonPress', events)
        self.assertIn('keysym 0x61, a', events)
        pointer = self.xcommand('xdotool', 'getmouselocation', '--shell').splitlines()
        self.assertIn('X=87', pointer)
        self.assertIn('Y=69', pointer)
        viewer.close()
        self.assertTrue(self.service(operation='status')['running'])
        viewer, rfb = self.open(self.h, identity, local=True)
        wrong_password = ('A' if password[0] != 'A' else 'B') + password[1:]
        self.assertFalse(rfb.authenticate(wrong_password))
        viewer.close()
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        until(lambda: self.service(operation='status')['running'])
        self.assertEqual(self.service(operation='credentials')['password'], password)
        viewer, rfb = self.open(self.h, identity, local=True)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        viewer.close()

        # The same device and service then run without Web or a gateway.
        self.h.stop(self.h.web)
        self.h.stop(self.h.server)
        caller = Harness(unified=True)
        self.addCleanup(caller.close)
        code, value = caller.post('/api/peers', {'operation': 'identity'})
        self.assertEqual(code, 200, value)
        caller_id = value['node_id']
        token_file = self.root / 'desktop.token'
        self.cli('grant', '--allow-unverified', '--node', caller_id, '--output-token', token_file, '--desktop')
        listener = free_port()
        self.cli('listen', '--address', f'127.0.0.1:{listener}')
        peer = self.h.start(NODE, '--config', self.config.with_name('settings.json'), 'peer', 'serve')
        code, value = caller.post('/api/peers', {'operation': 'connect', 'node': identity,
                                 'url': f'http://127.0.0.1:{listener}', 'token': token_file.read_text().strip()})
        self.assertEqual(code, 200, value)
        until(lambda: caller.post('/api/peers', {'operation': 'probe', 'node': identity})[0] == 200)
        try:
            until(lambda: self.port_ready(port))
        except AssertionError:
            desktop = json.loads(self.config.read_text()).get('desktop')
            log = (self.h.root / f'process-{len(self.h.logs) - 1}.log').read_text()
            diagnostics = self.vnc_log.read_text()[-4000:]
            self.fail(f'Headless VNC did not restore: desktop={desktop}, node_exit={peer.poll()}, log={log}, VNC={diagnostics}')
        viewer, rfb = self.open(caller, identity)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        self.cli('revoke', '--node', caller_id)
        self.assertEqual(viewer.recv()[0], 8)
        self.assertEqual(caller.post('/api/desktop/ticket', {'node': identity})[0], 400)
        self.assertIsNone(peer.poll())
        self.h.stop(peer)
        until(lambda: not self.port_ready(port))

    @staticmethod
    def port_ready(port):
        import socket
        try:
            # Real LibVNCServer may inspect the first bytes for its optional
            # WebSocket protocol before sending the native RFB greeting.
            with socket.create_connection(('127.0.0.1', port), timeout=2) as connection:
                return connection.recv(12).startswith(b'RFB ')
        except OSError:
            return False


if __name__ == '__main__':
    unittest.main()
