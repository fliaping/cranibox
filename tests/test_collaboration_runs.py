"""Durable coordination namespaces over real independently identified Rust nodes."""
import json
import os
import subprocess
import unittest
import uuid

import test_device_tasks as device_tasks
import test_task_runs as runs
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, free_port, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class CollaborationTests(unittest.TestCase):
    setUp = device_tasks.DeviceTasksTests.setUp
    cli = device_tasks.DeviceTasksTests.cli
    api = device_tasks.DeviceTasksTests.api
    tasks = device_tasks.DeviceTasksTests.tasks
    worker = device_tasks.DeviceTasksTests.worker
    direct = device_tasks.DeviceTasksTests.direct
    start_target = device_tasks.DeviceTasksTests.start_target
    outgoing = device_tasks.DeviceTasksTests.outgoing
    synced = device_tasks.DeviceTasksTests.synced
    call_worker = device_tasks.DeviceTasksTests.call_worker
    rejected = device_tasks.DeviceTasksTests.rejected

    def collaboration(self, operation, **values):
        return self.tasks(operation='collaboration', command=dict(operation=operation, **values))

    def create_run(self, members=None):
        command = dict(id=str(uuid.uuid4()), name='Release review', goal='Review the same revision; report evidence. 不自动执行。', members=members or [self.target], project_id=str(uuid.uuid4()))
        self.collaboration('create', **command)
        return command

    def delegate(self, run, node=None):
        command = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=node or self.target, title='Review files', description='Report verification evidence.', run_id=run['id'])
        self.tasks(**command)
        return command

    def view(self, run):
        return next(item for item in self.tasks(operation='list')['collaborations'] if item['scope']['id'] == run['id'])

    def test_two_members_receive_only_bound_context_and_results_and_questions_aggregate(self):
        settings = self.h.root/'second/settings.json'
        def second(*args):
            result = subprocess.run([str(NODE), '--config', str(settings), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        peer = second('identity')['node_id']
        token_file = self.h.root/'second.token'
        second('grant', '--node', self.caller, '--messages', '--output-token', token_file)
        port = free_port()
        second('listen', '--address', f'127.0.0.1:{port}')
        self.h.start(NODE, '--config', settings, 'peer', 'serve')
        until(lambda: self.h.http('GET', '/api/v1/mesh/nodes', port=port, headers={'Authorization':'Bearer '+token_file.read_text().strip(), 'X-Cranibox-Target':peer})[0] == 200)
        self.api(operation='connect', node=peer, url=f'http://127.0.0.1:{port}', token=token_file.read_text().strip())
        run = self.create_run([peer, self.target])
        first, other = self.delegate(run), self.delegate(run, peer)
        expected = dict(id=run['id'], owner=self.caller, name=run['name'], goal=run['goal'], project_id=run['project_id'])
        self.assertEqual(self.synced(first, 'available')['task']['collaboration'], expected)
        self.synced(other, 'available')
        received = second('tasks', '--command', json.dumps({'operation':'list'}))
        self.assertEqual(received['incoming'][0]['task']['collaboration'], expected)
        self.assertEqual(received['collaborations'], [], 'receiving context must not create coordinator authority')
        self.assertEqual(second('identity')['routes'], [])
        self.assertNotIn('bindings', json.dumps(self.view(run)))
        session, question = str(uuid.uuid4()), str(uuid.uuid4())
        self.call_worker(first, 'claim', session=session)
        self.call_worker(first, 'ask', session=session, question=question, text='Which revision?')
        self.synced(first, 'running')
        until(lambda: self.view(run)['stats']['questions'] == 1)
        self.tasks(operation='answer', id=first['id'], dispatch=first['dispatch'], question=question, text='Release 42')
        self.tasks(operation='sync', id=first['id'], dispatch=first['dispatch'])
        until(lambda: self.worker(operation='list')['incoming'][0]['current']['questions'][0]['answer'] == 'Release 42')
        self.call_worker(first, 'finish', session=session, outcome='succeeded', result='Reviewed release 42.')
        second('tasks', '--command', json.dumps(dict(operation='decline', id=other['id'], dispatch=other['dispatch'], reason='Required tool unavailable.')))
        self.synced(first, 'succeeded'); self.synced(other, 'failed')
        self.assertEqual(self.view(run)['stats'], dict(total=2, active=0, succeeded=1, failed=1, cancelled=0, questions=0, blocked=0, waiting=0))
        self.collaboration('close', id=run['id'], revision=1, result='Reviewed both reports; one device needs a dependency.')
        self.assertEqual(self.view(run)['state'], 'closed')
        self.rejected(dict(operation='retry', id=other['id'], dispatch=other['dispatch'], new_dispatch=str(uuid.uuid4())))

    def test_offline_stop_survives_restart_and_never_closes_before_final_receipt(self):
        run = self.create_run()
        active = self.delegate(run)
        self.synced(active, 'available')
        session = str(uuid.uuid4())
        self.call_worker(active, 'claim', session=session)
        self.synced(active, 'running')
        self.h.stop(self.process)
        queued = self.delegate(run)
        stop = dict(id=run['id'], revision=1)
        self.collaboration('stop', **stop); self.collaboration('stop', **stop)
        self.assertEqual(self.view(run)['stats']['active'], 2)
        self.assertTrue(all(item['cancel_requested'] for item in self.tasks(operation='list')['outgoing']))
        self.rejected(dict(operation='collaboration', command=dict(operation='close', id=run['id'], revision=2, result='Not actually stopped.')))
        self.rejected(dict(queued, id=str(uuid.uuid4())))
        self.tasks(**queued)  # An uncertain response is recoverable after stop without creating work.
        self.h.stop(self.h.web); self.h.start_web(); self.h.login()
        self.assertEqual(self.view(run)['state'], 'stopping')
        self.start_target()
        self.synced(active, 'cancel_requested'); self.synced(queued, 'cancelled')
        self.call_worker(active, 'finish', session=session, outcome='cancelled', result='Stopped safely after saving the report.')
        self.synced(active, 'cancelled')
        self.collaboration('close', id=run['id'], revision=2, result='Both tasks explicitly confirmed cancellation.')
        self.collaboration('archive', id=run['id'], revision=3)
        view = self.tasks(operation='list')
        self.assertEqual(view['collaborations'], []); self.assertEqual(view['outgoing'], [])
        self.assertEqual(view['collaboration_used'], 1, 'archive retains deduplication until retention expires')

    def test_member_changes_and_credential_rotation_do_not_redirect_new_or_existing_work(self):
        run = self.create_run()
        task = self.delegate(run); self.synced(task, 'available')
        self.api(operation='connect', node='outsider', url=self.url, token=self.token)
        rejected = dict(task, id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node='outsider')
        self.rejected(rejected)
        self.api(operation='connect', node=self.target, url=self.url, token='c'*64)
        self.assertEqual(self.view(run)['stats']['blocked'], 1)
        self.rejected(dict(task, id=str(uuid.uuid4()), dispatch=str(uuid.uuid4())))
        self.collaboration('stop', id=run['id'], revision=1)
        self.assertTrue(self.outgoing(task['id'])['cancel_requested'])
        self.assertTrue(self.outgoing(task['id'])['blocked'])
        self.assertEqual(self.worker(operation='list')['incoming'][0]['current']['state'], 'available')
        self.assertNotIn(self.token, self.path.read_text())

    def test_messages_mcp_exposes_coordination_without_process_launch(self):
        run = self.create_run()
        command = dict(operation='collaboration', command=dict(operation='create', **run))
        requests = [dict(jsonrpc='2.0', id=1, method='tools/list'), dict(jsonrpc='2.0', id=2, method='tools/call', params={'name':'device_tasks', 'arguments':command}), dict(jsonrpc='2.0', id=3, method='tools/call', params={'name':'device_tasks', 'arguments':{'operation':'run', 'id':str(uuid.uuid4()), 'dispatch':str(uuid.uuid4()), 'command':{'operation':'get'}}})]
        # Use the coordinator's existing peer configuration; this does not need relay settings.
        settings = self.path.with_name('settings.json')
        result = subprocess.run([str(NODE), '--config', str(settings), 'peer', 'messages', 'mcp'], input=''.join(json.dumps(request)+chr(10) for request in requests), capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        replies = {response['id']:response for line in result.stdout.splitlines() if 'id' in (response := json.loads(line))}
        tool = next(tool for tool in replies[1]['result']['tools'] if tool['name'] == 'device_tasks')
        variants = {item['properties']['operation']['const']:item for item in tool['inputSchema']['oneOf']}
        self.assertIn('run_id', variants['create']['properties'])
        self.assertEqual(variants['create']['properties']['depends_on']['maxItems'], 8)
        self.assertIn('collaboration', variants)
        self.assertNotIn('run', variants)
        self.assertFalse(replies[2]['result'].get('isError', False), replies[2])
        self.assertTrue('error' in replies[3] or replies[3]['result'].get('isError', False))
        self.assertEqual(len(self.tasks(operation='list')['collaborations']), 1)


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class CollaborationPromptTests(unittest.TestCase):
    setUp = runs.TaskRunsTests.setUp
    cli = runs.TaskRunsTests.cli
    command = runs.TaskRunsTests.command
    api = runs.TaskRunsTests.api
    tasks = runs.TaskRunsTests.tasks
    worker = runs.TaskRunsTests.worker
    direct = runs.TaskRunsTests.direct
    start_target = runs.TaskRunsTests.start_target
    outgoing = runs.TaskRunsTests.outgoing
    synced = runs.TaskRunsTests.synced
    run_task = runs.TaskRunsTests.run_task

    def test_review_carries_the_exact_scope_as_external_data_and_does_not_start_an_agent(self):
        run = dict(id=str(uuid.uuid4()), name='Review', goal='Inspect the current project. <img src=x>', members=[self.target], project_id=str(uuid.uuid4()))
        self.tasks(operation='collaboration', command=dict(operation='create', **run))
        task = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target, title='Review files', description='Report evidence.', run_id=run['id'])
        self.tasks(**task); self.synced(task, 'available')
        review = self.run_task(task, 'prepare', options=self.options)
        for value in (run['id'], run['goal'], run['project_id'], self.caller):
            self.assertIn(value, review['launch']['stdin'])
        self.assertFalse((self.project/'invocations').exists())
        self.assertNotIn(self.token, json.dumps(review))
        self.assertIsNone(self.worker(operation='list')['incoming'][0]['run'])
