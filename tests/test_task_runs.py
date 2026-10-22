"""Reviewed local task execution across two independent nodes, without model calls."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

import test_device_tasks as device_tasks
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class TaskRunsTests(unittest.TestCase):
    api = device_tasks.DeviceTasksTests.api
    tasks = device_tasks.DeviceTasksTests.tasks
    worker = device_tasks.DeviceTasksTests.worker
    direct = device_tasks.DeviceTasksTests.direct
    create = device_tasks.DeviceTasksTests.create
    outgoing = device_tasks.DeviceTasksTests.outgoing
    synced = device_tasks.DeviceTasksTests.synced

    def setUp(self):
        tools = tempfile.TemporaryDirectory(prefix='cranibox-task-tools-')
        self.addCleanup(tools.cleanup)
        self.tools = Path(tools.name)
        fixture = '#!' + sys.executable + '\n' + '''import pathlib,sys,time
root = pathlib.Path.cwd()
with (root/'invocations').open('a') as file: file.write('x')
(root/'prompt').write_text(sys.stdin.read())
print('审查🙂' * 12000, flush=True)
print('fixture diagnostic', file=sys.stderr, flush=True)
if (root/'hold').exists():
    deadline = time.monotonic() + 40
    while not (root/'release').exists() and time.monotonic() < deadline: time.sleep(.05)
print('fixture finished', flush=True)
'''
        for name in ('codex', 'claude'):
            path = self.tools/name
            path.write_text(fixture)
            path.chmod(0o700)
        device_tasks.DeviceTasksTests.setUp(self)
        self.project = self.h.root/'project'
        self.project.mkdir()
        self.options = dict(adapter='codex', access='read_only', cwd=str(self.project))

    def cli(self, *args):
        result = self.command(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def command(self, *args):
        return subprocess.run([str(NODE), '--config', str(self.config), 'peer', *map(str, args)], env={**os.environ, 'PATH':str(self.tools)}, capture_output=True, text=True, timeout=15)

    def start_target(self):
        self.h.node_env['PATH'] = str(self.tools)
        device_tasks.DeviceTasksTests.start_target(self)

    def run_task(self, task, operation, **values):
        return self.worker(operation='run', id=task['id'], dispatch=task['dispatch'], command=dict(operation=operation, **values))

    def denied(self, task, operation, **values):
        command = dict(operation='run', id=task['id'], dispatch=task['dispatch'], command=dict(operation=operation, **values))
        result = self.command('tasks', '--command', json.dumps(command))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        return result.stderr

    def incoming(self):
        return self.worker(operation='list')['incoming']

    def review(self, description='Read the project and provide evidence. $(touch not-a-command)'):
        task, _ = self.create(description)
        self.synced(task, 'available')
        return task, self.run_task(task, 'prepare', options=self.options)

    def start(self, task, review):
        return self.run_task(task, 'start', options=review['options'], digest=review['digest'])

    def settled(self, task):
        return until(lambda: value if (value := self.run_task(task, 'get'))['execution']['state'] in ('succeeded', 'failed', 'cancelled', 'interrupted') else None)

    def test_review_is_inert_and_targeted_sessions_cannot_be_replaced(self):
        task, review = self.review()
        self.assertIn(task['id'], review['launch']['stdin'])
        self.assertIn(task['description'], review['launch']['stdin'])
        self.assertEqual(review['owner'], self.target)
        self.assertEqual(review['options'], self.options)
        self.assertTrue(review['available'])
        self.assertEqual(review['launch']['args'][-1], '-')
        self.assertNotIn(task['description'], json.dumps(review['launch']['args']))
        self.assertFalse((self.project/'invocations').exists())
        self.assertIsNone(self.incoming()[0]['run'])
        self.denied(task, 'start', options=self.options, digest='0'*64)
        self.assertEqual(self.incoming()[0]['current']['state'], 'available')
        self.assertIsNone(self.incoming()[0]['run'])
        self.denied(task, 'prepare', options={**self.options, 'cwd':str(self.project/'missing')})
        (self.tools/'codex').unlink()
        missing = self.run_task(task, 'prepare', options=self.options)
        self.assertFalse(missing['available'])
        self.denied(task, 'start', options=self.options, digest=missing['digest'])
        self.assertEqual(self.incoming()[0]['current']['state'], 'available')
        targeted = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target, title='Use the existing session', description='Do not replace it.', target_session=str(uuid.uuid4()))
        self.tasks(**targeted)
        self.synced(targeted, 'available')
        self.assertIn('existing Agent session', self.denied(targeted, 'prepare', options=self.options))

    def test_once_only_launch_survives_observer_restart_and_requires_explicit_result(self):
        task, review = self.review()
        first = self.start(task, review)
        again = self.start(task, review)
        self.assertEqual(first['run'], again['run'])
        self.assertEqual(first['execution']['session_id'], self.incoming()[0]['current']['session'])
        self.h.stop(self.h.web)
        final = self.settled(task)
        self.assertEqual(final['execution']['state'], 'succeeded')
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        self.assertEqual((self.project/'prompt').read_text(), review['launch']['stdin'])
        self.assertFalse((self.project/'not-a-command').exists())
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        output, offset = '', 0
        while True:
            page = self.run_task(task, 'get', stdout_offset=offset)
            text = page['stdout']['text']
            self.assertLessEqual(len(text.encode()), 65536)
            self.assertNotIn('�', text)
            self.assertEqual(page['stdout']['next_offset'], offset + len(text.encode()))
            if not text:
                break
            offset = page['stdout']['next_offset']
            output += text
        self.assertEqual(output, '审查🙂'*12000 + '\nfixture finished\n')
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.synced(task, 'running')['current']['session'], first['run']['session'])
        self.run_task(task, 'finish', outcome='failed', result='The process exited successfully, but the requested evidence is incomplete.')
        report = self.synced(task, 'failed')
        self.assertIn('evidence is incomplete', report['current']['result'])
        self.run_task(task, 'finish', outcome='failed', result=report['current']['result'])
        self.denied(task, 'finish', outcome='succeeded', result='Replace the reviewed report.')
        self.assertEqual(self.start(task, review)['run'], first['run'])
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        self.assertEqual(self.cli('identity')['routes'], [])

    def test_cancel_stop_and_retry_preserve_attempt_and_process_identity(self):
        (self.project/'hold').touch()
        task, review = self.review()
        first = self.start(task, review)
        until(lambda: (self.project/'invocations').exists())
        self.assertIn('process to stop', self.denied(task, 'finish', outcome='succeeded', result='Premature result.'))
        premature = dict(operation='finish', id=task['id'], dispatch=task['dispatch'], session=first['run']['session'], outcome='succeeded', result='Cannot bypass the process check through the ordinary task command.')
        self.assertNotEqual(self.command('tasks', '--command', json.dumps(premature)).returncode, 0)
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        self.synced(task, 'cancel_requested')
        self.assertEqual(self.run_task(task, 'get')['execution']['state'], 'running')
        self.run_task(task, 'stop')
        self.assertEqual(self.settled(task)['execution']['state'], 'cancelled')
        self.assertEqual(self.incoming()[0]['current']['state'], 'cancel_requested')
        self.run_task(task, 'finish', outcome='cancelled', result='Stopped the task process and inspected the partial output.')
        self.synced(task, 'cancelled')
        next_dispatch = str(uuid.uuid4())
        self.tasks(operation='retry', id=task['id'], dispatch=task['dispatch'], new_dispatch=next_dispatch)
        retry = {**task, 'dispatch':next_dispatch}
        self.synced(retry, 'available')
        self.assertIn('stale dispatch', self.denied(task, 'get'))
        (self.project/'hold').unlink()
        next_review = self.run_task(retry, 'prepare', options=self.options)
        second = self.start(retry, next_review)
        self.assertNotEqual(second['run']['id'], first['run']['id'])
        self.assertNotEqual(second['run']['session'], first['run']['session'])
        self.settled(retry)
        self.assertEqual((self.project/'invocations').read_text(), 'xx')

    def test_reserved_claim_recovers_only_by_explicit_retry_after_queue_failure(self):
        task, review = self.review()
        self.h.stop(self.process)
        for _ in range(8):
            self.cli('runs', '--command', json.dumps(dict(operation='start', id=str(uuid.uuid4()), launch=dict(name='Queued fixture', harness='fixture', program=sys.executable, args=['-c', 'pass'], cwd=str(self.project)))))
        self.assertIn('eight runs', self.denied(task, 'start', options=self.options, digest=review['digest']))
        reserved = self.run_task(task, 'get')
        self.assertIsNone(reserved['execution'])
        self.assertEqual(self.incoming()[0]['current']['session'], reserved['run']['session'])
        self.assertFalse((self.project/'invocations').exists())
        self.start_target()
        self.assertIsNone(self.run_task(task, 'get')['execution'])
        self.assertFalse((self.project/'invocations').exists())
        launched = self.start(task, review)
        self.assertEqual(launched['run'], reserved['run'])
        self.settled(task)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(self.start(task, review)['execution']['state'], 'succeeded')
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def test_message_permissions_and_mcp_cannot_trigger_execution(self):
        task, review = self.review()
        command = dict(operation='run', id=task['id'], dispatch=task['dispatch'], command=dict(operation='start', options=self.options, digest=review['digest']))
        request = dict(jsonrpc='2.0', id=1, method='tools/call', params=dict(name='device_tasks', arguments=command))
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'messages', 'mcp'], input=json.dumps(request)+'\n', env={**os.environ, 'PATH':str(self.tools)}, capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['result']['isError'])
        status, _ = self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/tasks', command)
        self.assertEqual(status, 422)
        self.assertIsNone(self.incoming()[0]['run'])
        self.cli('revoke', '--node', self.caller)
        self.assertIn('grant revoked', self.denied(task, 'start', options=self.options, digest=review['digest']))
        self.assertFalse((self.project/'invocations').exists())

    def test_revoking_delivery_does_not_hide_or_kill_a_locally_confirmed_run(self):
        (self.project/'hold').touch()
        task, review = self.review()
        first = self.start(task, review)
        until(lambda: (self.project/'invocations').exists())
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.run_task(task, 'get')['run'], first['run'])
        self.assertEqual(self.run_task(task, 'get')['execution']['state'], 'running')
        self.assertEqual(self.start(task, review)['run'], first['run'])
        self.run_task(task, 'stop')
        self.settled(task)
        self.run_task(task, 'finish', outcome='cancelled', result='Stopped locally after revoking the delivery grant.')
        self.assertEqual(self.incoming()[0]['current']['state'], 'cancelled')
        self.assertEqual((self.project/'invocations').read_text(), 'x')


if __name__ == '__main__':
    unittest.main()
