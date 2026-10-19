"""Opt-in collaboration observations across real nodes, without a browser polling."""
import copy
import hashlib
import http.server
import json
import os
import subprocess
import threading
import time
import unittest

import test_federated_activity as fixture
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(),
                     'build Rust binaries; real sockets required')
class BackgroundActivitySyncTests(unittest.TestCase):
    setUp = fixture.ActivityTests.setUp
    cli = fixture.ActivityTests.cli
    api = fixture.ActivityTests.api
    direct = fixture.ActivityTests.direct
    start_target = fixture.ActivityTests.start_target
    grant = fixture.ActivityTests.enable
    listing = fixture.ActivityTests.snapshot
    envelope = fixture.ActivityTests.envelope

    def sync(self, **command):
        return self.api(operation='activity_sync', command=command or {'operation': 'status'})

    def configure(self, nodes=None):
        return self.sync(operation='configure', revision=self.sync()['revision'],
                         nodes=[self.target] if nodes is None else nodes, interval_seconds=15)

    def cache_path(self, node=None):
        directory = self.path.with_name('peers.json.activity-snapshots')
        return directory / (hashlib.sha256((node or self.target).encode()).hexdigest() + '.json')

    def cached(self):
        try:
            return json.loads(self.cache_path().read_text())
        except (OSError, ValueError):
            return None

    def send(self, text):
        status, value = self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/messages', self.envelope(text=text))
        self.assertEqual(status, 200, value)

    def phase(self, expected):
        return until(lambda: value if (value := self.sync())['nodes'][0]['phase'] == expected else None)

    def pause(self):
        return self.sync(operation='pause', revision=self.sync()['revision'])

    def rejected_read(self):
        status, value = self.h.post('/api/peers', {'operation': 'activity', 'node': self.target})
        self.assertEqual(status, 400, value)
        return value

    def proxy(self, reply, arrived=None, release=None):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                if arrived: arrived.set()
                if release: release.wait(timeout=12)
                code, value = reply[0]
                body = json.dumps(value).encode()
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *args): pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            if release: release.set()
            server.shutdown(); server.server_close(); thread.join(timeout=5)
        self.addCleanup(close)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)

    def test_default_off_manual_reads_do_not_store_and_background_survives_web_restart(self):
        self.grant('--messages', '--activity')
        self.send('Before enabling sync')
        self.assertNotIn('sync', self.listing())
        self.assertFalse(self.sync()['enabled'])
        self.assertFalse(self.path.with_name('activity-sync.json').exists())
        time.sleep(2.2)
        self.assertFalse(self.cache_path().exists())
        self.configure()
        until(lambda: self.cached())
        self.send('Changed with page closed')
        until(lambda: value if (value := self.cached()) and len(value['activity']['messages']) == 2 else None, seconds=22)
        self.h.stop(self.h.web)
        self.send('Receiver restarted')
        self.h.start_web(); self.h.login()
        until(lambda: value if (value := self.cached()) and len(value['activity']['messages']) == 3 else None, seconds=22)
        status = until(lambda: value if (value := self.sync())['nodes'][0]['record_count'] == 3 else None)
        self.assertNotIn('binding', json.dumps(status))
        for path in [self.path.with_name('activity-sync.json'), self.path.with_name('activity-sync-state.json'), self.cache_path()]:
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(self.token, path.read_text())
        self.assertFalse(self.path.exists())  # No receiver task ledger was created.
        for kind in ('project_sync', 'agent_sync'):
            self.assertFalse(self.api(operation=kind, command={'operation': 'status'})['enabled'])

    def test_headless_node_restores_policy_and_cli_without_web_or_gateway(self):
        self.grant('--messages', '--activity')
        self.send('Headless observation')
        self.configure(); self.phase('online')
        self.h.stop(self.h.web); self.h.stop(self.h.server)
        self.send('Gateway is stopped')
        settings = self.path.with_name('settings.json')
        process = self.h.start(NODE, '--config', settings, 'peer', 'serve')
        until(lambda: value if (value := self.cached()) and len(value['activity']['messages']) == 2 else None, seconds=22)
        result = subprocess.run([str(NODE), '--config', str(settings), 'peer', 'activity-sync'], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['enabled'])
        self.assertIsNone(process.poll())

    def test_pause_retains_exact_offline_observation_and_clear_deletes_it(self):
        self.grant('--messages', '--activity')
        self.send('Saved content')
        self.configure(); self.phase('online'); self.pause()
        saved = self.cached()
        self.send('Not synchronized while paused')
        time.sleep(2.2)
        self.assertEqual(self.cached(), saved)
        self.h.stop(self.process)
        offline = self.listing()
        self.assertEqual(offline['sync'], {'state': 'offline', 'synced_at': saved['synced_at']})
        self.assertEqual({key: value for key, value in offline.items() if key != 'sync'}, saved['activity'])
        cleared = self.configure([])
        self.assertFalse(cleared['enabled']); self.assertEqual(cleared['nodes'], [])
        self.assertFalse(self.cache_path().exists())
        self.rejected_read()
        self.start_target()
        self.assertNotIn('sync', self.listing())
        self.assertFalse(self.cache_path().exists())

    def test_denial_invalid_authority_content_or_injected_marker_erases_snapshot(self):
        self.grant('--messages', '--activity')
        self.send('Private record must not enter error messages')
        catalog = self.listing()
        reply = [(200, catalog)]
        self.proxy(reply)
        self.configure(); self.phase('online'); self.pause()
        modified = copy.deepcopy(catalog)
        modified['messages'][0]['message']['text'] = 'Changed without digest'
        duplicate = copy.deepcopy(catalog)
        duplicate['messages'] *= 2
        for response in [(403, {'error': 'Activity revoked'}), (200, dict(catalog, owner='other-device')),
                         (200, dict(catalog, sync={'state': 'offline', 'synced_at': 1})), (200, modified), (200, duplicate)]:
            with self.subTest(response=response[0]):
                reply[0] = response
                self.assertNotIn('Private record', json.dumps(self.rejected_read()))
                self.assertFalse(self.cache_path().exists())
                reply[0] = (200, catalog)
                self.assertEqual(self.listing()['sync']['state'], 'online')
                self.assertTrue(self.cache_path().exists())
        reply[0] = (401, {'error': 'Revoked'})
        self.rejected_read()
        reply[0] = (503, {'error': 'Unreachable'})
        self.rejected_read()  # Denied content cannot later return as offline.

    def test_tampered_or_oversize_cache_cannot_be_used_offline(self):
        self.grant('--activity')
        self.configure(); self.phase('online'); self.pause()
        self.h.stop(self.process)
        path = self.cache_path(); original = path.read_bytes()
        invalid = json.loads(original)
        invalid['activity']['owner'] = 'other-device'
        path.write_text(json.dumps(invalid))
        self.rejected_read()
        with path.open('wb') as output:
            output.truncate(24 * 1024 * 1024 + 4097)
        self.rejected_read()
        path.write_bytes(original)
        self.assertEqual(self.listing()['sync']['state'], 'offline')

    def test_late_background_responses_cannot_recreate_forgotten_or_deselected_content(self):
        self.grant('--activity')
        catalog = self.listing()
        for change in ('clear', 'forget', 'rotate'):
            with self.subTest(change=change):
                arrived, release = threading.Event(), threading.Event()
                self.proxy([(200, catalog)], arrived, release)
                self.configure()
                self.assertTrue(arrived.wait(timeout=8))
                if change == 'clear': self.configure([])
                elif change == 'forget': self.api(operation='forget', node=self.target)
                else:
                    peers = json.loads(self.path.with_name('peers.json').read_text())
                    with open(peers['routes'][self.target]['token_file'], 'w') as token:
                        token.write('b' * 64 + '\n')
                release.set()
                time.sleep(2.2)
                self.assertFalse(self.cache_path().exists())
                if change != 'clear': self.assertEqual(self.sync()['nodes'][0]['phase'], 'connection_changed')

    def test_actual_revocation_removes_cache_before_an_offline_retry(self):
        self.grant('--activity')
        self.configure(); self.phase('online')
        self.cli('revoke', '--node', self.caller)
        self.configure(); self.phase('error')
        self.assertFalse(self.cache_path().exists())
        self.h.stop(self.process)
        self.rejected_read()

    def test_invalid_or_conflicting_policy_and_missing_grant_fail_closed(self):
        self.configure(); self.phase('error')  # Existing message grant is insufficient.
        self.assertFalse(self.cache_path().exists())
        path = self.path.with_name('activity-sync.json'); original = path.read_bytes()
        for command in [dict(operation='pause', revision=0),
                        dict(operation='configure', revision=1, nodes=[self.target], interval_seconds=1),
                        dict(operation='configure', revision=1, nodes=[self.target, self.target], interval_seconds=15),
                        dict(operation='configure', revision=1, nodes=[self.caller], interval_seconds=15)]:
            self.assertEqual(self.h.post('/api/peers', {'operation': 'activity_sync', 'command': command})[0], 400)
            self.assertEqual(path.read_bytes(), original)
        path.write_bytes(b'broken policy')
        time.sleep(2.2)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'activity_sync', 'command': {'operation': 'status'}})[0], 400)
        self.assertEqual(path.read_bytes(), b'broken policy')
        self.assertEqual(self.api(operation='project_sync', command={'operation': 'status'})['nodes'], [])
        self.assertEqual(self.api(operation='agent_sync', command={'operation': 'status'})['nodes'], [])


if __name__ == '__main__':
    unittest.main()
