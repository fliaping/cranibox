"""Opt-in real wayvnc integration in an isolated, headless Sway session.

CRANIBOX_TEST_REAL_WAYLAND=1 python3 -m unittest discover -s tests -p test_desktop_real_wayland.py -v
Requires wayvnc >= 0.8, Sway, swaybg, Node and the Web UI npm dependencies.
No package installation or access to an existing login session.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import test_desktop_real_vnc as real_vnc
from test_cranibox_terminal import Socket
from workspace_web_harness import CORE, NODE, ROOT, WEB, Harness, free_port, sockets_allowed, until


CRYPTO = ROOT / 'addons/novnc/core/crypto/crypto.js'
TOOLS = ('wayvnc', 'sway', 'swaymsg', 'swaybg', 'node', 'touch')


class ArdRfb(real_vnc.Rfb):
    """Authenticate against real neatvnc using the viewer's own ARD crypto."""
    def authenticate(self, password, username='cranibox'):
        self.test.assertEqual(self.read(12), b'RFB 003.008\n')
        self.send(b'RFB 003.008\n')
        methods = self.read(self.read(1)[0])
        self.test.assertIn(30, methods, 'wayvnc must offer noVNC-compatible ARD authentication')
        self.test.assertNotIn(1, methods, 'Managed VNC must not offer unauthenticated access')
        self.send(bytes([30]))
        generator = self.read(2)
        key_size = struct.unpack('!H', self.read(2))[0]
        self.test.assertTrue(128 <= key_size <= 512)
        prime, public = self.read(key_size), self.read(key_size)
        # Match noVNC's DH, MD5 and AES-ECB flow. Credentials travel over stdin.
        script = '''
import { readFileSync } from 'node:fs';
import { webcrypto } from 'node:crypto';
globalThis.window = { crypto: webcrypto };
const { default: crypto } = await import(process.argv[1]);
const input = JSON.parse(readFileSync(0, 'utf8'));
const keys = crypto.generateKey({name: 'DH', g: Buffer.from(input.generator, 'hex'),
                                p: Buffer.from(input.prime, 'hex')}, false, ['deriveBits']);
const shared = crypto.deriveBits({name: 'DH', public: Buffer.from(input.public, 'hex')},
                                 keys.privateKey, input.size * 8);
const digest = await crypto.digest('MD5', shared);
const cipher = await crypto.importKey('raw', digest, {name: 'AES-ECB'}, false, ['encrypt']);
const credentials = new Uint8Array(128);
credentials.set(new TextEncoder().encode(input.username).slice(0, 63));
credentials.set(new TextEncoder().encode(input.password).slice(0, 63), 64);
const encrypted = await crypto.encrypt({name: 'AES-ECB'}, cipher, credentials);
const publicKey = crypto.exportKey('raw', keys.publicKey);
process.stdout.write(Buffer.concat([Buffer.from(encrypted), Buffer.from(publicKey)]));
'''
        response = subprocess.run(['node', '--input-type=module', '-e', script, CRYPTO.as_uri()],
                                  input=json.dumps({'generator': generator.hex(), 'prime': prime.hex(),
                                                    'public': public.hex(), 'size': key_size,
                                                    'username': username, 'password': password}).encode(),
                                  capture_output=True, timeout=10)
        self.test.assertEqual(response.returncode, 0, response.stderr.decode(errors='replace'))
        self.test.assertEqual(len(response.stdout), 128 + key_size)
        self.send(response.stdout)
        if self.read(4) != b'\0\0\0\0':
            return False
        self.send(b'\x01')
        self.width, self.height = struct.unpack('!HH', self.read(4))
        self.test.assertEqual((self.width, self.height), (320, 240))
        self.read(16)
        length = struct.unpack('!I', self.read(4))[0]
        self.test.assertLessEqual(length, 4096)
        self.read(length)
        return True


@unittest.skipUnless(os.environ.get('CRANIBOX_TEST_REAL_WAYLAND') == '1', 'opt in to real Wayland VNC integration')
class RealWaylandTests(unittest.TestCase):
    start = real_vnc.RealVncTests.start
    close_processes = real_vnc.RealVncTests.close_processes
    service = real_vnc.RealVncTests.service
    cli = real_vnc.RealVncTests.cli
    port_ready = staticmethod(real_vnc.RealVncTests.port_ready)

    def setUp(self):
        for tool in TOOLS:
            self.assertIsNotNone(shutil.which(tool), f'Missing integration tool: {tool}')
        for path in (CORE, NODE, WEB, CRYPTO):
            self.assertTrue(path.is_file(), f'Missing integration dependency: {path}')
        self.assertTrue(sockets_allowed(), 'Real sockets are required')
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-real-wayland-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.processes, self.logs = [], []
        self.addCleanup(self.close_processes)
        self.key_event = self.root / 'key-event'
        config = self.root / 'sway.conf'
        config.write_text('output HEADLESS-1 mode 320x240\noutput * bg #2468ac solid_color\n'
                          'seat * hide_cursor 100\nxwayland disable\n'
                          + 'bindsym a exec ' + shlex.quote(shutil.which('touch'))
                          + ' ' + shlex.quote(str(self.key_event)) + '\n')
        environment = patch.dict(os.environ, {
            'XDG_RUNTIME_DIR': str(self.root), 'XDG_SESSION_TYPE': 'wayland',
            'XDG_CURRENT_DESKTOP': 'sway', 'XDG_SESSION_DESKTOP': 'sway',
            'WAYLAND_DISPLAY': '', 'DISPLAY': '',
            'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_LIBINPUT_NO_DEVICES': '1',
        })
        environment.start()
        self.addCleanup(environment.stop)
        sway = self.start('sway', '--unsupported-gpu', '-c', str(config))
        def ready():
            self.assertIsNone(sway.poll(), 'Isolated Sway exited before creating its Wayland socket')
            return next((p for p in self.root.glob('wayland-*') if stat.S_ISSOCK(p.stat().st_mode)), None)
        os.environ['WAYLAND_DISPLAY'] = until(ready).name
        os.environ['SWAYSOCK'] = str(until(lambda: next(self.root.glob('sway-ipc.*.sock'), None)))
        # Keep real upstream diagnostics, while Rust owns the exec'd VNC process.
        binary = shutil.which('wayvnc')
        self.vnc_log, self.vnc_pid = self.root / 'vnc.log', self.root / 'vnc.pid'
        wrapper = self.root / 'wayvnc'
        wrapper.write_text('#!' + sys.executable + '\nimport os,sys\nfrom pathlib import Path\n'
                           + f'fd=os.open({str(self.vnc_log)!r},os.O_CREAT|os.O_WRONLY|os.O_APPEND,0o600)\n'
                           + 'os.dup2(fd,2)\nos.close(fd)\n'
                           + f'if "--version" not in sys.argv: Path({str(self.vnc_pid)!r}).write_text(str(os.getpid()))\n'
                           + f'os.execv({binary!r},[{binary!r},*sys.argv[1:]])\n')
        wrapper.chmod(0o700)
        os.environ['PATH'] = str(self.root) + os.pathsep + os.environ['PATH']
        self.h = Harness(unified=True)
        self.addCleanup(self.h.close)
        self.config = self.h.root / '.config/cranibox/peers.json'

    def open(self, harness, node, local=False):
        code, ticket = harness.post('/api/desktop/ticket', {'node': node, 'local': local})
        self.assertEqual(code, 200, ticket)
        viewer = Socket(harness, ticket['ticket'], path='/api/desktop/ws')
        self.addCleanup(viewer.close)
        self.assertEqual(viewer.status, 101)
        return viewer, ArdRfb(self, viewer)

    def test_real_ard_auth_pixels_keyboard_restart_and_authorized_direct_access(self):
        status = self.service(operation='status')
        self.assertTrue(status['supported'] and status['installed'])
        self.assertEqual(status['provider'], 'wayvnc')
        port = free_port()
        started = self.service(operation='start', provider='wayvnc',
                               display=os.environ['WAYLAND_DISPLAY'], output='HEADLESS-1', port=port)
        self.assertTrue(started['running'] and started['enabled'])
        self.assertEqual(started['output'], 'HEADLESS-1')
        credentials = self.service(operation='credentials')
        self.assertEqual(credentials['username'], 'cranibox')
        password = credentials['password']
        identity = json.loads(self.config.read_text())['node_id']
        for username, secret in [('wrong-user', password), ('cranibox', 'wrong-password')]:
            viewer, rfb = self.open(self.h, identity, local=True)
            self.assertFalse(rfb.authenticate(secret, username))
            viewer.close()
        viewer, rfb = self.open(self.h, identity, local=True)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        rfb.input()
        until(self.key_event.exists)
        viewer.close()
        first = int(self.vnc_pid.read_text())
        os.kill(first, signal.SIGKILL)
        def restarted():
            value = self.vnc_pid.read_text()
            return value.isdecimal() and int(value) != first
        until(restarted)
        until(lambda: self.service(operation='status')['running'])
        self.assertEqual(self.service(operation='credentials'), credentials)
        viewer, rfb = self.open(self.h, identity, local=True)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        viewer.close()
        for name in ('desktop-service.json', 'desktop-vnc.password', 'desktop-wayvnc.conf'):
            self.assertEqual((self.config.parent / name).stat().st_mode & 0o777, 0o600)
        self.assertNotIn(password, self.vnc_log.read_text())

        # Restore the same real desktop from the independent node, without a gateway.
        self.h.stop(self.h.web)
        caller = Harness(unified=True)
        self.addCleanup(caller.close)
        code, value = caller.post('/api/peers', {'operation': 'identity'})
        self.assertEqual(code, 200, value)
        caller_id = value['node_id']
        token = self.root / 'desktop.token'
        self.cli('grant', '--allow-unverified', '--node', caller_id, '--output-token', token, '--desktop')
        listener = free_port()
        self.cli('listen', '--address', f'127.0.0.1:{listener}')
        peer = self.h.start(NODE, '--config', self.config.with_name('settings.json'), 'peer', 'serve')
        code, value = caller.post('/api/peers', {'operation': 'connect', 'node': identity,
                                 'url': f'http://127.0.0.1:{listener}', 'token': token.read_text().strip()})
        self.assertEqual(code, 200, value)
        until(lambda: caller.post('/api/peers', {'operation': 'probe', 'node': identity})[0] == 200)
        until(lambda: self.port_ready(port))
        viewer, rfb = self.open(caller, identity)
        self.assertTrue(rfb.authenticate(password))
        rfb.frame()
        self.cli('revoke', '--node', caller_id)
        self.assertEqual(viewer.recv()[0], 8)
        self.assertEqual(caller.post('/api/desktop/ticket', {'node': identity})[0], 400)
        self.assertIsNone(peer.poll())
        self.h.stop(peer)
        until(lambda: not self.port_ready(port))


if __name__ == '__main__':
    unittest.main()
