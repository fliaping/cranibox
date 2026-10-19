"""Real node-owned public Agent snapshots, independent of browser polling."""
import http.server
import hashlib
import json
import os
import subprocess
import threading
import time
import unittest
import uuid

import test_web_direct_peers as fixture
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(),
                     'build Rust binaries; real sockets required')
class BackgroundAgentSyncTests(unittest.TestCase):
    setUp = fixture.WebDirectPeersTests.setUp
    cli = fixture.WebDirectPeersTests.cli
    api = fixture.WebDirectPeersTests.api

    def grant_messages(self):
        self.cli('revoke', '--node', self.caller)
        self.token_path = self.h.root / 'messages.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', self.token_path, '--messages')
        self.token = self.token_path.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)

    def sync(self, **command):
        return self.api(operation='agent_sync', command=command or {'operation': 'status'})

    def enable(self, nodes=None):
        return self.sync(operation='configure', revision=self.sync()['revision'],
                         nodes=nodes or [self.target], interval_seconds=15)

    def register(self, name):
        return self.cli('agent-sessions', '--command', json.dumps({
            'operation': 'register', 'id': str(uuid.uuid4()), 'name': name, 'harness': 'test-cli'}))

    def snapshot(self):
        directory = self.path.with_name(self.path.name + '.agent-snapshots')
        return next(directory.glob('*.json'), None)

    def cached_sessions(self):
        try:
            path = self.snapshot()
            return json.loads(path.read_text())['catalog']['sessions'] if path else []
        except (OSError, ValueError):
            return []

    def listing(self):
        return self.api(operation='agent_sessions', node=self.target, command={'operation': 'list'})

    def test_default_off_then_periodic_reads_and_restart_without_opening_tasks(self):
        self.grant_messages()
        first = self.register('Before opening tasks')
        initial = self.sync()
        self.assertFalse(initial['enabled'])
        self.assertFalse(self.path.with_name('agent-sync.json').exists())
        time.sleep(2.2)
        self.assertIsNone(self.snapshot())
        self.enable()
        until(lambda: self.cached_sessions() == first['sessions'])
        second = self.register('Changed with page closed')
        until(lambda: self.cached_sessions() == second['sessions'], seconds=22)
        self.h.stop(self.h.web)
        third = self.register('Receiver restarted')
        self.h.start_web()
        self.h.login()
        until(lambda: self.cached_sessions() == third['sessions'], seconds=22)
        value = until(lambda: (v if (v := self.sync())['nodes'][0]['session_count'] == 3 else None))
        self.assertTrue(value['enabled'])
        self.assertFalse(self.api(operation='project_sync', command={'operation': 'status'})['enabled'])
        for path in [self.path.with_name('agent-sync.json'), self.path.with_name('agent-sync-state.json'), self.snapshot()]:
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(self.token, path.read_text())
        self.assertNotIn('binding', json.dumps(value))

    def test_standalone_node_restores_policy_without_gateway_or_web(self):
        self.grant_messages()
        expected = self.register('Headless sync')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        self.h.stop(self.h.web)
        self.h.stop(self.h.server)
        expected = self.register('Changed without gateway')
        process = self.h.start(NODE, '--config', self.path.with_name('settings.json'), 'peer', 'serve')
        until(lambda: self.cached_sessions() == expected['sessions'], seconds=22)
        self.assertIsNone(process.poll())
        result = subprocess.run([str(NODE), '--config', str(self.path.with_name('settings.json')), 'peer', 'agent-sync'],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['enabled'])

    def test_offline_snapshot_keeps_original_lease_and_denial_erases_it(self):
        self.grant_messages()
        expected = self.register('Public report')
        online = self.listing()
        self.assertEqual(online['sync']['state'], 'online')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        saved_observation = json.loads(self.snapshot().read_text())['catalog']['observed_at']
        self.h.stop(self.target_process)
        offline = self.listing()
        self.assertEqual(offline['sync']['state'], 'offline')
        self.assertEqual(offline['sessions'], expected['sessions'])
        self.assertEqual(offline['observed_at'], saved_observation)
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'offline')
        self.cli('revoke', '--node', self.caller)
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'error')
        self.assertIsNone(self.snapshot())
        self.h.stop(self.target_process)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'agent_sessions', 'node': self.target,
                         'command': {'operation': 'list'}})[0], 400)

    def test_pause_and_forgotten_recreated_route_require_explicit_review(self):
        self.grant_messages()
        first = self.register('Kept report')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        paused = self.sync(operation='pause', revision=self.sync()['revision'])
        self.assertFalse(paused['enabled'])
        self.register('Not yet synchronized')
        time.sleep(2.2)
        self.assertEqual(self.cached_sessions(), first['sessions'])
        self.api(operation='forget', node=self.target)
        self.assertIsNone(self.snapshot())
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        self.assertEqual(self.sync()['nodes'][0]['phase'], 'connection_changed')
        self.enable()
        until(lambda: len(self.cached_sessions()) == 2)

    def proxy(self, handler):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                code, value = handler(self.headers['X-Cranibox-Target'])
                body = json.dumps(value).encode()
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.addCleanup(close)
        return f'http://127.0.0.1:{server.server_port}'

    def test_invalid_authority_or_injected_offline_marker_never_hides_failure(self):
        self.grant_messages()
        catalog = self.register('Read-only labels')
        reply = dict(catalog)
        url = self.proxy(lambda _: (200, reply))
        self.api(operation='connect', node=self.target, url=url, token=self.token)
        self.listing()
        self.assertIsNotNone(self.snapshot())
        for invalid in [dict(catalog, owner='other-device'), dict(catalog, sync={'state': 'offline', 'synced_at': 1})]:
            reply.clear()
            reply.update(invalid)
            self.assertEqual(self.h.post('/api/peers', {'operation': 'agent_sessions', 'node': self.target,
                             'command': {'operation': 'list'}})[0], 400)
            self.assertIsNone(self.snapshot())
            reply.clear()
            reply.update(catalog)
            self.listing()

    def test_response_after_forget_cannot_repopulate_snapshot(self):
        self.grant_messages()
        catalog = self.register('Late report')
        entered, release = threading.Event(), threading.Event()
        def handler(_):
            entered.set()
            release.wait(timeout=10)
            return 200, catalog
        url = self.proxy(handler)
        self.addCleanup(release.set)
        self.api(operation='connect', node=self.target, url=url, token=self.token)
        self.enable()
        self.assertTrue(entered.wait(timeout=8))
        self.api(operation='forget', node=self.target)
        release.set()
        time.sleep(2.2)
        self.assertIsNone(self.snapshot())
        self.assertEqual(self.sync()['nodes'][0]['phase'], 'connection_changed')

    def test_agent_project_and_activity_pulls_share_four_slots_and_pause_stops_queue(self):
        release = threading.Event()
        lock = threading.Lock()
        observed = {'active': 0, 'peak': 0, 'started': 0}
        class Handler(http.server.BaseHTTPRequestHandler):
            def answer(self, projects=False):
                if projects:
                    self.rfile.read(int(self.headers.get('Content-Length', 0)))
                node = self.headers['X-Cranibox-Target']
                with lock:
                    observed['active'] += 1
                    observed['started'] += 1
                    observed['peak'] = max(observed['peak'], observed['active'])
                release.wait(timeout=15)
                value = {'owner': node, 'permissions': {'projects': True}, 'revision': 0, 'projects': []} if projects else {
                    'owner': node, 'observed_at': int(time.time()), 'lease_secs': 300, 'capacity': 64, 'sessions': []}
                if self.path.endswith('/activity'):
                    value = {'version': 1, 'owner': node, 'observed_at': int(time.time()), 'tasks': [], 'messages': [], 'runs': [],
                             'digest': hashlib.sha256(json.dumps([1, node, [], [], []], separators=(',', ':')).encode()).hexdigest()}
                body = json.dumps(value).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                with lock:
                    observed['active'] -= 1
            def do_GET(self):
                self.answer()
            def do_POST(self):
                self.answer(projects=True)
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            release.set()
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.addCleanup(close)
        nodes = [f'bounded-source-{index}' for index in range(5)]
        for node in nodes:
            self.api(operation='connect', node=node, url=f'http://127.0.0.1:{server.server_port}', token=self.token)
        self.enable(nodes)
        self.api(operation='project_sync', command={'operation': 'configure', 'revision': 0, 'nodes': nodes, 'interval_seconds': 15})
        self.api(operation='activity_sync', command={'operation': 'configure', 'revision': 0, 'nodes': nodes, 'interval_seconds': 15})
        until(lambda: observed['active'] == 4)
        self.sync(operation='pause', revision=self.sync()['revision'])
        self.api(operation='project_sync', command={'operation': 'pause', 'revision': 1})
        self.api(operation='activity_sync', command={'operation': 'pause', 'revision': 1})
        release.set()
        until(lambda: observed['active'] == 0)
        time.sleep(2.2)
        self.assertEqual(observed['peak'], 4)
        self.assertEqual(observed['started'], 4)

    def test_missing_permission_and_invalid_or_conflicting_policy_fail_closed(self):
        self.register('No message permission')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'error')
        self.assertIsNone(self.snapshot())
        path = self.path.with_name('agent-sync.json')
        original = path.read_bytes()
        for command in [dict(operation='pause', revision=0),
                        dict(operation='configure', revision=1, nodes=[], interval_seconds=15),
                        dict(operation='configure', revision=1, nodes=[self.target], interval_seconds=1)]:
            self.assertEqual(self.h.post('/api/peers', {'operation': 'agent_sync', 'command': command})[0], 400)
            self.assertEqual(path.read_bytes(), original)
        broken = b'broken policy'
        path.write_bytes(broken)
        time.sleep(2.2)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'agent_sync', 'command': {'operation': 'status'}})[0], 400)
        self.assertEqual(path.read_bytes(), broken)
        self.assertEqual(self.api(operation='project_sync', command={'operation': 'status'})['nodes'], [])


if __name__ == '__main__':
    unittest.main()
