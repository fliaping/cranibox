"""Reviewed, task-bound worker communication through real stdio and HTTP nodes.

Fixtures replace model CLIs; they do not replace the Rust MCP or task protocol.
"""
import json
import os
import select
import subprocess
import time
import unittest
import uuid

import test_task_runs as task_runs
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


class Channel:
    def __init__(self, case, task, binary=NODE):
        self.case, self.sequence = case, 0
        self.process = subprocess.Popen([
            str(binary), '__cranibox-task-mcp', str(case.config.with_name('peers.json')),
            task['id'], task['dispatch']], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True)
        case.addCleanup(self.close)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=4)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=4)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            stream.close()

    def send(self, method, params=None):
        self.sequence += 1
        self.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.sequence, method=method, params=params or {})) + '\n')
        self.process.stdin.flush()

    def receive(self):
        self.case.assertTrue(select.select([self.process.stdout], [], [], 12)[0], 'MCP response timed out')
        line = self.process.stdout.readline()
        self.case.assertTrue(line, 'MCP closed unexpectedly')
        response = json.loads(line)
        self.case.assertEqual(response['id'], self.sequence)
        self.case.assertNotIn(self.case.token, line)
        return response['result']

    def call(self, method, params=None):
        self.send(method, params)
        return self.receive()

    def tool(self, error=False, name='cranibox_task', **arguments):
        result = self.call('tools/call', dict(name=name, arguments=arguments))
        self.case.assertEqual(bool(result.get('isError')), error, result)
        text = result['content'][0]['text']
        return text if error else json.loads(text)


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class TaskWorkerMcpTests(unittest.TestCase):
    # Reuse setup/helpers without inheriting and rerunning the operator suite.
    setUp = task_runs.TaskRunsTests.setUp
    api = task_runs.TaskRunsTests.api
    tasks = task_runs.TaskRunsTests.tasks
    worker = task_runs.TaskRunsTests.worker
    direct = task_runs.TaskRunsTests.direct
    create = task_runs.TaskRunsTests.create
    outgoing = task_runs.TaskRunsTests.outgoing
    synced = task_runs.TaskRunsTests.synced
    cli = task_runs.TaskRunsTests.cli
    command = task_runs.TaskRunsTests.command
    start_target = task_runs.TaskRunsTests.start_target
    run_task = task_runs.TaskRunsTests.run_task
    denied = task_runs.TaskRunsTests.denied
    incoming = task_runs.TaskRunsTests.incoming
    review = task_runs.TaskRunsTests.review
    start = task_runs.TaskRunsTests.start
    settled = task_runs.TaskRunsTests.settled

    def active(self):
        (self.project/'hold').touch()
        self.options['communication'] = True
        task, review = self.review()
        self.start(task, review)
        until(lambda: (self.project/'invocations').exists())
        return task, review, Channel(self, task)

    def test_review_opt_in_is_digest_bound_for_both_adapters_and_default_remains_inert(self):
        import tomllib
        task, plain = self.review()
        self.assertNotIn('communication', plain['options'])
        self.assertNotIn('__cranibox-task-mcp', json.dumps(plain))
        for adapter in ('codex', 'claude_code'):
            options = dict(self.options, adapter=adapter, communication=True)
            review = self.run_task(task, 'prepare', options=options)
            self.assertNotEqual(plain['digest'], review['digest'])
            self.denied(task, 'start', options=options, digest=plain['digest'])
            args = review['launch']['args']
            if adapter == 'codex':
                setting = next(arg for arg in args if arg.startswith('mcp_servers.cranibox_task='))
                config = tomllib.loads(setting)['mcp_servers']['cranibox_task']
                self.assertTrue(config['required'])
                self.assertEqual(config['enabled_tools'], ['cranibox_task'])
                self.assertLess(args.index(setting), args.index('exec'))
            else:
                config = json.loads(args[args.index('--mcp-config') + 1])['mcpServers']['cranibox_task']
                self.assertIn('--strict-mcp-config', args)
                self.assertNotIn('--disallowedTools', args)
                self.assertEqual(args[args.index('--tools') + 1], 'Read,Glob,Grep')
                self.assertIn('mcp__cranibox_task__cranibox_task', args[args.index('--allowedTools') + 1])
            self.assertEqual(config['args'], ['__cranibox-task-mcp', str(self.config.with_name('peers.json')), task['id'], task['dispatch']])
            self.assertTrue(os.path.isabs(config['command']))
            self.assertNotIn(self.token, json.dumps(review))
            self.assertIn('operator must review', review['launch']['stdin'])
        self.assertFalse((self.project/'invocations').exists())
        (self.project/'hold').touch()
        self.start(task, plain)
        blocked = subprocess.run([str(NODE), '__cranibox-task-mcp', str(self.config.with_name('peers.json')), task['id'], task['dispatch']], input='', capture_output=True, text=True, timeout=10)
        self.assertNotEqual(blocked.returncode, 0)
        self.assertIn('disabled', blocked.stderr)

    def test_question_wait_answer_candidate_and_explicit_report_survive_observer_disconnect(self):
        task, _, channel = self.active()
        self.h.stop(self.h.server)  # No fixed gateway/reverse route required.
        context = channel.tool(operation='get')
        self.assertEqual(context['task']['id'], task['id'])
        self.assertTrue(context['review_required'])
        progress = channel.tool(operation='progress', text='Checked the project; need the release revision.')
        question = str(uuid.uuid4())
        arguments = dict(operation='ask', question=question, text='Which release revision?')
        asked = channel.tool(**arguments)
        self.assertEqual(channel.tool(**arguments), asked)
        self.assertGreater(asked['current']['revision'], progress['current']['revision'])
        self.synced(task, 'running')
        until(lambda: self.outgoing(task['id'])['current'].get('questions'))
        channel.send('tools/call', dict(name='cranibox_task', arguments=dict(operation='wait', after_revision=asked['current']['revision'], timeout_secs=10)))
        self.tasks(operation='answer', id=task['id'], dispatch=task['dispatch'], question=question, text='Review release-42. 请附上证据。')
        self.tasks(operation='sync', id=task['id'], dispatch=task['dispatch'])
        reply = json.loads(channel.receive()['content'][0]['text'])
        self.assertEqual(reply['current']['questions'][0]['answer'], 'Review release-42. 请附上证据。')
        self.h.stop(self.h.web)
        proposal = dict(operation='propose', outcome='succeeded', result='Reviewed release-42; inspected two files. <img src=x onerror=unsafe()>')
        saved = channel.tool(**proposal)
        self.assertEqual(channel.tool(**proposal), saved)
        channel.tool(operation='propose', outcome='failed', result='Silently replace the first candidate.', error=True)
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        self.assertEqual(self.run_task(task, 'get')['run']['proposal'], saved['proposal'])
        self.assertIn('process to stop', self.denied(task, 'finish', outcome='succeeded', result='Must not report while active.'))
        (self.project/'release').touch()
        final = self.settled(task)
        self.assertEqual(final['execution']['state'], 'succeeded')
        self.assertEqual(final['run']['proposal'], saved['proposal'])
        channel.tool(operation='get', error=True)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.synced(task, 'running')['current']['questions'][0]['answer'], 'Review release-42. 请附上证据。')
        self.run_task(task, 'finish', outcome='failed', result='Operator inspected the candidate; evidence is incomplete.')
        self.assertEqual(self.synced(task, 'failed')['current']['result'], 'Operator inspected the candidate; evidence is incomplete.')
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(self.run_task(task, 'get')['run']['proposal'], saved['proposal'])
        self.assertEqual(self.cli('identity')['routes'], [])

    def test_all_binary_entries_expose_only_the_bound_tool_and_validate_arguments(self):
        task, _, channel = self.active()
        for binary in (NODE, CORE, RUNTIME, WEB):
            candidate = Channel(self, task, binary)
            self.assertEqual(candidate.call('initialize', {'protocolVersion':'2025-06-18'})['serverInfo']['name'], 'cranibox-task-worker')
            tools = candidate.call('tools/list')['tools']
            self.assertEqual([tool['name'] for tool in tools], ['cranibox_task'])
            self.assertEqual(candidate.tool(operation='get')['dispatch'], task['dispatch'])
            candidate.close()
        for arguments in (
            dict(operation='get', id=str(uuid.uuid4())), dict(operation='get', node=self.caller),
            dict(operation='finish', outcome='succeeded', result='Cannot finish.'), dict(operation='start'),
            dict(operation='wait', after_revision=0, timeout_secs=0), dict(operation='wait', after_revision=0, timeout_secs=31),
            dict(operation='wait', after_revision=-1, timeout_secs=1), dict(operation='progress', text='🙂'*1025),
            dict(operation='ask', question=str(uuid.UUID(int=0)), text='Invalid ID'),
            dict(operation='ask', question=str(uuid.uuid4()), text=' '),
            dict(operation='propose', outcome='succeeded', result='🙂'*2049),
        ):
            channel.tool(**arguments, error=True)
        for name in ('device_tasks', 'device_agent_runs', 'workspace_exec'):
            channel.tool(name=name, operation='list', error=True)
        current = channel.tool(operation='get')
        began = time.monotonic()
        self.assertEqual(channel.tool(operation='wait', after_revision=current['current']['revision'], timeout_secs=1), current)
        self.assertGreaterEqual(time.monotonic() - began, .9)
        self.assertIsNone(self.run_task(task, 'get')['run']['proposal'])
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def test_cancellation_wakes_wait_and_old_channel_cannot_touch_a_retry(self):
        task, _, channel = self.active()
        context = channel.tool(operation='get')
        channel.send('tools/call', dict(name='cranibox_task', arguments=dict(operation='wait', after_revision=context['current']['revision'], timeout_secs=10)))
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        self.synced(task, 'cancel_requested')
        reply = json.loads(channel.receive()['content'][0]['text'])
        self.assertEqual(reply['current']['state'], 'cancel_requested')
        channel.tool(operation='ask', question=str(uuid.uuid4()), text='Cannot ask during cancellation.', error=True)
        channel.tool(operation='propose', outcome='cancelled', result='Stopped at a safe boundary; operator must inspect partial output.')
        self.run_task(task, 'stop')
        self.settled(task)
        self.run_task(task, 'finish', outcome='cancelled', result='Inspected saved work.')
        self.synced(task, 'cancelled')
        retry = dict(task, dispatch=str(uuid.uuid4()))
        self.tasks(operation='retry', id=task['id'], dispatch=task['dispatch'], new_dispatch=retry['dispatch'])
        self.synced(retry, 'available')
        review = self.run_task(retry, 'prepare', options=self.options)
        self.start(retry, review)
        until(lambda: (self.project/'invocations').read_text() == 'xx')
        channel.tool(operation='get', error=True)
        channel.tool(operation='progress', text='Old worker must not alter new attempt.', error=True)
        new = Channel(self, retry)
        self.assertEqual(new.tool(operation='get')['dispatch'], retry['dispatch'])
        self.assertIsNone(self.run_task(retry, 'get')['run']['proposal'])

    def test_both_harnesses_boot_the_injected_server_and_deliver_questions_and_candidates(self):
        fixture = '#!' + __import__('sys').executable + '\n' + r"""import json,pathlib,subprocess,sys,tomllib,uuid
root = pathlib.Path.cwd()
(root/'prompt').write_text(sys.stdin.read())
args = sys.argv[1:]
if '--mcp-config' in args:
    config = json.loads(args[args.index('--mcp-config')+1])['mcpServers']['cranibox_task']
else:
    config = tomllib.loads(next(arg for arg in args if arg.startswith('mcp_servers.cranibox_task=')))['mcp_servers']['cranibox_task']
p = subprocess.Popen([config['command'], *config['args']], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
sequence = 0
def call(**arguments):
    global sequence
    sequence += 1
    p.stdin.write(json.dumps(dict(jsonrpc='2.0', id=sequence, method='tools/call', params=dict(name='cranibox_task', arguments=arguments)))+'\n')
    p.stdin.flush()
    reply = json.loads(p.stdout.readline())['result']
    assert not reply.get('isError'), reply
    return json.loads(reply['content'][0]['text'])
try:
    context = call(operation='get')
    call(operation='progress', text='Harness connected to its reviewed task.')
    context = call(operation='ask', question=str(uuid.uuid4()), text='Which revision should the harness inspect?')
    while not context['current']['questions'][0]['answer']:
        context = call(operation='wait', after_revision=context['current']['revision'], timeout_secs=2)
    context = call(operation='propose', outcome='succeeded', result='Inspected '+context['current']['questions'][0]['answer']+'; ready for operator review.')
    (root/'harness-candidate.json').write_text(json.dumps(context['proposal']))
    print('Task MCP harness completed. 中文🙂', flush=True)
finally:
    p.stdin.close()
    p.wait(timeout=5)
"""
        for name in ('codex', 'claude'):
            (self.tools/name).write_text(fixture)
        self.options['communication'] = True
        for adapter in ('codex', 'claude_code'):
            self.options['adapter'] = adapter
            task, review = self.review()
            self.start(task, review)
            self.synced(task, 'running')
            item = until(lambda: value if (value := self.outgoing(task['id']))['current'].get('questions') else None)
            self.assertEqual(item['current']['progress'], 'Harness connected to its reviewed task.')
            self.tasks(operation='answer', id=task['id'], dispatch=task['dispatch'], question=item['current']['questions'][0]['id'], text=adapter+'-release')
            self.tasks(operation='sync', id=task['id'], dispatch=task['dispatch'])
            detail = self.settled(task)
            self.assertEqual(detail['execution']['state'], 'succeeded', detail)
            self.assertIn('Task MCP harness completed. 中文🙂', detail['stdout']['text'])
            self.assertEqual(detail['run']['proposal']['result'], 'Inspected '+adapter+'-release; ready for operator review.')
            self.assertEqual(detail['run']['proposal'], json.loads((self.project/'harness-candidate.json').read_text()))
            self.assertEqual(next(item for item in self.incoming() if item['task']['id'] == task['id'])['current']['state'], 'running')
            self.run_task(task, 'finish', outcome='succeeded', result='Operator verified the fixture round trip.')
            self.synced(task, 'succeeded')

    def test_revocation_is_rechecked_on_an_open_channel_and_new_grant_cannot_restore_it(self):
        task, _, channel = self.active()
        context = channel.tool(operation='get')
        channel.send('tools/call', dict(name='cranibox_task', arguments=dict(operation='wait', after_revision=context['current']['revision'], timeout_secs=10)))
        self.cli('revoke', '--node', self.caller)
        rejected = channel.receive()
        self.assertTrue(rejected.get('isError'))
        self.assertIn('revoked', rejected['content'][0]['text'])
        channel.tool(operation='get', error=True)
        channel.tool(operation='progress', text='Must not write after revocation.', error=True)
        self.cli('grant', '--node', self.caller, '--output-token', self.h.root/'rotated.token', '--messages')
        channel.tool(operation='get', error=True)
        self.assertEqual(self.run_task(task, 'get')['execution']['state'], 'running')
        self.assertIsNone(self.run_task(task, 'get')['run']['proposal'])


if __name__ == '__main__':
    unittest.main()
