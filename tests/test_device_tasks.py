"""Persistent explicit Agent claims and reports across two real independent nodes."""
import json
import os
import subprocess
import time
import unittest
import uuid

from workspace_web_harness import Harness, CORE, NODE, RUNTIME, WEB, free_port, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class DeviceTasksTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.caller = self.api(operation='identity')['node_id']
        self.config = self.h.root / 'target/settings.json'
        self.target = self.cli('identity')['node_id']
        self.token_file = self.h.root / 'tasks.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', self.token_file, '--messages', '--inbound', 'accept')
        self.token = self.token_file.read_text().strip()
        self.port = free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.cli('listen', '--address', f'127.0.0.1:{self.port}')
        self.start_target()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        self.path = self.h.root / '.config/cranibox-node/device-tasks.json'

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def api(self, **command):
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, value)
        if hasattr(self, 'token'):
            self.assertNotIn(self.token, json.dumps(value))
        return value

    def tasks(self, **command):
        return self.api(operation='tasks', command=command)

    def worker(self, **command):
        return self.cli('tasks', '--command', json.dumps(command))

    def start_target(self):
        self.process = self.h.start(NODE, '--config', self.config, 'peer', 'serve')
        until(lambda: self.direct('GET', '/api/v1/mesh/nodes')[0] == 200)

    def direct(self, method, path, body=None, headers=None):
        return self.h.http(method, path, body, port=self.port, headers={
            'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json',
            'X-Cranibox-Target': self.target, **(headers or {})})

    def create(self, description='Read the files and report evidence. 不要自动运行。'):
        command = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target,
                       title='Review the project', description=description)
        view = self.tasks(**command)
        return command, next(item for item in view['outgoing'] if item['task']['id'] == command['id'])

    def outgoing(self, task_id):
        return next(item for item in self.tasks(operation='list')['outgoing'] if item['task']['id'] == task_id)

    def synced(self, task, state):
        self.tasks(operation='sync', id=task['id'], dispatch=task['dispatch'])
        return until(lambda: item if (item := self.outgoing(task['id']))['current']['state'] == state else None)

    def call_worker(self, task, operation, **extra):
        return self.worker(operation=operation, id=task['id'], dispatch=task['dispatch'], **extra)

    def rejected(self, command):
        self.assertEqual(self.h.post('/api/peers', {'operation':'tasks', 'command':command})[0], 400)

    def test_offline_delegation_survives_restart_and_reports_need_no_reverse_connection(self):
        self.h.stop(self.h.server)
        self.h.stop(self.process)
        task, queued = self.create()
        self.assertEqual(queued['current']['state'], 'queued')
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        until(lambda: self.outgoing(task['id'])['attempts'] > 0)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.outgoing(task['id'])['task'], queued['task'])
        self.start_target()
        available = self.synced(task, 'available')
        self.assertIsNone(available['current']['session'])
        self.assertEqual(self.cli('identity')['routes'], [])
        session = str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'progress', session=session, text='Checked the first file.')
        self.assertEqual(self.synced(task, 'running')['current']['progress'], 'Checked the first file.')
        self.call_worker(task, 'finish', session=session, outcome='succeeded', result='Reviewed two files; no changes required.')
        self.assertEqual(self.synced(task, 'succeeded')['current']['session'], session)
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['state'], 'succeeded')
        self.assertNotIn(self.token, self.path.read_text())
        self.assertFalse(self.config.exists(), 'independent tasks must not need relay settings')

    def test_repeated_delivery_is_inert_and_archive_retains_deduplication(self):
        marker = self.h.root / 'must-not-execute'
        task, first = self.create('touch ' + str(marker))
        self.tasks(**task)
        self.rejected({**task, 'description':'Different content'})
        self.synced(task, 'available')
        path = f'/api/v1/mesh/nodes/{self.target}/tasks'
        offer = {'task':first['task'], 'dispatch':task['dispatch'], 'cancel':False}
        self.assertEqual(self.direct('POST', path, offer)[0], 200)
        self.assertEqual(len(self.worker(operation='list')['incoming']), 1)
        self.call_worker(task, 'decline', reason='No suitable Agent is available.')
        self.synced(task, 'failed')
        self.call_worker(task, 'archive')
        self.assertEqual(self.worker(operation='list')['incoming'], [])
        self.assertEqual(self.direct('POST', path, offer)[1]['state'], 'failed')
        self.assertEqual(self.worker(operation='list')['incoming'], [])
        self.assertFalse(marker.exists())
        self.tasks(operation='archive', id=task['id'], dispatch=task['dispatch'])
        self.assertEqual(self.tasks(operation='list')['outgoing'], [])

    def test_retries_require_confirmed_failure_and_late_old_reports_cannot_finish_new_attempt(self):
        task, _ = self.create()
        retry = dict(operation='retry', id=task['id'], dispatch=task['dispatch'], new_dispatch=str(uuid.uuid4()))
        self.rejected(retry)
        self.synced(task, 'available')
        session = str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'finish', session=session, outcome='failed', result='Dependency unavailable.')
        self.synced(task, 'failed')
        self.tasks(**retry)
        self.tasks(**retry)
        next_task = {**task, 'dispatch':retry['new_dispatch']}
        item = self.synced(next_task, 'available')
        self.assertEqual(len(item['history']), 2)
        old_report = dict(operation='finish', id=task['id'], dispatch=task['dispatch'], session=session,
                          outcome='succeeded', result='An old report must not complete the new attempt.')
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'tasks', '--command', json.dumps(old_report)], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['state'], 'available')
        self.assertNotIn('An old report', self.worker(operation='list')['incoming'][0]['current']['result'] or '')

    def test_cancel_before_claim_is_atomic_but_running_agent_must_confirm(self):
        self.h.stop(self.process)
        task, _ = self.create()
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        self.assertEqual(self.outgoing(task['id'])['current']['state'], 'queued')
        self.start_target()
        self.assertIsNone(self.synced(task, 'cancelled')['current']['session'])
        task, _ = self.create()
        self.synced(task, 'available')
        session = str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        self.assertEqual(self.synced(task, 'cancel_requested')['current']['session'], session)
        self.rejected(dict(operation='retry', id=task['id'], dispatch=task['dispatch'], new_dispatch=str(uuid.uuid4())))
        self.call_worker(task, 'finish', session=session, outcome='cancelled', result='Stopped after saving the current file.')
        self.assertEqual(self.synced(task, 'cancelled')['current']['result'], 'Stopped after saving the current file.')

    def test_worker_questions_and_offline_replies_persist_without_reverse_routes(self):
        self.h.stop(self.h.server)
        task, _ = self.create()
        self.synced(task, 'available')
        session, question = str(uuid.uuid4()), str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'ask', session=session, question=question, text='Which revision should I review?')
        self.call_worker(task, 'ask', session=session, question=question, text='Which revision should I review?')
        self.synced(task, 'running')
        view = until(lambda: item if (item := self.outgoing(task['id']))['current'].get('questions') else None)
        self.assertEqual(view['current']['questions'], [dict(id=question, text='Which revision should I review?', answer=None)])
        self.h.stop(self.process)
        reply = dict(operation='answer', id=task['id'], dispatch=task['dispatch'], question=question, text='Use the checked release revision. 不运行部署。')
        self.tasks(**reply)
        self.tasks(**reply)
        self.rejected({**reply, 'text':'Replace the queued reply'})
        self.rejected({**reply, 'dispatch':str(uuid.uuid4())})
        self.assertIsNone(self.outgoing(task['id'])['current']['questions'][0]['answer'])
        self.assertEqual(self.outgoing(task['id'])['pending_answer']['text'], reply['text'])
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.outgoing(task['id'])['pending_answer']['question'], question)
        self.start_target()
        self.tasks(operation='sync', id=task['id'], dispatch=task['dispatch'])
        view = until(lambda: item if (item := self.outgoing(task['id']))['current']['questions'][0]['answer'] == reply['text'] else None)
        self.assertIsNone(view['pending_answer'])
        self.assertEqual(self.cli('identity')['routes'], [])
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['questions'][0]['answer'], reply['text'])
        self.tasks(**reply)
        self.rejected({**reply, 'text':'Change confirmed reply'})
        self.call_worker(task, 'finish', session=session, outcome='succeeded', result='Reviewed the requested revision.')
        self.synced(task, 'succeeded')
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['questions'][0]['answer'], reply['text'])

    def test_auth_identity_grants_receipt_privacy_and_limits_are_enforced(self):
        task, item = self.create()
        self.synced(task, 'available')
        base = f'/api/v1/mesh/nodes/{self.target}'
        offer = dict(task=item['task'], dispatch=task['dispatch'], cancel=False)
        self.assertEqual(self.direct('POST', base+'/tasks', offer, {'Authorization':''})[0], 401)
        self.assertEqual(self.direct('POST', base+'/tasks', offer, {'Origin':'https://untrusted.example'})[0], 403)
        self.assertEqual(self.direct('POST', base+'/tasks', offer, {'X-Cranibox-Target':'wrong-node'})[0], 409)
        self.assertEqual(self.direct('POST', base+'/tasks', {**offer, 'task':{**item['task'], 'sender':'spoofed'}})[0], 400)
        self.assertEqual(self.direct('POST', base+'/tasks', {**offer, 'task':{**item['task'], 'description':'🙂'*2049}})[0], 400)
        self.assertEqual(self.direct('GET', base+f'/tasks/{task["id"]}/{task["dispatch"]}')[1]['state'], 'available')
        other = self.h.root / 'other.token'
        self.cli('grant', '--allow-unverified', '--node', 'other-device', '--output-token', other, '--messages', '--inbound', 'accept')
        self.assertEqual(self.direct('GET', base+f'/tasks/{task["id"]}/{task["dispatch"]}', headers={'Authorization':'Bearer '+other.read_text().strip()})[0], 400)
        caps = self.direct('GET', '/api/v1/mesh/nodes')[1]['nodes'][0]
        self.assertTrue(caps['tasks'])
        self.assertFalse(caps['exec'])
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.direct('POST', base+'/tasks', offer)[0], 401)
        denied = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'tasks', '--command', json.dumps(dict(operation='claim',id=task['id'],dispatch=task['dispatch'],session=str(uuid.uuid4())))], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(denied.returncode, 0)
        self.assertFalse(self.worker(operation='list')['incoming'][0]['grant_active'])

    def test_changed_connection_blocks_delivery_and_does_not_move_an_ambiguous_attempt(self):
        task, _ = self.create()
        self.synced(task, 'available')
        self.api(operation='connect', node=self.target, url=self.url, token='c'*64)
        self.assertTrue(self.outgoing(task['id'])['blocked'])
        self.rejected(dict(operation='sync', id=task['id'], dispatch=task['dispatch']))
        self.rejected(dict(operation='retry', id=task['id'], dispatch=task['dispatch'], new_dispatch=str(uuid.uuid4())))
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['state'], 'available')
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        self.assertTrue(self.outgoing(task['id'])['blocked'])
        self.rejected(dict(operation='sync', id=task['id'], dispatch=task['dispatch']))

    def test_independent_mcp_claims_reports_and_rejects_missing_or_wrong_session(self):
        task, _ = self.create()
        self.synced(task, 'available')
        self.h.stop(self.process)
        session, question = str(uuid.uuid4()), str(uuid.uuid4())
        ask = dict(operation='ask', id=task['id'], dispatch=task['dispatch'], session=session, question=question, text='Which release should I inspect?')
        commands = [(dict(operation='list'), False),
                    (dict(operation='claim', id=task['id'], dispatch=task['dispatch'], session=session), False),
                    (dict(operation='claim', id=task['id'], dispatch=task['dispatch'], session=str(uuid.uuid4())), True),
                    (dict(operation='progress', id=task['id'], dispatch=task['dispatch'], session=session, text='Read the requested files.'), False),
                    (ask, False), (ask, False), ({**ask, 'session':str(uuid.uuid4())}, True),
                    (dict(operation='list', unknown=True), True),
                    (dict(operation='finish', id=task['id'], dispatch=task['dispatch'], session=session, outcome='succeeded', result='Done with evidence.'), False),
                    (dict(operation='create', node=self.caller, title='Missing stable IDs', description='Must fail.'), True)]
        requests = [dict(jsonrpc='2.0', id=0, method='tools/list')]
        requests += [dict(jsonrpc='2.0', id=index+1, method='tools/call', params={'name':'device_tasks','arguments':command}) for index, (command, _) in enumerate(commands)]
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'messages', 'mcp'], input=''.join(json.dumps(request)+'\n' for request in requests), capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        replies = {reply['id']:reply for line in result.stdout.splitlines() if 'id' in (reply := json.loads(line))}
        tool = next(tool for tool in replies[0]['result']['tools'] if tool['name'] == 'device_tasks')
        variants = {variant['properties']['operation']['const']:variant for variant in tool['inputSchema']['oneOf']}
        self.assertIn('question', variants['ask']['required'])
        self.assertIn('session', variants['ask']['required'])
        self.assertIn('question', variants['answer']['required'])
        for index, (_, rejected) in enumerate(commands, 1):
            self.assertEqual('error' in replies[index] or replies[index]['result'].get('isError', False), rejected, replies[index])
        self.start_target()
        self.assertEqual(self.synced(task, 'succeeded')['current']['result'], 'Done with evidence.')
        self.assertFalse(self.config.exists())

    def agent(self, **command):
        return self.cli('agent-sessions', '--command', json.dumps(command))

    def test_session_targeting_survives_restart_and_requires_a_current_registration(self):
        session, other = str(uuid.uuid4()), str(uuid.uuid4())
        catalog = self.agent(operation='register', id=session, name='Build reviewer', harness='custom-cli')
        self.assertEqual(catalog['sessions'][0]['revision'], 1)
        self.agent(operation='register', id=other, name='Another Agent', harness='custom-cli')
        self.h.stop(self.h.server)
        self.h.stop(self.process)
        self.start_target()
        found = self.api(operation='agent_sessions', node=self.target, command={'operation':'list'})
        self.assertEqual(found['owner'], self.target)
        self.assertEqual(len(found['sessions']), 2)
        self.assertNotIn('token', json.dumps(found))
        self.assertNotIn('cwd', json.dumps(found))
        command = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target,
                       target_session=session, title='Review on a specific Agent', description='Report evidence using this session.')
        self.tasks(**command)
        self.tasks(**command)
        self.rejected({**command, 'target_session':other})
        self.synced(command, 'available')
        wrong = dict(operation='claim', id=command['id'], dispatch=command['dispatch'], session=other)
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'tasks', '--command', json.dumps(wrong)], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.call_worker(command, 'claim', session=session)
        self.agent(operation='end', id=session, revision=1)
        self.call_worker(command, 'finish', session=session, outcome='succeeded', result='Finished after ending the discovery registration.')
        self.assertEqual(self.synced(command, 'succeeded')['current']['session'], session)
        self.assertEqual(self.cli('identity')['routes'], [])
        self.assertFalse(self.config.exists())

    def test_agent_discovery_is_scoped_read_only_and_expired_reports_do_not_allow_claims(self):
        session = str(uuid.uuid4())
        self.agent(operation='register', id=session, name='Review', harness='custom')
        base = f'/api/v1/mesh/nodes/{self.target}/agent-sessions'
        self.assertEqual(self.direct('GET', base, headers={'Origin':'https://untrusted.example'})[0], 403)
        self.assertEqual(self.direct('GET', base, headers={'X-Cranibox-Target':'wrong-device'})[0], 409)
        self.assertEqual(self.direct('POST', base, {'operation':'end','id':session,'revision':1})[0], 405)
        self.assertEqual(self.h.post('/api/peers', {'operation':'agent_sessions','node':self.target,'command':{'operation':'end','id':session,'revision':1}})[0], 400)
        command = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target,
                       target_session=session, title='Wait for Agent renewal', description='Must not be claimed by a stale registration.')
        self.tasks(**command)
        self.synced(command, 'available')
        registry = self.config.parent / 'agent-sessions.json'
        ledger = json.loads(registry.read_text())
        ledger['sessions'][session].update(updated_at=int(time.time())-301, expires_at=int(time.time())-1)
        registry.write_text(json.dumps(ledger))
        args = dict(operation='claim', id=command['id'], dispatch=command['dispatch'], session=session)
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'tasks', '--command', json.dumps(args)], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.agent(operation='heartbeat', id=session, revision=1, status='busy')
        self.call_worker(command, 'claim', session=session)
        self.cli('revoke', '--node', self.caller)
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', self.h.root / 'projects-only.token', '--projects')
        no_messages = (self.h.root / 'projects-only.token').read_text().strip()
        self.assertEqual(self.direct('GET', base, headers={'Authorization':'Bearer '+no_messages})[0], 403)

    def test_mcp_agent_registration_uses_revisions_and_rejects_unknown_fields(self):
        session = str(uuid.uuid4())
        commands = [dict(operation='register', id=session, name='MCP reviewer', harness='custom-cli'),
                    dict(operation='heartbeat', id=session, revision=1, status='busy'),
                    dict(operation='heartbeat', id=session, revision=1, status='waiting'),
                    dict(operation='list', unknown='must be rejected'),
                    dict(operation='end', id=session, revision=2),
                    dict(operation='heartbeat', id=session, revision=3, status='ready')]
        requests = [dict(jsonrpc='2.0', id=index, method='tools/call', params={'name':'device_agent_sessions','arguments':command}) for index, command in enumerate(commands)]
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'messages', 'mcp'], input=''.join(json.dumps(request)+'\n' for request in requests), capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        replies = [json.loads(line)['result'] for line in result.stdout.splitlines()]
        self.assertFalse(replies[0].get('isError'), replies[0])
        self.assertFalse(replies[1].get('isError'), replies[1])
        self.assertTrue(replies[2].get('isError'), replies[2])
        self.assertTrue(replies[3].get('isError'), replies[3])
        self.assertFalse(replies[4].get('isError'), replies[4])
        self.assertTrue(replies[5].get('isError'), replies[5])
        self.assertEqual(self.agent(operation='list')['sessions'][0]['status'], 'ended')
        self.assertFalse(self.config.exists())

    def test_corrupt_ledger_is_preserved_and_never_treated_as_empty(self):
        task, _ = self.create()
        self.h.stop(self.h.web)
        self.path.write_text('broken ledger')
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.h.post('/api/peers', {'operation':'tasks','command':{'operation':'list'}})[0], 400)
        self.rejected(task)
        self.assertEqual(self.path.read_text(), 'broken ledger')


if __name__ == '__main__':
    unittest.main()
