"""Explicit gateway identity migration against real Rust services and node PTYs."""
import json
import os
import subprocess
import sys
import unittest

from test_gateway_terminal import GatewaySocket
from workspace_web_harness import CORE, NODE, Harness, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(),
                     'build cranibox and cranibox-node; real sockets and Unix PTY required')
class NodeIdentityTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN': '/missing-node-herdr'})
        self.addCleanup(self.h.close)
        self.token = (self.h.root / 'node.token').read_text().strip()
        self.node_id = json.loads((self.h.root / 'test-node/peers.json').read_text())['node_id']

    def peer(self, path, body=None, token=None):
        return self.h.http('POST' if body is not None else 'GET', '/api/v1/mesh/' + path,
                           body, self.h.gateway_port, {'Authorization': 'Bearer ' + (token or self.token),
                                                      'Content-Type': 'application/json'})

    def migrate(self, target=None, source='test-node', status=200):
        result, value = self.h.post('/api/nodes/identity', {'id': source, 'target': target or self.node_id})
        self.assertEqual(result, status, value)
        return value

    def connected(self):
        return until(lambda: next((node for node in self.h.gateway('/api/v1/nodes')['nodes']
                                   if node['id'] == self.node_id and node['online']), None))

    def attach(self, session):
        # An already installed client still uses its old registration in this URL.
        ws = GatewaySocket(self.h, session, self.token)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        return ws

    def catalog(self, **command):
        status, value = self.h.post('/api/projects', command)
        self.assertEqual(status, 200, value)
        return value

    def test_token_group_projects_and_running_shell_survive_migration_and_restart(self):
        self.h.cli('group', 'test-node', '--group', 'personal', '--credentials', self.h.credentials)
        catalog = self.catalog(operation='register', node='test-node', name='Device project', path=str(self.h.root))
        status, value = self.peer('nodes/test-node/terminal/sessions',
                                  {'operation': 'create', 'name': 'Original shell', 'cwd': str(self.h.root)})
        self.assertEqual(status, 200, value)
        session = value['session']['id']
        before = self.catalog(operation='bind', replica_id=catalog['projects'][0]['replicas'][0]['id'],
                              terminal_id=session)
        ws = self.attach(session)
        pid_file = self.h.root / 'shell.pid'
        ws.command(f'export IDENTITY_SHELL=retained; echo $$ > {pid_file}; printf "%s%s" BEFORE_ "$IDENTITY_SHELL"')
        ws.expect('BEFORE_retained')
        pid = int(pid_file.read_text())
        migrated = self.migrate()
        self.assertEqual(migrated, {'id': self.node_id, 'previous_id': 'test-node', 'aliases': ['test-node']})
        connected = self.connected()
        self.assertEqual(connected['aliases'], ['test-node'])
        self.assertEqual(connected['mesh_group'], 'personal')
        credentials = json.loads(self.h.credentials.read_text())
        self.assertEqual(credentials['nodes'], {self.node_id: self.token})
        self.assertEqual(credentials['mesh_groups'], {self.node_id: 'personal'})
        after = self.catalog(operation='list')
        before['revision'] += 1
        before['projects'][0]['replicas'][0]['node'] = self.node_id
        self.assertEqual(after, before)
        self.assertEqual(self.peer('projects', {'operation': 'list'}), (200, after))
        self.assertEqual(self.catalog(operation='register', node='test-node', name='Device project',
                                      path=str(self.h.root)), after)
        ws.close()
        ws = self.attach(session)
        new_pid = self.h.root / 'same-shell.pid'
        ws.command(f'echo $$ > {new_pid}; printf "%s%s" AFTER_ "$IDENTITY_SHELL"')
        ws.expect('AFTER_retained')
        self.assertEqual(int(new_pid.read_text()), pid)
        # Retrying a successful request must not disconnect a newly attached shell.
        self.migrate()
        ws.command('printf "%s%s" RETRY_ "$IDENTITY_SHELL"')
        ws.expect('RETRY_retained')
        self.assertEqual(self.catalog(operation='list'), after)
        ws.close()
        self.h.stop(self.h.server)
        os.kill(pid, 0)
        self.h.start_web()
        self.h.login()
        self.connected()
        self.assertEqual(self.catalog(operation='list'), after)
        ws = self.attach(session)
        ws.command('printf "%s%s" RESTART_ "$IDENTITY_SHELL"')
        ws.expect('RESTART_retained')
        self.assertIsNone(self.h.node.poll())

    def test_running_jobs_block_migration_and_finished_job_permissions_are_retained(self):
        status, job = self.peer('nodes/test-node/exec', {'program': sys.executable,
                              'args': ['-c', 'import time; time.sleep(60)'], 'timeout_secs': 90})
        self.assertEqual(status, 202, job)
        before = self.h.credentials.read_bytes()
        token = json.loads(before)['controller_token']
        status, error = self.h.http('POST', '/api/v1/nodes/test-node/identity', {'target': self.node_id},
                                    self.h.gateway_port, {'Authorization': 'Bearer ' + token,
                                                          'Content-Type': 'application/json'})
        self.assertEqual(status, 409, error)
        self.assertIn('tasks to finish', error['error'])
        self.assertEqual(self.h.credentials.read_bytes(), before)
        self.assertEqual(self.peer('jobs/' + job['id'] + '/cancel', {})[0], 200)
        until(lambda: self.peer('jobs/' + job['id'])[1]['outcome'] != 'running')
        self.migrate()
        self.connected()
        status, completed = self.peer('jobs/' + job['id'])
        self.assertEqual(status, 200, completed)
        self.assertEqual(completed['outcome'], 'cancelled')
        status, new_job = self.peer('nodes/test-node/exec', {'program': sys.executable,
                                  'args': ['-c', 'print("new identity")']})
        self.assertEqual(status, 202, new_job)
        self.assertEqual(new_job['node'], self.node_id)
        self.assertEqual(new_job['requester'], self.node_id)
        until(lambda: self.peer('jobs/' + new_job['id'])[1]['outcome'] == 'exited')

        # A response lost in transit can be retried after work has resumed.
        # Retrying the same mapping must neither reject nor interrupt that work.
        status, active = self.peer('nodes/test-node/exec', {'program': sys.executable,
                                  'args': ['-c', 'import time; time.sleep(60)'], 'timeout_secs': 90})
        self.assertEqual(status, 202, active)
        self.migrate()
        self.assertEqual(self.peer('jobs/' + active['id'])[1]['outcome'], 'running')
        self.assertEqual(self.peer('jobs/' + active['id'] + '/cancel', {})[0], 200)
        until(lambda: self.peer('jobs/' + active['id'])[1]['outcome'] != 'running')

    def test_registration_host_and_project_conflicts_never_overwrite_other_devices(self):
        self.h.cli('enroll', 'occupied', '--credentials', self.h.credentials,
                   '--token-file', self.h.root / 'occupied.token')
        self.catalog(operation='register', name='Other device', node='occupied', path='/private/project')
        before = self.h.credentials.read_bytes()
        projects = (self.h.credentials.parent / 'projects.json').read_bytes()
        self.migrate('occupied', status=400)
        self.assertEqual(self.h.credentials.read_bytes(), before)
        self.assertEqual((self.h.credentials.parent / 'projects.json').read_bytes(), projects)
        self.h.cli('revoke', 'occupied', '--credentials', self.h.credentials)
        before = self.h.credentials.read_bytes()
        self.migrate('occupied', status=400)
        self.assertEqual(self.h.credentials.read_bytes(), before)
        host = self.h.post('/api/poll', {})[1]['snapshot']['local_node']['id']
        self.migrate(host, status=400)
        self.migrate(source='missing', status=400)
        self.assertEqual(self.h.credentials.read_bytes(), before)
        self.assertEqual((self.h.credentials.parent / 'projects.json').read_bytes(), projects)

    def test_revocation_by_alias_reserves_old_names_and_never_restores_the_old_token(self):
        self.migrate()
        self.connected()
        self.assertEqual(self.h.post('/api/nodes/revoke', {'id': 'test-node'})[0], 200)
        self.assertEqual(self.peer('nodes')[0], 401)
        self.assertEqual(self.h.post('/api/nodes/enroll', {'id': 'test-node'})[0], 400)
        with self.assertRaises(subprocess.CalledProcessError):
            self.h.cli('enroll', 'test-node', '--credentials', self.h.credentials,
                       '--token-file', self.h.root / 'reserved.token')
        self.assertFalse((self.h.root / 'reserved.token').exists())
        status, enrolled = self.h.post('/api/nodes/enroll', {'id': self.node_id})
        self.assertEqual(status, 200, enrolled)
        self.assertNotEqual(enrolled['token'], self.token)
        self.assertEqual(self.peer('nodes')[0], 401)
        self.assertEqual(self.peer('nodes', token=enrolled['token'])[0], 200)
        self.assertEqual(json.loads(self.h.credentials.read_text())['node_aliases'], {'test-node': self.node_id})

    def test_only_controller_can_migrate_and_cli_can_reverse_an_approved_mapping(self):
        before = self.h.credentials.read_bytes()
        self.assertEqual(self.peer('nodes/test-node/identity', {'target': self.node_id})[0], 401)
        self.assertEqual(self.h.credentials.read_bytes(), before)
        self.assertEqual(self.h.http('POST', '/api/nodes/identity',
                                    {'id': 'test-node', 'target': self.node_id},
                                    headers={'Cookie': self.h.cookie, 'Content-Type': 'application/json'})[0], 401)
        self.h.cli('migrate-identity', 'test-node', self.node_id, '--credentials', self.h.credentials,
                   '--url', f'http://127.0.0.1:{self.h.gateway_port}')
        self.connected()
        self.h.cli('group', 'test-node', '--group', 'trusted', '--credentials', self.h.credentials)
        self.assertEqual(json.loads(self.h.credentials.read_text())['mesh_groups'], {self.node_id: 'trusted'})
        self.h.cli('migrate-identity', self.node_id, 'test-node', '--credentials', self.h.credentials,
                   '--url', f'http://127.0.0.1:{self.h.gateway_port}')
        until(lambda: any(node['id'] == 'test-node' and node['online']
                          for node in self.h.gateway('/api/v1/nodes')['nodes']))
        self.assertEqual(json.loads(self.h.credentials.read_text())['nodes'], {'test-node': self.token})
        self.assertEqual(json.loads(self.h.credentials.read_text())['node_aliases'], {self.node_id: 'test-node'})


if __name__ == '__main__':
    unittest.main()
