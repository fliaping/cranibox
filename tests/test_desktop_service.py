"""Managed VNC lifecycle through real Rust Web and its local node."""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from workspace_web_harness import Harness, CORE, NODE, ROOT, WEB, free_port, sockets_allowed, until
from test_cranibox_terminal import Socket
from test_remote_desktops import DesktopServer, SelkiesServer


@unittest.skipUnless(sys.platform == 'linux' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'Linux Rust binaries and real sockets required')
class DesktopServiceTests(unittest.TestCase):
    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory(prefix='cranibox-managed-vnc-')
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)
        executable = self.root / 'x11vnc'
        shutil.copyfile(ROOT / 'tests/desktop_vnc_fixture.py', executable)
        executable.chmod(0o700)
        wayvnc = self.root / 'wayvnc'
        shutil.copyfile(ROOT / 'tests/desktop_vnc_fixture.py', wayvnc)
        wayvnc.chmod(0o700)
        runtime = self.root / 'runtime'
        runtime.mkdir(mode=0o700)
        wayland = getattr(self, 'wayland', False)
        environment = patch.dict(os.environ, {
            'PATH': str(self.root) + os.pathsep + os.environ.get('PATH', ''),
            'DISPLAY': ':42', 'XDG_SESSION_TYPE': 'wayland' if wayland else 'x11',
            'WAYLAND_DISPLAY': 'wayland-42' if wayland else '',
            'XDG_CURRENT_DESKTOP': 'sway' if wayland else '', 'XDG_SESSION_DESKTOP': '',
            'XDG_RUNTIME_DIR': str(runtime),
            'CRANIBOX_DESKTOP_TEST_ROOT': str(self.root),
        })
        environment.start()
        self.addCleanup(environment.stop)
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.config = self.h.root / '.config/cranibox/peers.json'

    def service(self, **command):
        status, body = self.h.post('/api/desktop/service', command)
        self.assertEqual(status, 200, body)
        return body

    def events(self):
        path = self.root / 'events.jsonl'
        if not path.exists():
            return []
        return [json.loads(line) for line in path.read_text().splitlines() if line.endswith('}')]

    def started(self):
        return [event for event in self.events() if event['event'] == 'started']

    def dead(self, pid):
        try:
            os.kill(pid, 0)
            return False
        except ProcessLookupError:
            return True

    def test_viewer_disconnect_crash_recovery_restart_and_explicit_stop(self):
        initial = self.service(operation='status')
        self.assertTrue(initial['installed'])
        self.assertFalse(initial['enabled'])
        self.assertEqual(initial['display'], ':42')
        port = free_port()
        status = self.service(operation='start', display=':42.0', port=port)
        self.assertTrue(status['running'] and status['enabled'])
        first = until(lambda: self.started())[0]
        identity = json.loads(self.config.read_text())['node_id']
        password = self.service(operation='credentials')['password']
        self.assertEqual(len(password), 8)
        self.assertNotIn(password, json.dumps(status))
        self.assertNotIn(password, json.dumps(self.events()))
        for filename in ('desktop-service.json', 'desktop-vnc.password'):
            self.assertEqual((self.config.parent / filename).stat().st_mode & 0o777, 0o600)
        check = self.service(operation='check', address=f'127.0.0.1:{port}')
        self.assertTrue(check['ready'], check)
        self.h.stop(self.h.server)
        code, ticket = self.h.post('/api/desktop/ticket', {'node': identity, 'local': True})
        self.assertEqual(code, 200, ticket)
        viewer = Socket(self.h, ticket['ticket'], path='/api/desktop/ws')
        self.addCleanup(viewer.close)
        self.assertEqual(viewer.status, 101)
        self.assertEqual(viewer.recv(), (2, b'RFB 003.008\n'))
        viewer.close()
        self.assertTrue(self.service(operation='status')['running'])
        self.assertFalse(self.dead(first['pid']))
        os.kill(first['pid'], signal.SIGKILL)
        until(lambda: len(self.started()) >= 2)
        until(lambda: self.service(operation='status')['running'])
        self.assertNotEqual(self.started()[-1]['pid'], first['pid'])
        before_restart = self.started()[-1]['pid']
        self.h.stop(self.h.web)
        until(lambda: self.dead(before_restart))
        self.h.start_web()
        self.h.login()
        until(lambda: len(self.started()) >= 3)
        until(lambda: self.service(operation='status')['running'])
        self.assertEqual(json.loads(self.config.read_text())['node_id'], identity)
        self.assertEqual(self.service(operation='credentials')['password'], password)
        last = self.started()[-1]['pid']
        stopped = self.service(operation='stop')
        self.assertFalse(stopped['enabled'] or stopped['running'])
        until(lambda: self.dead(last))
        time.sleep(2.6)
        self.assertEqual(len(self.started()), 3)
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])

    def test_installation_restart_and_unreadable_record_do_not_enable_or_replay_sharing(self):
        journal = self.config.with_name('desktop-setup.json')
        for previous in [
            json.dumps(dict(phase='running', provider='x11vnc', error=None)).encode(),
            b'x' * 8193,
        ]:
            with self.subTest(previous=previous[:64]):
                self.h.stop(self.h.web)
                journal.write_bytes(previous)
                self.h.start_web()
                self.h.login()
                status = self.service(operation='status')
                self.assertEqual(status['installation']['phase'], 'interrupted')
                self.assertIn('package manager', status['installation']['error'])
                self.assertEqual(status['phase'], 'failed')
                self.assertFalse(status['enabled'] or status['running'])
                self.assertEqual(self.started(), [])
                self.service(operation='stop')
                self.assertEqual(self.service(operation='status')['installation']['phase'], 'interrupted')
                self.assertEqual(journal.read_bytes(), previous)
                for command in [dict(operation='install'), dict(operation='install', plan_id='unreviewed')]:
                    self.assertEqual(self.h.post('/api/desktop/service', command)[0], 400)
                self.assertEqual(journal.read_bytes(), previous)
                self.assertFalse(self.config.with_name('desktop-service.json').exists())
                self.assertFalse(self.config.with_name('desktop-vnc.password').exists())
    def test_failures_preserve_configuration_and_external_disable_stops_service(self):
        for display, port in [('remote:0', 5900), (':65536', 5900), (':0', 80), (':0;touch /tmp/unwanted', 5900)]:
            self.assertEqual(self.h.post('/api/desktop/service', {'operation': 'start', 'display': display, 'port': port})[0], 400)
        with socket.socket() as occupied:
            occupied.bind(('127.0.0.1', 0))
            occupied.listen()
            code, value = self.h.post('/api/desktop/service', {'operation': 'start', 'display': ':42', 'port': occupied.getsockname()[1]})
            self.assertEqual(code, 400)
            self.assertIn('occupied', json.dumps(value))
        marker = self.root / 'fail-start'
        marker.touch()
        port = free_port()
        code, value = self.h.post('/api/desktop/service', {'operation': 'start', 'display': ':42', 'port': port})
        self.assertEqual(code, 400)
        self.assertIn('X11', json.dumps(value))
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])
        self.assertFalse((self.config.parent / 'desktop-service.json').exists())
        marker.unlink()
        self.assertTrue(self.service(operation='start', display=':42', port=port)['running'])
        pid = self.started()[-1]['pid']
        self.assertEqual(self.h.post('/api/peers', {'operation': 'desktop', 'desktop': None})[0], 200)
        until(lambda: self.dead(pid))
        count = len(self.started())
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertFalse(self.service(operation='status')['enabled'])
        time.sleep(.7)
        self.assertEqual(len(self.started()), count)

    def test_service_endpoint_authentication_and_loopback_probe_validation(self):
        endpoint = '/api/desktop/service'
        command = {'operation': 'status'}
        for headers in ({'Content-Type': 'application/json'}, {'Cookie': self.h.cookie, 'Content-Type': 'application/json'}, {**self.h.headers(), 'Origin': 'https://evil.invalid'}):
            self.assertIn(self.h.http('POST', endpoint, command, headers=headers)[0], (401, 403))
        self.assertEqual(self.h.post(endpoint, {'operation': 'credentials'})[0], 400)
        self.assertEqual(self.h.post(endpoint, {'operation': 'status', 'unexpected': True})[0], 400)
        for address in ('0.0.0.0:5900', '192.168.1.2:5900', '127.0.0.1:0'):
            self.assertEqual(self.h.post(endpoint, {'operation': 'check', 'address': address})[0], 400)
        self.assertFalse(self.service(operation='check', address=f'127.0.0.1:{free_port()}')['ready'])
        self.assertEqual(self.h.post(endpoint, {'operation': 'discover', 'address': '192.168.1.2:5900'})[0], 400)

    def test_discovery_reads_vnc_greetings_and_keeps_sharing_disabled(self):
        # Fixed, conventional ports are intentional: discovery accepts no scan range.
        try:
            server = DesktopServer(port=5900)
        except OSError as error:
            self.skipTest(f'Loopback VNC discovery port is unavailable: {error}')
        self.addCleanup(server.close)
        before = self.config.read_bytes()
        found = self.service(operation='discover')
        self.assertEqual(found['checked'], 4)
        self.assertIn({'address': server.address, 'protocol': 'vnc', 'configured': False}, found['services'])
        until(lambda: server.active == 0)
        self.assertEqual(server.received, [])  # No authentication or viewer bytes.
        self.assertEqual(self.config.read_bytes(), before)
        self.assertFalse(self.config.with_name('desktop-service.json').exists())
        self.assertFalse(self.config.with_name('desktop-vnc.password').exists())
        self.assertEqual(self.started(), [])
        for banner in (b'HTTP/1.1 200 OK\r\n', b''):
            server.banner = banner
            started = time.monotonic()
            found = self.service(operation='discover')
            self.assertLess(time.monotonic() - started, 5)
            self.assertNotIn(server.address, [item['address'] for item in found['services']])
            self.assertEqual(self.config.read_bytes(), before)
        server.banner = DesktopServer.banner
        self.assertIn(server.address, [item['address'] for item in self.service(operation='discover')['services']])

    def test_discovery_includes_saved_custom_selkies_without_taking_control(self):
        server = SelkiesServer()
        self.addCleanup(server.close)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'desktop', 'desktop': {'address': server.address, 'protocol': 'selkies'}})[0], 200)
        before = self.config.read_bytes()
        found = self.service(operation='discover')
        self.assertEqual(found['checked'], 5)
        self.assertEqual(found['services'][0], {'address': server.address, 'protocol': 'selkies', 'configured': True})
        until(lambda: server.active == 0)
        self.assertEqual(server.received, [])  # In particular, no SETTINGS takeover.
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(self.started(), [])
        self.assertFalse(self.config.with_name('desktop-service.json').exists())

    def test_discovery_finds_ipv6_vnc_without_enabling_sharing(self):
        try:
            server = DesktopServer(host='::1', port=5900)
        except OSError as error:
            self.skipTest(f'IPv6 loopback discovery port is unavailable: {error}')
        self.addCleanup(server.close)
        before = self.config.read_bytes()
        found = self.service(operation='discover')
        self.assertEqual(found['checked'], 4)
        self.assertIn({'address': server.address, 'protocol': 'vnc', 'configured': False}, found['services'])
        self.assertEqual(self.config.read_bytes(), before)

    def test_selkies_background_service_does_not_offer_xwayland_as_an_x11_desktop(self):
        self.h.stop(self.h.web)
        with patch.dict(os.environ, {
            'PIXELFLUX_WAYLAND': 'true', 'XDG_SESSION_TYPE': '',
            'WAYLAND_DISPLAY': '', 'XDG_CURRENT_DESKTOP': '', 'XDG_SESSION_DESKTOP': '',
        }):
            self.h.start_web()
        self.h.login()
        status = self.service(operation='status')
        self.assertEqual(status['provider'], 'wayvnc')
        self.assertTrue(status['wayland'] and status['selkies_wayland'])
        self.assertFalse(status['supported'])
        self.assertEqual(status['display'], '')
        self.assertIn('Selkies browser entry', status['unsupported_reason'])
        for command in (
            dict(operation='start', display=':42', port=free_port()),
            dict(operation='start', provider='wayvnc', display='wayland-0', port=free_port()),
            dict(operation='plan'), dict(operation='install', plan_id='unreviewed'),
        ):
            code, body = self.h.post('/api/desktop/service', command)
            self.assertEqual(code, 400, body)
        self.assertEqual(self.started(), [])
        self.assertFalse(self.config.with_name('desktop-service.json').exists())
        self.assertFalse(self.config.with_name('desktop-vnc.password').exists())
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])

    def test_headless_peer_restores_the_same_managed_service_without_a_gateway(self):
        port = free_port()
        self.service(operation='start', display=':42', port=port)
        pid = self.started()[-1]['pid']
        listener = {'operation': 'listener', 'listener': {'address': f'127.0.0.1:{free_port()}', 'allow_insecure': False}}
        self.assertEqual(self.h.post('/api/peers', listener)[0], 200)
        self.h.stop(self.h.web)
        self.h.stop(self.h.server)
        until(lambda: self.dead(pid))
        settings = self.config.with_name('settings.json')
        peer = self.h.start(NODE, '--config', settings, 'peer', 'serve')
        until(lambda: len(self.started()) >= 2)
        pid = self.started()[-1]['pid']
        with socket.create_connection(('127.0.0.1', port), timeout=3) as stream:
            self.assertEqual(stream.recv(12), b'RFB 003.008\n')
        result = subprocess.run([str(NODE), '--config', str(settings), 'peer', 'desktop'], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        until(lambda: self.dead(pid))
        self.assertIsNone(peer.poll())


@unittest.skipUnless(sys.platform == 'linux' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'Linux Rust binaries and real sockets required')
class WaylandDesktopServiceTests(unittest.TestCase):
    wayland = True
    setUp = DesktopServiceTests.setUp
    service = DesktopServiceTests.service
    events = DesktopServiceTests.events
    started = DesktopServiceTests.started
    dead = DesktopServiceTests.dead

    def start(self, **options):
        return self.service(operation='start', provider='wayvnc', display='wayland-42', port=options.pop('port', free_port()), **options)

    def test_wayland_process_restarts_with_private_auth_and_the_same_output(self):
        # Service privacy must not depend on the launching shell's umask.
        self.h.stop(self.h.web)
        previous_umask = os.umask(0)
        try:
            self.h.start_web()
        finally:
            os.umask(previous_umask)
        self.h.login()
        initial = self.service(operation='status')
        self.assertTrue(initial['supported'] and initial['installed'] and initial['wayland'])
        self.assertEqual((initial['provider'], initial['package'], initial['display']), ('wayvnc', 'wayvnc', 'wayland-42'))
        port = free_port()
        status = self.start(port=port, output='DP-1')
        self.assertTrue(status['running'] and status['enabled'])
        first = until(lambda: self.started())[0]
        socket_path = Path(first['args'][first['args'].index('--socket') + 1])
        self.assertEqual(socket_path.parent.stat().st_mode & 0o777, 0o700)
        config = self.config.with_name('desktop-wayvnc.conf')
        self.assertEqual(config.stat().st_mode & 0o777, 0o600)
        credentials = self.service(operation='credentials')
        self.assertEqual(credentials['username'], 'cranibox')
        self.assertNotIn(credentials['password'], json.dumps(status) + json.dumps(self.events()))
        self.assertTrue(self.service(operation='check', address=f'127.0.0.1:{port}')['ready'])
        # A local config edit cannot silently disable auth on a managed restart.
        config.write_text('enable_auth=false\n')
        os.kill(first['pid'], signal.SIGKILL)
        until(lambda: len(self.started()) >= 2)
        until(lambda: self.service(operation='status')['running'])
        self.assertIn('enable_auth=true', config.read_text())
        until(lambda: not socket_path.parent.exists())
        pid = self.started()[-1]['pid']
        self.h.stop(self.h.web)
        until(lambda: self.dead(pid))
        self.h.start_web()
        self.h.login()
        until(lambda: len(self.started()) >= 3)
        restored = self.service(operation='status')
        self.assertEqual((restored['provider'], restored['output'], restored['port']), ('wayvnc', 'DP-1', port))
        self.assertEqual(self.service(operation='credentials'), credentials)
        final = self.started()[-1]
        final_socket = Path(final['args'][final['args'].index('--socket') + 1])
        self.assertEqual(final['args'][final['args'].index('--output') + 1], 'DP-1')
        self.service(operation='stop')
        until(lambda: self.dead(final['pid']))
        until(lambda: not final_socket.parent.exists())
        count = len(self.started())
        time.sleep(1)
        self.assertEqual(len(self.started()), count)
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])

    def test_wayland_validation_old_versions_and_failed_launch_do_not_enable_sharing(self):
        for fields in [
            {'display': '../wayland-42'}, {'display': '/tmp/wayland-42'},
            {'output': 'DP-1\nenable_auth=false'}, {'output': '--config=/tmp/conf'},
            {'provider': 'x11vnc', 'display': ':42'}, {'provider': 'unknown'}, {'port': 80},
        ]:
            command = dict(operation='start', provider='wayvnc', display='wayland-42', port=free_port())
            command.update(fields)
            self.assertEqual(self.h.post('/api/desktop/service', command)[0], 400)
        version = self.root / 'wayvnc-version'
        version.write_text('0.7.2')
        code, body = self.h.post('/api/desktop/service', dict(operation='start', provider='wayvnc', display='wayland-42', port=free_port()))
        self.assertEqual(code, 400, body)
        self.assertIn('0.8', json.dumps(body))
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])
        version.unlink()
        marker = self.root / 'fail-start'
        marker.touch()
        code, body = self.h.post('/api/desktop/service', dict(operation='start', provider='wayvnc', display='wayland-42', port=free_port()))
        self.assertEqual(code, 400, body)
        self.assertIn('wlroots', json.dumps(body))
        self.assertIsNone(json.loads(self.config.read_text())['desktop'])
        self.assertFalse(self.config.with_name('desktop-service.json').exists())
        marker.unlink()
        self.assertTrue(self.start()['running'])
        pid = self.started()[-1]['pid']
        self.assertEqual(self.h.post('/api/peers', {'operation': 'desktop', 'desktop': None})[0], 200)
        until(lambda: self.dead(pid))

    def test_unsupported_compositors_are_explained_before_launch(self):
        for desktop in ('GNOME', 'KDE', 'Weston'):
            self.h.stop(self.h.web)
            with patch.dict(os.environ, {'XDG_CURRENT_DESKTOP': desktop}):
                self.h.start_web()
            self.h.login()
            status = self.service(operation='status')
            self.assertEqual(status['provider'], 'wayvnc')
            self.assertFalse(status['supported'])
            self.assertIn('does not support', status['unsupported_reason'])
            for command in [dict(operation='start', provider='wayvnc', display='wayland-42', port=free_port()), dict(operation='plan'), dict(operation='install', plan_id='unreviewed')]:
                code, value = self.h.post('/api/desktop/service', command)
                self.assertEqual(code, 400, value)
            self.assertFalse(self.service(operation='status')['running'])
        self.assertEqual(self.started(), [])


if __name__ == '__main__':
    unittest.main()
