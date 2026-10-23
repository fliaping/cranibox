"""Real owner-authenticated incremental catalog pulls through Web's Rust adapter."""
import copy
import http.client
import http.server
import json
import os
import threading
import unittest
import uuid

import test_web_direct_peers as direct_fixture
import test_web_host_node as host_fixture
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(),
                     'build Rust binaries; real sockets required')
class ProjectCatalogSyncTests(unittest.TestCase):
    setUp = direct_fixture.WebDirectPeersTests.setUp
    cli = direct_fixture.WebDirectPeersTests.cli
    api = direct_fixture.WebDirectPeersTests.api
    project_grant = direct_fixture.WebDirectPeersTests.project_grant

    def owner(self, suffix='', command=None, headers=None):
        return self.h.http('POST', f'/api/v1/mesh/nodes/{self.target}/projects{suffix}',
                           command or {'operation': 'list'}, port=self.port, headers={
                               'Authorization': 'Bearer ' + self.token,
                               'X-Cranibox-Target': self.target,
                               'Content-Type': 'application/json', **(headers or {})})

    def register(self, label):
        status, catalog = self.owner(command={'operation': 'register', 'name': label,
            'node': self.target, 'path': str(self.h.root / label)})
        self.assertEqual(status, 200, catalog)
        return catalog

    def listing(self):
        return self.api(operation='projects', node=self.target, command={'operation': 'list'})

    def snapshot(self):
        return next(self.path.with_name(self.path.name + '.project-snapshots').glob('*.json'))

    def proxy(self):
        fixture = self
        self.events = []
        self.behavior = {}
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                request = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                is_sync = self.path.endswith('/projects/sync')
                connection = http.client.HTTPConnection('127.0.0.1', fixture.port, timeout=5)
                try:
                    connection.request('POST', self.path, request, {
                        'Authorization': self.headers['Authorization'],
                        'X-Cranibox-Target': self.headers['X-Cranibox-Target'],
                        'Content-Type': 'application/json'})
                    response = connection.getresponse()
                    status, body = response.status, response.read()
                except OSError:
                    status, body = 503, b'{"error":"isolated target offline"}'
                finally:
                    connection.close()
                value = json.loads(body)
                if fixture.behavior.get('legacy') and not is_sync and status == 200:
                    value.pop('sync_protocol', None)
                if is_sync:
                    if 'status' in fixture.behavior:
                        status, value = fixture.behavior['status'], {'error': 'test protocol status'}
                    elif 'corrupt' in fixture.behavior and status == 200:
                        fixture.behavior['corrupt'](value)
                body = json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()
                # Never record authorization headers or credentials.
                fixture.events.append({'path': self.path, 'request': json.loads(request),
                                       'status': status, 'value': value, 'bytes': len(body)})
                self.send_response(status)
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
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)

    def test_pulls_only_changes_and_deletions_then_recovers_offline_and_after_restart(self):
        self.project_grant()
        first = self.register('first-' + 'x' * 70)
        # Add a second replica to this project: a partial deletion must replace
        # the project, rather than delete the remaining replica or Shell binding.
        status, before = self.owner(command={'operation': 'register', 'project_id': first['projects'][0]['id'],
            'name': 'same logical project', 'node': self.target, 'path': str(self.h.root / 'other-copy')})
        self.assertEqual(status, 200, before)
        before = self.register('second-' + 'y' * 65)
        owned_file = self.target_config.with_name('node-projects.json')
        original = owned_file.read_bytes()
        self.proxy()
        full = self.listing()
        self.assertEqual(full['projects'], before['projects'])
        full_bytes = self.events[-1]['bytes']
        self.assertFalse(self.events[-1]['path'].endswith('/sync'))
        self.assertEqual(self.listing(), full)
        unchanged = self.events[-1]
        self.assertTrue(unchanged['path'].endswith('/projects/sync'))
        self.assertEqual(unchanged['value']['delta']['changed'], [])
        self.assertEqual(unchanged['value']['delta']['removed'], [])
        self.assertLess(unchanged['bytes'], full_bytes)
        self.assertNotIn(str(self.h.root), json.dumps(unchanged['request']))
        self.assertNotIn(self.token, self.snapshot().read_text())
        self.assertEqual(self.snapshot().stat().st_mode & 0o777, 0o600)
        replica = before['projects'][0]['replicas'][0]['id']
        self.assertEqual(self.owner(command={'operation': 'forget', 'replica_id': replica})[0], 200)
        last = self.register('new-after-offline')
        second = last['projects'][1]['replicas'][0]['id']
        status, expected = self.owner(command={'operation': 'forget', 'replica_id': second})
        self.assertEqual(status, 200)
        fresh = self.listing()
        delta = self.events[-1]['value']['delta']
        self.assertEqual(fresh['projects'], expected['projects'])
        self.assertEqual(len(delta['changed']), 2)
        self.assertEqual(delta['removed'], [before['projects'][1]['id']])
        # Observe offline state through a second Web process, then reconcile an
        # owner restored to an older revision. Revisions are not trusted clocks.
        self.h.stop(self.target_process)
        offline = self.listing()
        self.assertEqual(offline['sync']['state'], 'offline')
        self.assertEqual(offline['projects'], expected['projects'])
        self.assertFalse(offline['permissions']['terminals'])
        owned_file.write_bytes(original)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.listing(), offline)
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        until(lambda: self.owner()[0] == 200)
        restored = self.listing()
        self.assertEqual(restored['projects'], before['projects'])
        self.assertLess(restored['revision'], fresh['revision'])
        self.assertTrue(self.events[-1]['path'].endswith('/sync'))
        self.assertNotIn('sync', restored)

    def test_delta_checks_current_authorization_and_never_falls_back_after_denial(self):
        self.project_grant()
        self.register('private')
        self.proxy()
        self.listing()
        self.listing()
        cache = self.snapshot()
        self.cli('revoke', '--node', self.caller)
        count = len(self.events)
        status, value = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})
        self.assertEqual(status, 400, value)
        self.assertIn('401', value['error'])
        self.assertEqual(len(self.events), count + 1)
        self.assertTrue(self.events[-1]['path'].endswith('/sync'))
        self.assertFalse(cache.exists())

    def test_stopped_forward_is_offline_but_cannot_hide_denial_on_another_path(self):
        self.project_grant()
        self.register('forwarded-project')
        full = self.listing()
        cache = self.snapshot()
        configuration = json.loads(self.path.read_text())
        route = configuration['routes'][self.target]
        route['local_forward_lock'] = str(self.h.root / 'stopped-forward.lock')
        self.path.write_text(json.dumps(configuration))
        offline = self.listing()
        self.assertEqual(offline['sync']['state'], 'offline')
        self.assertEqual(offline['projects'], full['projects'])
        route['alternatives'] = [{'url': f'http://localhost:{self.port}', 'allow_insecure': False}]
        self.path.write_text(json.dumps(configuration))
        self.assertNotIn('sync', self.listing())
        self.cli('revoke', '--node', self.caller)
        status, value = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target,
                                                'command': {'operation': 'list'}})
        self.assertEqual(status, 400, value)
        self.assertIn('401', value['error'])
        self.assertFalse(cache.exists())

    def test_rejects_bad_deltas_and_clears_cache_without_replaying_or_hiding_errors(self):
        self.project_grant()
        self.register('metadata')
        self.proxy()
        def wrong_owner(value):
            value['owner'] = 'someone-else'
        def no_permission(value):
            value['permissions']['projects'] = False
        def wrong_protocol(value):
            value['sync_protocol'] = 'unknown-future-protocol'
        def wrong_base(value):
            value['delta']['base'] = 'f' * 64
        def wrong_digest(value):
            value['delta']['digest'] = 'f' * 64
        def missing_record(value):
            value['delta']['order'] = []
        def duplicate_order(value):
            value['delta']['order'] *= 2
        def unknown_deletion(value):
            value['delta']['removed'] = [str(uuid.uuid4())]
        for corrupt in [wrong_owner, no_permission, wrong_protocol, wrong_base, wrong_digest,
                        missing_record, duplicate_order, unknown_deletion]:
            with self.subTest(corrupt=corrupt.__name__):
                self.behavior.clear()
                self.listing()
                cache = self.snapshot()
                self.behavior['corrupt'] = corrupt
                count = len(self.events)
                status, value = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target,
                                                        'command': {'operation': 'list'}})
                self.assertEqual(status, 400, value)
                self.assertEqual(len(self.events), count + 1)
                self.assertFalse(cache.exists())

    def test_legacy_owners_and_explicit_endpoint_rollback_keep_full_reads_compatible(self):
        self.project_grant()
        self.register('compatible')
        self.proxy()
        self.behavior['legacy'] = True
        full = self.listing()
        self.listing()
        self.assertTrue(all(not event['path'].endswith('/sync') for event in self.events))
        self.assertNotIn('sync_protocol', full)
        self.behavior.clear()
        self.listing()  # Now the full reply advertises incremental support.
        self.behavior['status'] = 404
        count = len(self.events)
        self.assertEqual(self.listing()['projects'], full['projects'])
        self.assertEqual(len(self.events), count + 2)
        self.assertTrue(self.events[-2]['path'].endswith('/sync'))
        self.assertFalse(self.events[-1]['path'].endswith('/sync'))
        self.behavior['status'] = 400
        count = len(self.events)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'projects', 'node': self.target,
                                                  'command': {'operation': 'list'}})[0], 400)
        self.assertEqual(len(self.events), count + 1)

    def test_endpoint_bounds_index_and_uses_target_and_origin_authentication(self):
        self.project_grant()
        self.register('index-check')
        self.proxy()
        self.listing()
        self.listing()
        index = self.events[-1]['request']
        for headers, expected in [({'Authorization': 'Bearer invalid'}, 401),
                                  ({'Origin': 'https://foreign.example'}, 403),
                                  ({'X-Cranibox-Target': 'wrong-node'}, 409)]:
            with self.subTest(headers=list(headers)):
                self.assertEqual(self.owner('/sync', index, headers)[0], expected)
        for invalid in [dict(index, base='bad'), dict(index, known=index['known'] * 2),
                        dict(index, known=[{'id': str(uuid.uuid4()), 'digest': 'bad'}])]:
            self.assertEqual(self.owner('/sync', invalid)[0], 400)
        maximum = {'base': index['base'], 'known': [{'id': str(uuid.uuid4()), 'digest': 'a' * 64} for _ in range(512)]}
        status, result = self.owner('/sync', maximum)
        self.assertEqual(status, 200, result)
        self.assertEqual(len(result['delta']['removed']), 512)
        self.assertEqual(len(result['delta']['changed']), 1)
        oversized = copy.deepcopy(maximum)
        oversized['known'].append({'id': str(uuid.uuid4()), 'digest': 'a' * 64})
        self.assertIn(self.owner('/sync', oversized)[0], [400, 413])


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(),
                     'build cranibox; real sockets required')
class EmbeddedHostCatalogSyncTests(unittest.TestCase):
    setUp = host_fixture.WebHostNodeTests.setUp
    api = host_fixture.WebHostNodeTests.api
    restart = host_fixture.WebHostNodeTests.restart
    direct = host_fixture.WebHostNodeTests.direct
    enable_listener = host_fixture.WebHostNodeTests.enable_listener
    direct_projects = host_fixture.WebHostNodeTests.direct_projects

    def test_embedded_workspace_advertises_sync_of_its_own_projects(self):
        own = self.api('/api/projects', operation='register', name='host-owned', node=None, path=str(self.h.root))
        self.enable_listener(projects=True)
        status, listing = self.direct_projects(operation='list')
        self.assertEqual(status, 200, listing)
        self.assertEqual(listing['sync_protocol'], 'project-index-v1')
        index = {'base': 'a' * 64, 'known': []}
        status, response = self.direct('POST', f'/nodes/{self.target}/projects/sync', index)
        self.assertEqual(status, 200, response)
        self.assertEqual(response['delta']['changed'], own['projects'])
        self.assertNotIn('/private/another', json.dumps(response))
        self.assertNotIn('other-device', json.dumps(response))


if __name__ == '__main__':
    unittest.main()
