"""Sender-local dependency gating over real HTTP workers and durable Rust stores."""
import json
import os
import unittest
import uuid

import test_collaboration_runs as collaborations
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class DependencyTests(unittest.TestCase):
    setUp = collaborations.CollaborationTests.setUp
    cli = collaborations.CollaborationTests.cli
    api = collaborations.CollaborationTests.api
    tasks = collaborations.CollaborationTests.tasks
    worker = collaborations.CollaborationTests.worker
    direct = collaborations.CollaborationTests.direct
    start_target = collaborations.CollaborationTests.start_target
    outgoing = collaborations.CollaborationTests.outgoing
    synced = collaborations.CollaborationTests.synced
    call_worker = collaborations.CollaborationTests.call_worker
    rejected = collaborations.CollaborationTests.rejected
    collaboration = collaborations.CollaborationTests.collaboration
    create_run = collaborations.CollaborationTests.create_run
    delegate = collaborations.CollaborationTests.delegate
    view = collaborations.CollaborationTests.view

    def dependent(self, run, parents):
        command = dict(operation='create', id=str(uuid.uuid4()), dispatch=str(uuid.uuid4()), node=self.target, title='Test after build', description='Check the prepared revision, then report evidence.', run_id=run['id'], depends_on=[parent['id'] for parent in parents])
        self.tasks(**command)
        return command

    def assert_unsent(self, task):
        item = self.outgoing(task['id'])
        self.assertEqual(item['current']['state'], 'queued')
        self.assertEqual(item['dependencies']['state'], 'waiting')
        self.assertEqual(item['attempts'], 0)
        self.assertIsNone(item['synced_at'])
        self.assertNotIn(task['id'], [item['task']['id'] for item in self.worker(operation='list')['incoming']])
        return item

    def finish(self, task, outcome='succeeded'):
        session = str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'finish', session=session, outcome=outcome, result='Explicit worker report: '+outcome)
        return self.synced(task, outcome)

    def test_all_prerequisites_must_succeed_failure_and_sync_do_not_bypass_waiting(self):
        run = self.create_run()
        first, second = self.delegate(run), self.delegate(run)
        child = self.dependent(run, [first, second])
        self.synced(first, 'available'); self.synced(second, 'available')
        self.assert_unsent(child)
        self.finish(first)
        self.call_worker(second, 'decline', reason='Missing compiler')
        self.synced(second, 'failed')
        self.tasks(operation='sync', id=child['id'], dispatch=child['dispatch'])
        self.assertEqual(self.view(run)['stats']['waiting'], 1)
        item = self.assert_unsent(child)
        self.assertEqual({parent['current']['state'] for parent in item['dependencies']['prerequisites']}, {'succeeded', 'failed'})
        dispatch = str(uuid.uuid4())
        self.tasks(operation='retry', id=second['id'], dispatch=second['dispatch'], new_dispatch=dispatch)
        second['dispatch'] = dispatch
        self.synced(second, 'available'); self.assert_unsent(child)
        self.finish(second)
        released = until(lambda: item if (item := self.outgoing(child['id']))['current']['state'] == 'available' else None)
        self.assertEqual(released['dependencies']['state'], 'released')
        self.assertEqual(released['current']['dispatch'], child['dispatch'])
        self.assertEqual(self.view(run)['stats']['waiting'], 0)
        received = next(item for item in self.worker(operation='list')['incoming'] if item['task']['id'] == child['id'])
        self.assertEqual(received['current']['state'], 'available', 'dependency release must not launch or claim a worker')
        self.assertNotIn('depends_on', received['task'], 'dependency coordination stays at the sender')

    def test_waiting_and_local_retry_survive_coordinator_restart(self):
        run = self.create_run()
        parent = self.delegate(run)
        child = self.dependent(run, [parent])
        self.synced(parent, 'available')
        self.assert_unsent(child)
        self.h.stop(self.h.web); self.h.start_web(); self.h.login()
        self.assert_unsent(child)
        cancel = dict(operation='cancel', id=child['id'], dispatch=child['dispatch'])
        self.tasks(**cancel); self.tasks(**cancel)
        cancelled = self.outgoing(child['id'])
        self.assertEqual(cancelled['current']['state'], 'cancelled')
        self.assertEqual(cancelled['dependencies']['state'], 'cancelled_before_dispatch')
        new_dispatch = str(uuid.uuid4())
        self.tasks(operation='retry', id=child['id'], dispatch=child['dispatch'], new_dispatch=new_dispatch)
        child['dispatch'] = new_dispatch
        self.assert_unsent(child)
        self.h.stop(self.h.web); self.h.start_web(); self.h.login()
        self.finish(parent)
        item = self.synced(child, 'available')
        self.assertEqual([receipt['state'] for receipt in item['history']], ['cancelled', 'available'])
        received = next(item for item in self.worker(operation='list')['incoming'] if item['task']['id'] == child['id'])
        self.assertEqual(received['current']['dispatch'], new_dispatch)

    def test_run_stop_locally_cancels_unsent_tasks_while_offline_and_prevents_late_release(self):
        run = self.create_run()
        parent = self.delegate(run); self.synced(parent, 'available')
        child = self.dependent(run, [parent]); self.assert_unsent(child)
        self.h.stop(self.process)
        self.collaboration('stop', id=run['id'], revision=1)
        self.assertEqual(self.outgoing(child['id'])['current']['state'], 'cancelled')
        self.assertEqual(self.view(run)['stats']['active'], 1)
        self.assertEqual(self.view(run)['stats']['waiting'], 0)
        self.rejected(dict(operation='collaboration', command=dict(operation='close', id=run['id'], revision=2, result='Not confirmed')))
        self.h.stop(self.h.web); self.h.start_web(); self.h.login()
        self.start_target(); self.synced(parent, 'cancelled')
        self.assertNotIn(child['id'], [item['task']['id'] for item in self.worker(operation='list')['incoming']])
        self.collaboration('close', id=run['id'], revision=2, result='Reviewed local cancellation and the target receipt.')
        self.collaboration('archive', id=run['id'], revision=3)
        self.assertEqual(self.tasks(operation='list')['outgoing'], [])
        saved = json.loads(self.path.read_text())
        self.assertIn(child['id'], saved['outgoing'], 'archive must retain the whole dependency graph until retention expires')

    def test_invalid_dependencies_preserve_existing_tasks_and_idempotent_create_binds_edges(self):
        run, other = self.create_run(), self.create_run()
        first, foreign = self.delegate(run), self.delegate(other)
        child = self.dependent(run, [first])
        self.tasks(**child)
        for change in [dict(depends_on=[]), dict(depends_on=[foreign['id']]), dict(depends_on=[first['id'], first['id']]), dict(depends_on=[str(uuid.uuid4())]), dict(depends_on=[child['id']]), dict(depends_on=[first['id']]*9), dict(run_id=None)]:
            self.rejected(dict(child, **change))
        self.assertEqual(self.outgoing(child['id'])['dependencies']['prerequisites'][0]['id'], first['id'])
        self.assertEqual(len(self.tasks(operation='list')['outgoing']), 3)
        self.assertNotIn(self.token, self.path.read_text())

    def test_credential_rotation_cannot_release_work_but_unsent_work_can_be_cancelled(self):
        run = self.create_run()
        parent = self.delegate(run); self.synced(parent, 'available')
        child = self.dependent(run, [parent]); self.assert_unsent(child)
        self.api(operation='connect', node=self.target, url=self.url, token='c'*64)
        self.assertTrue(self.outgoing(child['id'])['blocked'])
        self.tasks(operation='cancel', id=child['id'], dispatch=child['dispatch'])
        item = self.outgoing(child['id'])
        self.assertEqual(item['current']['state'], 'cancelled')
        self.assertEqual(item['attempts'], 0)
        self.assertEqual(item['dependencies']['state'], 'cancelled_before_dispatch')
        self.assertNotIn(child['id'], [item['task']['id'] for item in self.worker(operation='list')['incoming']])
