"""Opt-in receiver policies across real Rust services; model CLIs are local fixtures."""
import json
import os
import subprocess
import sys
import time
import unittest
import uuid

import test_task_runs as runs
from test_task_worker_mcp import Channel
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class AutomaticTaskWorkerTests(unittest.TestCase):
    api = runs.TaskRunsTests.api
    tasks = runs.TaskRunsTests.tasks
    worker = runs.TaskRunsTests.worker
    direct = runs.TaskRunsTests.direct
    create = runs.TaskRunsTests.create
    outgoing = runs.TaskRunsTests.outgoing
    synced = runs.TaskRunsTests.synced
    cli = runs.TaskRunsTests.cli
    command = runs.TaskRunsTests.command
    start_target = runs.TaskRunsTests.start_target
    run_task = runs.TaskRunsTests.run_task
    incoming = runs.TaskRunsTests.incoming
    settled = runs.TaskRunsTests.settled

    def setUp(self):
        runs.TaskRunsTests.setUp(self)
        for name in ('codex', 'claude'):
            path = self.tools/name
            path.write_text(path.read_text() + "sys.exit(int((root/'exit-code').read_text()) if (root/'exit-code').exists() else 0)\n")

    def policy(self, operation='get', **values):
        value = self.worker(operation='automatic', command=dict(operation=operation, **values))
        self.assertNotIn(self.token, json.dumps(value))
        return value

    def prepare(self, **changes):
        options = dict(enabled=True, senders=[self.caller], agent=dict(self.options, communication=True), parallel=1, report='human_review')
        options.update(changes)
        return self.policy('prepare', revision=self.policy()['revision'], options=options)

    def save(self, review):
        return self.policy('save', revision=review['revision'], options=review['options'], digest=review['digest'])

    def enable(self, **changes):
        return self.save(self.prepare(**changes))

    def rejected(self, operation, **values):
        value = self.command('tasks', '--command', json.dumps(dict(operation='automatic', command=dict(operation=operation, **values))))
        self.assertNotEqual(value.returncode, 0, value.stdout)
        return value.stderr

    def accepted(self, task):
        return until(lambda: item if (item := next((item for item in self.incoming() if item['task']['id'] == task['id']), None)) and item.get('run') else None)

    def test_default_and_prepare_are_inert_and_save_retries_are_idempotent(self):
        task, _ = self.create()
        self.synced(task, 'available')
        self.assertIsNone(self.policy()['policy'])
        review = self.prepare()
        time.sleep(2.4)
        self.assertFalse((self.project/'invocations').exists())
        self.assertIsNone(self.incoming()[0]['run'])
        self.rejected('save', revision=review['revision'], options=review['options'], digest='0'*64)
        first = self.save(review)
        self.assertEqual(self.save(review)['generation'], first['generation'])
        self.accepted(task)
        self.settled(task)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        self.assertEqual(self.policy()['runs'][0]['phase'], 'review_required')
        path = self.config.with_name('task-worker.json')
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertNotIn(self.token, path.read_text())

    def test_configuration_is_revision_grant_and_directory_bound(self):
        review = self.prepare()
        changed = dict(review['options'], parallel=2)
        self.rejected('save', revision=review['revision'], options=changed, digest=review['digest'])
        self.enable(enabled=False)
        self.assertIn('changed', self.rejected('save', revision=review['revision'], options=review['options'], digest=review['digest']))
        for changes in (dict(senders=[]), dict(senders=[self.caller]*2), dict(senders=['ungranted-node']), dict(parallel=5), dict(agent=dict(self.options, communication=False)), dict(agent=dict(self.options, communication=True, cwd='relative'))):
            options = dict(review['options'], **changes)
            self.rejected('prepare', revision=self.policy()['revision'], options=options)
        fresh = self.prepare()
        self.cli('revoke', '--node', self.caller)
        self.rejected('save', revision=fresh['revision'], options=fresh['options'], digest=fresh['digest'])
        self.assertFalse((self.project/'invocations').exists())

    def test_parallel_queue_pause_and_closed_observers(self):
        (self.project/'hold').touch()
        self.enable()
        tasks = [self.create('Inspect fixture task ' + str(i))[0] for i in range(3)]
        until(lambda: len(self.incoming()) == 3)
        until(lambda: (self.project/'invocations').exists())
        time.sleep(2.2)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        current = self.policy()
        paused = self.policy('pause', revision=current['revision'])
        self.assertFalse(paused['policy']['enabled'])
        self.assertEqual(self.policy('pause', revision=current['revision'])['revision'], paused['revision'])
        self.h.stop(self.h.web)
        self.assertEqual(sum(bool(item.get('run')) for item in self.incoming()), 1)
        (self.project/'release').touch()
        accepted = next(item for item in self.incoming() if item.get('run'))
        self.settled(dict(id=accepted['task']['id'], dispatch=accepted['current']['dispatch']))
        time.sleep(2.2)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        # A new explicit policy can drain the remaining durable queue without a UI.
        self.enable(parallel=2)
        until(lambda: (self.project/'invocations').read_text() == 'xxx')
        self.assertEqual(len({item['run']['id'] for item in self.incoming()}), 3)
        self.assertTrue(all(item['current']['state'] == 'running' for item in self.incoming()))
        self.assertEqual(self.cli('identity')['routes'], [])

    def test_proposal_is_reported_only_after_the_process_stops(self):
        (self.project/'hold').touch()
        self.enable(report='agent_proposal')
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        channel = Channel(self, task)
        self.assertFalse(channel.tool(operation='get')['review_required'])
        result = 'Inspected the fixture files; evidence is explicitly reported. <img src=x onerror=unsafe()>'
        channel.tool(operation='propose', outcome='succeeded', result=result)
        time.sleep(2.2)
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        (self.project/'release').touch()
        self.assertEqual(self.synced(task, 'succeeded')['current']['result'], result)
        self.assertEqual(self.policy()['runs'][0]['phase'], 'reported')
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def test_human_review_and_missing_proposals_never_autofinish(self):
        (self.project/'hold').touch()
        self.enable()
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        channel = Channel(self, task)
        self.assertTrue(channel.tool(operation='get')['review_required'])
        channel.tool(operation='propose', outcome='succeeded', result='Candidate needs human verification.')
        (self.project/'release').touch()
        self.settled(task)
        self.enable(report='agent_proposal')
        other, _ = self.create()
        self.accepted(other)
        self.settled(other)
        time.sleep(2.4)
        self.assertTrue(all(item['current']['state'] == 'running' for item in self.incoming()))
        self.run_task(task, 'finish', outcome='failed', result='Human review found missing evidence.')
        self.assertEqual(self.synced(task, 'failed')['current']['result'], 'Human review found missing evidence.')

    def test_cancel_stops_the_automatic_process_and_confirms_only_termination(self):
        (self.project/'hold').touch()
        self.enable()
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        result = self.synced(task, 'cancelled')
        self.assertIn('not rollback', result['current']['result'])
        self.assertEqual(self.run_task(task, 'get')['execution']['state'], 'cancelled')

    def test_failed_exit_cannot_report_a_success_proposal(self):
        (self.project/'hold').touch()
        (self.project/'exit-code').write_text('7')
        self.enable(report='agent_proposal')
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        Channel(self, task).tool(operation='propose', outcome='succeeded', result='This success claim must not override the failed process.')
        (self.project/'release').touch()
        self.assertEqual(self.settled(task)['execution']['exit_code'], 7)
        time.sleep(2.2)
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        self.assertEqual(self.policy()['runs'][0]['phase'], 'review_required')
        (self.project/'release').unlink()
        other, _ = self.create()
        self.accepted(other)
        until(lambda: (self.project/'invocations').read_text() == 'xx')
        Channel(self, other).tool(operation='propose', outcome='failed', result='Reported failure with process evidence.')
        (self.project/'release').touch()
        self.assertEqual(self.synced(other, 'failed')['current']['result'], 'Reported failure with process evidence.')

    def test_unanswered_question_blocks_reporting_after_exit(self):
        (self.project/'hold').touch()
        self.enable(report='agent_proposal')
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        channel = Channel(self, task)
        question = str(uuid.uuid4())
        channel.tool(operation='ask', question=question, text='Which result should be verified?')
        channel.tool(operation='propose', outcome='succeeded', result='Candidate saved while clarification is outstanding.')
        (self.project/'release').touch()
        self.settled(task)
        time.sleep(2.2)
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        self.synced(task, 'running')
        self.tasks(operation='answer', id=task['id'], dispatch=task['dispatch'], question=question, text='The fixture evidence has now been verified.')
        self.synced(task, 'succeeded')

    def test_revocation_stops_active_worker_and_new_grant_does_not_reenable_policy(self):
        (self.project/'hold').touch()
        self.enable()
        task, _ = self.create()
        self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.settled(task)['execution']['state'], 'cancelled')
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', self.h.root/'rotated.token', '--messages', '--inbound', 'accept')
        state = self.policy()
        self.assertFalse(state['senders'][0]['binding_active'])
        self.assertEqual(state['runs'][0]['phase'], 'grant_revoked')
        rotated = (self.h.root/'rotated.token').read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=rotated)
        other, _ = self.create()
        self.synced(other, 'available')
        time.sleep(2.2)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        self.assertEqual(sum(bool(item.get('run')) for item in self.incoming()), 1)

    def test_node_restart_never_replays_an_ambiguous_worker_or_reports_its_proposal(self):
        (self.project/'hold').touch()
        self.enable(report='agent_proposal')
        task, _ = self.create()
        accepted = self.accepted(task)
        until(lambda: (self.project/'invocations').exists())
        Channel(self, task).tool(operation='propose', outcome='succeeded', result='Candidate saved before interruption.')
        self.h.stop(self.process)
        self.start_target()
        time.sleep(2.4)
        self.assertEqual(self.run_task(task, 'get')['execution']['state'], 'interrupted')
        self.assertEqual(self.incoming()[0]['run'], accepted['run'])
        self.assertEqual(self.incoming()[0]['current']['state'], 'running')
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def fill_history(self):
        self.h.stop(self.process)
        self.cli('runs', '--command', json.dumps(dict(operation='start', id=str(uuid.uuid4()), launch=dict(name='History fixture', harness='fixture', program=sys.executable, args=['-c', 'pass'], cwd=str(self.project)))))
        path = self.config.with_name('agent-executions.json')
        ledger = json.loads(path.read_text())
        template = next(iter(ledger['records'].values()))
        template.update(state='succeeded', exit_code=0)
        ledger['records'] = {}
        for _ in range(128):
            identity = str(uuid.uuid4())
            ledger['records'][identity] = dict(template, id=identity, session_id=str(uuid.uuid4()))
        path.write_text(json.dumps(ledger))
        return path, ledger

    def test_full_history_reservation_resumes_once_without_changing_its_identity(self):
        path, ledger = self.fill_history()
        self.enable()
        task, _ = self.create()
        self.start_target()
        self.accepted(task)
        self.assertIsNone(self.run_task(task, 'get')['execution'])
        self.tasks(operation='cancel', id=task['id'], dispatch=task['dispatch'])
        self.assertIn('no process was launched', self.synced(task, 'cancelled')['current']['result'])
        self.assertFalse((self.project/'invocations').exists())
        task, _ = self.create()
        reserved = self.accepted(task)['run']
        self.assertIsNone(self.run_task(task, 'get')['execution'])
        self.assertEqual(next(item for item in self.policy()['runs'] if item['task'] == task['id'])['phase'], 'reserved')
        self.h.stop(self.process)
        ledger['records'].pop(next(iter(ledger['records'])))
        path.write_text(json.dumps(ledger))
        self.start_target()
        until(lambda: self.run_task(task, 'get')['execution'])
        self.settled(task)
        self.assertEqual(next(item for item in self.incoming() if item['task']['id'] == task['id'])['run'], reserved)
        self.assertEqual((self.project/'invocations').read_text(), 'x')
        time.sleep(2.2)
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def test_policy_changes_do_not_take_over_a_pending_reservation(self):
        path, ledger = self.fill_history()
        self.enable()
        task, _ = self.create()
        self.start_target()
        reserved = self.accepted(task)['run']
        state = self.policy()
        self.policy('pause', revision=state['revision'])
        self.assertEqual(self.policy()['runs'][0]['phase'], 'paused')
        self.enable()
        self.assertEqual(self.policy()['runs'][0]['phase'], 'review_required')
        self.h.stop(self.process)
        ledger['records'].pop(next(iter(ledger['records'])))
        path.write_text(json.dumps(ledger))
        self.start_target()
        time.sleep(2.4)
        self.assertFalse((self.project/'invocations').exists())
        self.assertEqual(self.incoming()[0]['run'], reserved)
        self.run_task(task, 'finish', outcome='failed', result='Policy changed before this reserved process could start.')
        self.synced(task, 'failed')
        time.sleep(2.2)
        self.assertFalse((self.project/'invocations').exists())

    def test_targeted_sessions_and_other_senders_are_never_taken_over(self):
        self.cli('grant', '--allow-unverified', '--node', 'other-node', '--output-token', self.h.root/'other.token', '--messages', '--inbound', 'accept')
        self.enable(senders=['other-node'])
        ordinary, _ = self.create()
        self.synced(ordinary, 'available')
        targeted = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target, title='Existing session only', description='Must remain addressed to its original session.', target_session=str(uuid.uuid4()))
        self.tasks(**targeted)
        self.synced(targeted, 'available')
        self.enable()
        self.accepted(ordinary)
        self.settled(ordinary)
        time.sleep(2.2)
        addressed = next(item for item in self.incoming() if item['task']['id'] == targeted['id'])
        self.assertIsNone(addressed['run'])
        self.assertEqual(addressed['current']['state'], 'available')
        self.assertEqual((self.project/'invocations').read_text(), 'x')

    def test_messages_mcp_and_remote_offer_cannot_configure_worker(self):
        command = dict(operation='automatic', command=dict(operation='get'))
        request = dict(jsonrpc='2.0', id=1, method='tools/call', params=dict(name='device_tasks', arguments=command))
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', 'messages', 'mcp'], input=json.dumps(request)+'\n', env={**os.environ, 'PATH':str(self.tools)}, capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['result']['isError'])
        self.assertEqual(self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/tasks', command)[0], 422)
        self.assertIsNone(self.policy()['policy'])


if __name__ == '__main__':
    unittest.main()
