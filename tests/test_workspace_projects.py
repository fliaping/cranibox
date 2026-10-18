"""Project identity, authorization and real node-owned shells across entrypoints."""
import json
import os
import stat
import unittest
import uuid

from workspace_web_harness import CORE, NODE, Harness, sockets_allowed, until
from test_cranibox_terminal import Socket


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(),
                     'build cranibox and cranibox-node; integration tests require Unix sockets')
class ProjectTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN': '/missing-node-herdr'})
        self.addCleanup(self.h.close)

    def projects(self, **command):
        status, value = self.h.post('/api/projects', command)
        self.assertEqual(status, 200, value)
        return value

    def peer(self, path, command, token=None, **headers):
        return self.h.http('POST', '/api/v1/mesh/' + path, command, self.h.gateway_port,
                           {'Authorization': 'Bearer ' + (token or (self.h.root / 'node.token').read_text().strip()),
                            'Content-Type': 'application/json', **headers})

    def register(self, node='test-node', path=None, project_id=None):
        return self.projects(operation='register', name='Personal project', node=node,
                             path=path or str(self.h.root), project_id=project_id)

    def attach(self, terminal_id):
        status, ticket = self.h.post('/api/terminal/ticket',
                                     {'node': 'test-node', 'session_id': terminal_id, 'cols': 80, 'rows': 24})
        self.assertEqual(status, 200, ticket)
        ws = Socket(self.h, ticket['ticket'])
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        return ws

    def test_web_and_peer_share_identity_and_shell_survives_gateway_restart(self):
        catalog = self.register()
        replica = catalog['projects'][0]['replicas'][0]
        self.assertEqual(self.peer('projects', {'operation': 'list'}), (200, catalog))
        self.assertEqual(self.register(), catalog)
        status, created = self.peer('nodes/test-node/terminal/sessions',
                                    {'operation': 'create', 'name': 'Project shell', 'cwd': str(self.h.root)})
        self.assertEqual(status, 200, created)
        terminal_id = created['session']['id']
        binding = {'operation': 'bind', 'replica_id': replica['id'], 'terminal_id': terminal_id}
        status, linked = self.peer('projects', binding)
        self.assertEqual(status, 200, linked)
        self.assertEqual(self.projects(**binding), linked)
        ws = self.attach(terminal_id)
        ws.command("export CRANIBOX_PROJECT=retained; printf '%s%s' PROJECT _READY")
        ws.expect('PROJECT_READY')
        ws.close()
        self.h.stop(self.h.server)
        self.h.start_web()
        self.h.login()
        until(lambda: self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        self.assertEqual(self.projects(operation='list'), linked)
        ws = self.attach(terminal_id)
        ws.command('printf "%s%s" STILL_ "$CRANIBOX_PROJECT"')
        ws.expect('STILL_retained')
        status, listing = self.peer('nodes/test-node/terminal/sessions', {'operation': 'list'})
        self.assertEqual(status, 200, listing)
        self.assertEqual([s['id'] for s in listing['sessions']], [terminal_id])
        self.assertEqual(listing['sessions'][0]['status'], 'running')
        self.projects(operation='unbind', replica_id=replica['id'], terminal_id=terminal_id)
        self.assertEqual(self.peer('nodes/test-node/terminal/sessions', {'operation': 'list'})[1], listing)
        self.assertEqual(self.projects(operation='forget', replica_id=replica['id'])['projects'], [])
        self.assertEqual(self.peer('nodes/test-node/terminal/sessions', {'operation': 'list'})[1], listing)
        self.assertTrue(self.h.root.is_dir())
        mode = stat.S_IMODE((self.h.credentials.parent / 'projects.json').stat().st_mode)
        self.assertEqual(mode, 0o600)

    def test_device_groups_filter_nested_replicas_and_enforce_mutations(self):
        self.h.cli('enroll', 'private-node', '--credentials', self.h.credentials,
                   '--token-file', self.h.root / 'private.token')
        catalog = self.register()
        project_id = catalog['projects'][0]['id']
        catalog = self.register('private-node', '/private/secret', project_id)
        private_replica = catalog['projects'][0]['replicas'][1]['id']
        self.register(None, '/workspace/private')
        status, visible = self.peer('projects', {'operation': 'list'})
        self.assertEqual(status, 200, visible)
        self.assertEqual(len(visible['projects']), 1)
        self.assertEqual(len(visible['projects'][0]['replicas']), 1)
        self.assertNotIn('/private/secret', json.dumps(visible))
        for operation in ('bind', 'unbind'):
            self.assertEqual(self.peer('projects', {'operation': operation, 'replica_id': private_replica,
                                                   'terminal_id': str(uuid.uuid4())})[0], 400)
        self.assertEqual(self.peer('projects', {'operation': 'forget', 'replica_id': private_replica})[0], 400)
        self.assertEqual(self.peer('projects', {'operation': 'register', 'name': 'No', 'node': None,
                                               'path': '/forbidden'})[0], 400)
        self.assertEqual(self.peer('nodes/private-node/terminal/sessions', {'operation': 'list'})[0], 403)
        for node in ('test-node', 'private-node'):
            self.h.cli('group', node, '--group', 'personal', '--credentials', self.h.credentials)
        self.assertEqual(len(self.peer('projects', {'operation': 'list'})[1]['projects'][0]['replicas']), 2)
        self.h.cli('group', 'private-node', '--credentials', self.h.credentials)
        self.assertEqual(len(self.peer('projects', {'operation': 'list'})[1]['projects'][0]['replicas']), 1)

    def test_offline_binding_is_kept_and_missing_shell_is_not_recreated(self):
        catalog = self.register()
        replica_id = catalog['projects'][0]['replicas'][0]['id']
        status, created = self.peer('nodes/test-node/terminal/sessions',
                                    {'operation': 'create', 'name': 'Original', 'cwd': str(self.h.root)})
        self.assertEqual(status, 200, created)
        terminal_id = created['session']['id']
        linked = self.projects(operation='bind', replica_id=replica_id, terminal_id=terminal_id)
        self.h.stop(self.h.node)
        until(lambda: not self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        self.assertEqual(self.projects(operation='list'), linked)
        self.h.node = self.h.start(NODE, '--id', 'test-node', '--server',
                                  f'ws://127.0.0.1:{self.h.gateway_port}', '--token-file', self.h.root / 'node.token')
        until(lambda: self.h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        self.assertEqual(self.peer('nodes/test-node/terminal/sessions', {'operation': 'list'})[1]['sessions'], [])
        self.assertEqual(self.projects(operation='list'), linked)

    def test_authentication_validation_and_nonexistent_shell(self):
        catalog = self.register()
        replica_id = catalog['projects'][0]['replicas'][0]['id']
        self.assertEqual(self.peer('projects', {'operation': 'list'}, token='invalid')[0], 401)
        self.assertEqual(self.peer('projects', {'operation': 'list'}, Origin='https://example.com')[0], 403)
        self.assertEqual(self.h.http('POST', '/api/projects', {'operation': 'list'},
                                     headers={'Cookie': self.h.cookie, 'Content-Type': 'application/json'})[0], 401)
        for command in ({'operation': 'register', 'name': 'Invalid', 'node': 'test-node', 'path': 'relative'},
                        {'operation': 'register', 'name': 'Invalid', 'node': 'unknown', 'path': '/absolute'},
                        {'operation': 'list', 'unexpected': True},
                        {'operation': 'bind', 'replica_id': replica_id, 'terminal_id': str(uuid.uuid4())}):
            self.assertGreaterEqual(self.h.post('/api/projects', command)[0], 400)
        self.assertEqual(self.projects(operation='list'), catalog)


if __name__ == '__main__':
    unittest.main()
