"""Real direct-node durable runs: authorization, recovery and revocation."""
import http.client
import json
import os
import subprocess
import sys
import unittest
import uuid

import test_web_direct_peers as direct
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class RemoteAgentExecutionsTests(unittest.TestCase):
    setUp = direct.WebDirectPeersTests.setUp
    api = direct.WebDirectPeersTests.api
    cli = direct.WebDirectPeersTests.cli

    def action(self, operation, **values):
        return self.api(operation='executions', node=self.target, command={'operation': operation, **values})

    def request(self, command, token=None, **headers):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        try:
            connection.request('POST', f'/api/v1/mesh/nodes/{self.target}/executions', json.dumps(command), {
                'Authorization': 'Bearer ' + (token or self.token), 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json', **headers})
            response = connection.getresponse()
            data = response.read()
            return response.status, json.loads(data) if data else None
        finally:
            connection.close()

    def launch(self, code):
        return {'id': str(uuid.uuid4()), 'launch': {'name': 'Remote review', 'harness': 'fixture-cli', 'program': sys.executable, 'args': ['-u', '-c', code], 'cwd': str(self.h.root)}}

    def detail(self, run):
        return self.action('get', id=run['id'])

    def state(self, run, states):
        return until(lambda: detail if (detail := self.detail(run))['execution']['state'] in states else None)

    def test_remote_process_survives_observer_and_web_restart_without_duplicate_launch(self):
        self.h.stop(self.h.server)
        self.assertTrue(self.api(operation='probe', node=self.target)['nodes'][0]['executions'])
        prepared = self.action('prepare_agent', agent={'name': 'Review', 'adapter': 'codex', 'access': 'read_only', 'prompt': 'No implicit launch', 'cwd': str(self.h.root)})
        self.assertEqual(prepared['launch']['stdin'], 'No implicit launch')
        self.assertEqual(self.action('list')['executions'], [])
        run = self.launch("import os,pathlib,time; p=pathlib.Path('remote-launch-count'); p.write_text(p.read_text()+'x' if p.exists() else 'x'); print(os.environ['CRANIBOX_NODE_ID'],flush=True); time.sleep(1.5); print('finished 遠端')")
        self.action('start', **run)
        self.action('start', **run)
        running = self.state(run, {'running', 'succeeded'})
        self.assertEqual(running['execution']['requester'], self.caller)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        detail = self.state(run, {'succeeded'})
        self.assertIn(self.target, detail['stdout']['text'])
        self.assertIn('finished 遠端', detail['stdout']['text'])
        self.assertEqual((self.h.root / 'remote-launch-count').read_text(), 'x')
        self.assertEqual(len(self.action('list')['executions']), 1)
        self.action('archive', id=run['id'])
        self.action('start', **run)
        self.assertEqual(self.action('list')['executions'], [])
        self.assertEqual((self.h.root / 'remote-launch-count').read_text(), 'x')

    def test_cli_selects_the_remote_node_and_accepts_escaped_standard_input(self):
        run = self.launch("import sys; value=sys.stdin.read(); print('input-bytes',len(value.encode())); assert value=='\\x01'*16384")
        run['launch']['stdin'] = '\x01' * 16384
        result = subprocess.run([str(NODE), '--config', str(self.path.with_name('settings.json')), 'peer', 'runs', '--node', self.target, '--command', json.dumps({'operation': 'start', **run})], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['owner'], self.target)
        detail = self.state(run, {'succeeded', 'failed'})
        self.assertEqual(detail['execution']['state'], 'succeeded', detail)
        self.assertIn('input-bytes 16384', detail['stdout']['text'])

    def test_other_callers_local_runs_and_rotated_grants_remain_private(self):
        run = self.launch("print('private remote output')")
        self.action('start', **run)
        self.state(run, {'succeeded'})
        local = self.launch("print('local-only prompt and output')")
        self.cli('runs', '--command', json.dumps({'operation': 'start', **local}))
        other_token = self.h.root / 'other.token'
        self.cli('grant', '--allow-unverified', '--node', 'other-caller', '--output-token', other_token, '--exec')
        other = other_token.read_text().strip()
        status, catalog = self.request({'operation': 'list'}, other)
        self.assertEqual(status, 200)
        self.assertEqual(catalog['executions'], [])
        self.assertEqual(catalog['used'], 0)
        for identity in [run['id'], local['id']]:
            for command in [{'operation': 'get', 'id': identity}, {'operation': 'stop', 'id': identity}, {'operation': 'archive', 'id': identity}, {'operation': 'start', 'id': identity, 'launch': run['launch']}]:
                status, value = self.request(command, other)
                self.assertEqual(status, 400, value)
                self.assertEqual(value, {'error': 'run unavailable'})
        self.assertEqual(len(self.action('list')['executions']), 1)
        self.assertEqual(len(self.cli('runs')['executions']), 2)
        self.cli('revoke', '--node', self.caller)
        rotated_path = self.h.root / 'rotated.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', rotated_path, '--exec')
        new_token = rotated_path.read_text().strip()
        self.assertEqual(self.request({'operation': 'list'})[0], 401)
        self.assertEqual(self.request({'operation': 'list'}, new_token)[1]['executions'], [])
        self.assertEqual(self.request({'operation': 'get', 'id': run['id']}, new_token)[0], 400)

    def test_revocation_stops_remote_process_tree_and_rejects_new_requests(self):
        run = self.launch("import subprocess,time,pathlib; child=subprocess.Popen(['sleep','90']); pathlib.Path('remote-child-pid').write_text(str(child.pid)); print('started',flush=True); time.sleep(90)")
        self.action('start', **run)
        self.state(run, {'running'})
        child = until(lambda: int(path.read_text()) if (path := self.h.root / 'remote-child-pid').exists() else None)
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.request({'operation': 'list'})[0], 401)
        detail = until(lambda: detail if (detail := self.cli('runs', '--command', json.dumps({'operation': 'get', 'id': run['id']})))['execution']['state'] == 'cancelled' else None)
        self.assertIn('revoked', detail['execution']['error'])
        def stopped():
            stat = self.h.root.__class__(f'/proc/{child}/stat')
            return not stat.exists() or stat.read_text().split()[2] == 'Z'
        until(stopped)

    def test_non_execution_grants_browser_origins_and_oversize_requests_are_rejected(self):
        other_token = self.h.root / 'read-only.token'
        self.cli('grant', '--allow-unverified', '--node', 'messages-only', '--output-token', other_token, '--messages', '--inbound', 'accept')
        token = other_token.read_text().strip()
        run = self.launch('pass')
        for command in [{'operation': 'list'}, {'operation': 'start', **run}, {'operation': 'prepare_agent', 'agent': {'name': 'Review', 'adapter': 'codex', 'access': 'read_only', 'prompt': 'Review', 'cwd': str(self.h.root)}}]:
            self.assertEqual(self.request(command, token)[0], 403)
        self.assertEqual(self.request({'operation': 'list'}, Origin='https://example.invalid')[0], 403)
        self.assertEqual(self.request({'operation': 'list'}, **{'X-Cranibox-Target': 'wrong-device'})[0], 409)
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        try:
            body = json.dumps({'operation': 'start', **run, 'padding': 'x' * (128 * 1024)})
            connection.request('POST', f'/api/v1/mesh/nodes/{self.target}/executions', body, {'Authorization': 'Bearer ' + self.token, 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'})
            response = connection.getresponse()
            self.assertEqual(response.status, 413)
            response.read()
        finally:
            connection.close()
        self.assertEqual(self.action('list')['executions'], [])
