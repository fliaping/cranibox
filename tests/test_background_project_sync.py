"""Real node-owned catalog synchronization with no project-page requests."""
import http.server
import json
import os
import subprocess
import threading
import time
import unittest

import test_web_direct_peers as fixture
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(),
                     'build Rust binaries; real sockets required')
class BackgroundProjectSyncTests(unittest.TestCase):
    setUp = fixture.WebDirectPeersTests.setUp
    cli = fixture.WebDirectPeersTests.cli
    api = fixture.WebDirectPeersTests.api
    project_grant = fixture.WebDirectPeersTests.project_grant

    def sync(self, **command):
        return self.api(operation='project_sync', command=command or {'operation': 'status'})

    def enable(self, nodes=None, interval=15):
        return self.sync(operation='configure', revision=self.sync()['revision'],
                         nodes=nodes or [self.target], interval_seconds=interval)

    def owner(self, name):
        status, value = self.h.http('POST', f'/api/v1/mesh/nodes/{self.target}/projects',
            {'operation': 'register', 'name': name, 'node': self.target, 'path': str(self.h.root / name)}, port=self.port,
            headers={'Authorization': 'Bearer ' + self.token, 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'})
        self.assertEqual(status, 200, value)
        return value

    def snapshot(self):
        directory = self.path.with_name(self.path.name + '.project-snapshots')
        return next(directory.glob('*.json'), None)

    def cached_projects(self):
        try:
            snapshot = self.snapshot()
            return json.loads(snapshot.read_text())['catalog']['projects'] if snapshot else []
        except (OSError, ValueError):
            return []

    def test_default_off_then_periodic_reads_and_restart_work_without_project_page(self):
        self.project_grant()
        first = self.owner('before-page-open')
        value = self.sync()
        self.assertEqual(value['owner'], self.caller)
        self.assertFalse(value['enabled'])
        self.assertEqual(value['nodes'], [])
        self.assertFalse(self.path.with_name('project-sync.json').exists())
        time.sleep(2.2)
        self.assertIsNone(self.snapshot())
        self.enable()
        until(lambda: self.cached_projects() == first['projects'])
        second = self.owner('changed-with-page-closed')
        until(lambda: self.cached_projects() == second['projects'], seconds=22)
        state = until(lambda: (v if (v := self.sync())['nodes'][0]['phase'] == 'online' else None))
        self.assertEqual(state['nodes'][0]['project_count'], 2)
        self.assertGreater(state['nodes'][0]['last_success'], 0)
        self.h.stop(self.h.web)
        third = self.owner('changed-while-receiver-stopped')
        self.h.start_web()
        self.h.login()
        until(lambda: self.cached_projects() == third['projects'], seconds=22)
        state = self.sync()
        self.assertTrue(state['enabled'])
        for name in ['project-sync.json', 'project-sync-state.json']:
            path = self.path.with_name(name)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(self.token, path.read_text())
        self.assertNotIn('binding', json.dumps(state))
        self.assertNotIn('token_file', json.dumps(state))

    def test_standalone_peer_service_restores_sync_without_workspace_gateway(self):
        self.project_grant()
        expected = self.owner('headless-synchronization')
        self.h.stop(self.h.web)
        self.h.stop(self.h.server)
        settings = self.path.with_name('settings.json')
        command = json.dumps({'operation': 'configure', 'revision': 0, 'nodes': [self.target], 'interval_seconds': 15})
        result = subprocess.run([str(NODE), '--config', str(settings), 'peer', 'project-sync', '--command', command], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        process = self.h.start(NODE, '--config', settings, 'peer', 'serve')
        until(lambda: self.cached_projects() == expected['projects'])
        self.assertIsNone(process.poll())
        result = subprocess.run([str(NODE), '--config', str(settings), 'peer', 'project-sync'], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['enabled'])

    def test_pause_preserves_snapshot_and_requires_explicit_resume(self):
        self.project_grant()
        first = self.owner('kept-snapshot')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        paused = self.sync(operation='pause', revision=self.sync()['revision'])
        self.assertFalse(paused['enabled'])
        self.assertEqual(paused['nodes'][0]['phase'], 'paused')
        self.assertGreater(paused['nodes'][0]['last_success'], 0)
        second = self.owner('wait-for-resume')
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        time.sleep(2.2)
        self.assertEqual(self.cached_projects(), first['projects'])
        self.assertFalse(self.sync()['enabled'])
        self.enable()
        until(lambda: self.cached_projects() == second['projects'])

    def test_background_reader_yields_to_an_interactive_catalog_request(self):
        import fcntl
        self.project_grant()
        first = self.owner('interactive-snapshot')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        previous_attempt = self.sync()['nodes'][0]['last_attempt']
        with open(str(self.snapshot()) + '.lock', 'r+') as locked:
            fcntl.flock(locked, fcntl.LOCK_EX)
            second = self.owner('waiting-background-update')
            self.enable()
            until(lambda: (v := self.sync())['nodes'][0]['phase'] == 'waiting' and v['nodes'][0]['last_attempt'] > previous_attempt)
            self.assertEqual(self.cached_projects(), first['projects'])
            fcntl.flock(locked, fcntl.LOCK_UN)
        until(lambda: self.cached_projects() == second['projects'], seconds=22)

    def test_offline_keeps_snapshot_but_denial_clears_it(self):
        self.project_grant()
        expected = self.owner('private-catalog')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        self.h.stop(self.target_process)
        self.enable()
        offline = until(lambda: (v if (v := self.sync())['nodes'][0]['phase'] == 'offline' else None))
        self.assertEqual(self.cached_projects(), expected['projects'])
        self.assertIsNotNone(offline['nodes'][0]['last_success'])
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': self.target})[0] == 200)
        self.cli('revoke', '--node', self.caller)
        self.enable()
        denied = until(lambda: (v if (v := self.sync())['nodes'][0]['phase'] == 'error' else None))
        self.assertIsNone(self.snapshot())
        self.assertNotIn('API 401', json.dumps(denied))
        self.assertNotIn(str(self.h.root), json.dumps(denied))

    def test_forget_and_recreate_same_target_requires_selection_again(self):
        self.project_grant()
        self.owner('before-forget')
        self.enable()
        until(lambda: self.sync()['nodes'][0]['phase'] == 'online')
        self.api(operation='forget', node=self.target)
        self.assertIsNone(self.snapshot())
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        changed = self.sync()['nodes'][0]
        self.assertFalse(changed['connection_valid'])
        self.assertEqual(changed['phase'], 'connection_changed')
        time.sleep(2.2)
        self.assertIsNone(self.snapshot())
        self.enable()
        until(lambda: self.snapshot() is not None)

    def test_invalid_configuration_and_stale_revision_preserve_policy(self):
        self.enable()
        path = self.path.with_name('project-sync.json')
        original = path.read_bytes()
        cases = [
            {'revision': 0, 'nodes': [self.target], 'interval_seconds': 15},
            {'revision': 1, 'nodes': [], 'interval_seconds': 15},
            {'revision': 1, 'nodes': [self.target] * 2, 'interval_seconds': 15},
            {'revision': 1, 'nodes': ['unknown-device'], 'interval_seconds': 15},
            {'revision': 1, 'nodes': [self.caller], 'interval_seconds': 15},
            {'revision': 1, 'nodes': [self.target], 'interval_seconds': 1},
            {'revision': 1, 'nodes': [self.target], 'interval_seconds': 15, 'unknown': True},
        ]
        for case in cases:
            with self.subTest(case=case):
                status, value = self.h.post('/api/peers', {'operation': 'project_sync', 'command': {'operation': 'configure', **case}})
                self.assertIn(status, [400, 422], value)
                self.assertEqual(path.read_bytes(), original)

    def test_corrupt_or_wrong_owner_policy_fails_closed_without_replacing_data(self):
        self.enable()
        path = self.path.with_name('project-sync.json')
        policy = json.loads(path.read_text())
        for payload in [b'broken', json.dumps({**policy, 'owner': 'different-device'}).encode()]:
            path.write_bytes(payload)
            status, _ = self.h.post('/api/peers', {'operation': 'project_sync', 'command': {'operation': 'status'}})
            self.assertEqual(status, 400)
            time.sleep(2.2)
            self.assertEqual(path.read_bytes(), payload)

    def test_at_most_four_reads_and_pause_does_not_start_queued_device(self):
        gate = threading.Event()
        lock = threading.Lock()
        observed = {'active': 0, 'peak': 0, 'nodes': set()}
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length', 0)))
                node = self.headers['X-Cranibox-Target']
                with lock:
                    observed['active'] += 1
                    observed['peak'] = max(observed['peak'], observed['active'])
                    observed['nodes'].add(node)
                gate.wait(timeout=10)
                body = json.dumps({'owner': node, 'permissions': {'projects': True}, 'revision': 0, 'projects': []}).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                with lock:
                    observed['active'] -= 1
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            gate.set()
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.addCleanup(close)
        nodes = [f'isolated-target-{i}' for i in range(5)]
        for node in nodes:
            self.api(operation='connect', node=node, url=f'http://127.0.0.1:{server.server_port}', token=self.token)
        self.enable(nodes)
        until(lambda: observed['active'] == 4)
        self.sync(operation='pause', revision=self.sync()['revision'])
        gate.set()
        until(lambda: observed['active'] == 0)
        time.sleep(2.2)
        self.assertEqual(observed['peak'], 4)
        self.assertEqual(len(observed['nodes']), 4)


if __name__ == '__main__':
    unittest.main()
