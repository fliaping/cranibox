"""The Web/Docker host is a peer with its existing catalog and herdr shells."""
import http.client
import json
import os
import subprocess
import time
import unittest

from test_cranibox_terminal import Socket
from test_node_peers import DirectSocket
from workspace_web_harness import CORE, NODE, Harness, free_port, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and CORE.exists() and NODE.exists() and sockets_allowed(),
                     'build cranibox and cranibox-node; real sockets and Unix PTY required')
class WebHostNodeTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN': '/missing-node-herdr'})
        self.addCleanup(self.h.close)
        snapshot = self.h.post('/api/poll', {})[1]['snapshot']
        self.target = snapshot['local_node']['id']
        self.assertRegex(self.target, r'^node-[a-f0-9]{32}$')
        self.assertEqual(self.api('/api/peers', operation='identity')['node_id'], self.target)
        self.settings = self.h.root / '.config/cranibox-node/settings.json'
        self.caller = 'node-external-device'

    def api(self, endpoint, **command):
        status, value = self.h.post(endpoint, command)
        self.assertEqual(status, 200, value)
        return value

    def sessions(self, **command):
        return self.api('/api/terminal/sessions', node=self.target, **command)

    def restart(self):
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        until(lambda: self.h.post('/api/poll', {})[1].get('snapshot'))
        self.assertEqual(self.api('/api/peers', operation='identity')['node_id'], self.target)

    def enable_listener(self, **permissions):
        self.token = self.api('/api/peers', operation='grant', require_verified=False, node=self.caller,
                              permissions=permissions)['token']
        self.port = free_port()
        result = subprocess.run([str(NODE), '--config', str(self.settings), 'peer', 'listen',
                                 '--address', f'127.0.0.1:{self.port}'],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.restart()
        until(lambda: self.direct('GET', '/nodes')[0] == 200)

    def direct(self, method, path, body=None, token=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        try:
            connection.request(method, '/api/v1/mesh' + path,
                               json.dumps(body) if body is not None else None,
                               {'Authorization': 'Bearer ' + (self.token if token is None else token),
                                'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'})
            response = connection.getresponse()
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def direct_projects(self, **command):
        return self.direct('POST', f'/nodes/{self.target}/projects', command)

    def open_web(self, session):
        ticket = self.api('/api/terminal/ticket', node=self.target, local=True,
                          session_id=session, cols=80, rows=24)['ticket']
        ws = Socket(self.h, ticket)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        ws.expect('ready')
        return ws

    def open_direct(self, session):
        ws = DirectSocket(self, session)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        return ws

    def test_web_and_direct_share_shell_catalog_and_identity_across_web_restart(self):
        catalog = self.api('/api/projects', operation='register', name='Host project',
                           node=None, path=str(self.h.root))
        project = catalog['projects'][0]
        replica = project['replicas'][0]
        self.assertEqual(replica['node'], self.target)
        shell = self.sessions(operation='create', name='Host shared shell', cwd=str(self.h.root))['session']
        catalog = self.api('/api/projects', operation='bind', replica_id=replica['id'], terminal_id=shell['id'])
        web = self.open_web(shell['id'])
        state_file = self.h.root / 'shell.pid'
        web.command(f'export CRANIBOX_HOST_SHELL=kept; printf "%s" "$$" > {state_file}; printf "%s%s" WEB_KEPT _OK')
        web.expect('WEB_KEPT_OK')
        pid = until(lambda: state_file.read_text() if state_file.exists() else None)
        web.close()
        self.enable_listener(terminals=True, projects=True)
        peer = self.direct('GET', '/nodes')[1]['nodes'][0]
        self.assertEqual(peer['id'], self.target)
        self.assertTrue(peer['terminal_sessions'])
        self.assertFalse(peer['exec'])
        status, direct = self.direct_projects(operation='list')
        self.assertEqual(status, 200, direct)
        self.assertEqual(direct['projects'], catalog['projects'])
        listing = self.direct('POST', f'/nodes/{self.target}/terminal/sessions', {'operation': 'list'})
        self.assertEqual(listing[0], 200, listing)
        self.assertIn(shell['id'], [s['id'] for s in listing[1]['sessions']])
        ws = self.open_direct(shell['id'])
        ws.command('printf "%s:%s:%s" DIRECT_ "$CRANIBOX_HOST_SHELL" "$$"')
        ws.expect('DIRECT_:kept:' + pid)
        self.api('/api/peers', operation='revoke', node=self.caller)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if ws.recv()[0] == 8:
                break
        else:
            self.fail('revoked peer kept its terminal attachment')
        ws.close()
        self.assertEqual(self.direct_projects(operation='list')[0], 401)
        self.assertEqual(self.sessions(operation='list')['sessions'][0]['status'], 'running')
        self.restart()
        self.assertEqual(self.api('/api/projects', operation='list')['projects'], catalog['projects'])
        web = self.open_web(shell['id'])
        web.command('printf "%s:%s:%s" WEB_AGAIN_ "$CRANIBOX_HOST_SHELL" "$$"')
        web.expect('WEB_AGAIN_:kept:' + pid)
        web.close()
        self.sessions(operation='end', id=shell['id'])

    def test_legacy_catalog_is_claimed_once_and_peer_permissions_do_not_leak_remote_paths(self):
        host = self.api('/api/projects', operation='register', name='Legacy host', node=None, path=str(self.h.root))
        host_project = host['projects'][0]
        shell = self.sessions(operation='create', name='Legacy shell', cwd=str(self.h.root))['session']
        host = self.api('/api/projects', operation='bind', replica_id=host_project['replicas'][0]['id'], terminal_id=shell['id'])
        both = self.api('/api/projects', operation='register', name='Remote private', node='test-node', path='/private/remote')
        self.h.stop(self.h.web)
        path = self.h.root / '.config/agent-workspace/projects.json'
        legacy = json.loads(path.read_text())
        legacy['projects'][0]['replicas'][0]['node'] = None
        path.write_text(json.dumps(legacy))
        self.h.start_web()
        self.h.login()
        upgraded = self.api('/api/projects', operation='list')
        self.assertEqual(upgraded['projects'], both['projects'])
        self.assertEqual(upgraded['revision'], legacy['revision'] + 1)
        self.enable_listener(projects=True)
        status, view = self.direct_projects(operation='list')
        self.assertEqual(status, 200, view)
        self.assertEqual(view['projects'], host['projects'])
        self.assertNotIn('/private/remote', json.dumps(view))
        self.assertEqual(self.api('/api/projects', operation='list')['revision'], upgraded['revision'])
        self.assertEqual(self.direct('POST', f'/nodes/{self.target}/terminal/sessions', {'operation': 'list'})[0], 403)
        self.assertEqual(self.direct_projects(operation='bind', replica_id=host_project['replicas'][0]['id'], terminal_id=shell['id'])[0], 403)
        remote_replica = both['projects'][1]['replicas'][0]['id']
        self.assertEqual(self.direct_projects(operation='forget', replica_id=remote_replica)[0], 400)
        self.assertEqual(self.direct_projects(operation='register', name='Wrong owner', node='test-node', path='/wrong/owner')[0], 400)
        self.sessions(operation='end', id=shell['id'])

    def test_explicit_local_target_cannot_fall_back_to_another_device(self):
        before = self.sessions(operation='list')['sessions']
        for command in ({'node': 'wrong-node', 'local': True},
                        {'local': True}, {'node': self.target, 'direct': True, 'local': True}):
            status, value = self.h.post('/api/terminal/sessions', {**command, 'operation': 'create', 'name': 'must not start'})
            self.assertEqual(status, 400, value)
        self.assertEqual(self.sessions(operation='list')['sessions'], before)
        # Ordinary mesh credentials never inherit controller access to the host.
        token = (self.h.root / 'node.token').read_text().strip()
        status, visible = self.h.http('POST', '/api/v1/mesh/projects', {'operation': 'list'}, self.h.gateway_port,
                                      {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
        self.assertEqual(status, 200, visible)
        self.assertEqual(visible['projects'], [])
        status, value = self.h.http('POST', '/api/v1/mesh/projects',
                                    {'operation': 'register', 'name': 'unauthorized', 'node': self.target, 'path': '/must-not-register'},
                                    self.h.gateway_port, {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
        self.assertEqual(status, 400, value)


if __name__ == '__main__':
    unittest.main()
