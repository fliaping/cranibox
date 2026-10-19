"""Explicit read-only grants and source-bound activity over independent Rust nodes."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import subprocess
import threading
import time
import unittest
import uuid

import test_device_tasks as task_tests
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class ActivityTests(unittest.TestCase):
    setUp = task_tests.DeviceTasksTests.setUp
    cli = task_tests.DeviceTasksTests.cli
    api = task_tests.DeviceTasksTests.api
    tasks = task_tests.DeviceTasksTests.tasks
    worker = task_tests.DeviceTasksTests.worker
    direct = task_tests.DeviceTasksTests.direct
    start_target = task_tests.DeviceTasksTests.start_target
    create = task_tests.DeviceTasksTests.create
    synced = task_tests.DeviceTasksTests.synced
    outgoing = task_tests.DeviceTasksTests.outgoing
    call_worker = task_tests.DeviceTasksTests.call_worker

    def enable(self, *permissions):
        path = self.h.root / (str(uuid.uuid4()) + '.token')
        self.cli('revoke', '--node', self.caller)
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', path, *permissions)
        self.token = path.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)

    def snapshot(self):
        return self.api(operation='activity', node=self.target)

    def envelope(self, sender=None, text='Private third-party message 中文'):
        now = int(time.time())
        identifier = str(uuid.uuid4())
        return dict(id=identifier, thread=identifier, in_reply_to=None, sender=sender or self.caller,
                    recipient=self.target, text=text,
                    created_at=now, expires_at=now+604800)

    def test_messages_alone_cannot_read_others_and_activity_alone_cannot_deliver(self):
        path = f'/api/v1/mesh/nodes/{self.target}/activity'
        self.assertEqual(self.direct('GET', path)[0], 403)
        other_settings = self.h.root/'third/settings.json'
        result = subprocess.run([str(NODE), '--config', str(other_settings), 'peer', 'identity'], capture_output=True, text=True, check=True, timeout=10)
        other = json.loads(result.stdout)['node_id']
        token_path = self.h.root/'third.token'
        self.cli('grant', '--allow-unverified', '--node', other, '--messages', '--output-token', token_path)
        other_token = token_path.read_text().strip()
        message = self.envelope(other)
        self.assertEqual(self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/messages', message, {'Authorization':'Bearer '+other_token})[0], 200)
        self.enable('--activity')
        snapshot = self.snapshot()
        self.assertEqual(snapshot['owner'], self.target)
        self.assertEqual(snapshot['messages'][0]['message'], message)
        self.assertEqual(self.api(operation='probe', node=self.target)['nodes'][0]['activity'], True)
        self.assertEqual(self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/messages', self.envelope())[0], 403)
        self.assertEqual(self.direct('POST', path, {})[0], 405)
        self.assertNotIn(other_token, json.dumps(snapshot))
        self.assertEqual(self.api(operation='activity')['messages'], [])
        self.assertEqual(self.cli('activity')['messages'], snapshot['messages'])
        caller_settings = self.h.root/'.config/cranibox-node/settings.json'
        read = subprocess.run([str(NODE), '--config', str(caller_settings), 'peer', 'activity', '--node', self.target], capture_output=True, text=True, check=True, timeout=10)
        self.assertEqual(json.loads(read.stdout)['digest'], snapshot['digest'])

    def test_tasks_questions_results_run_revision_and_archiving_are_source_observations(self):
        self.enable('--messages', '--activity')
        task, _ = self.create()
        self.synced(task, 'available')
        session, question = str(uuid.uuid4()), str(uuid.uuid4())
        self.call_worker(task, 'claim', session=session)
        self.call_worker(task, 'progress', session=session, text='Checked two files.')
        self.call_worker(task, 'ask', session=session, question=question, text='Which release?')
        self.synced(task, 'running')
        self.tasks(operation='answer', id=task['id'], dispatch=task['dispatch'], question=question, text='Release 42')
        self.tasks(operation='sync', id=task['id'], dispatch=task['dispatch'])
        until(lambda: self.worker(operation='list')['incoming'][0]['current']['questions'][0]['answer'] == 'Release 42')
        before = self.config.with_name('device-tasks.json').read_bytes()
        snapshot = self.snapshot()
        row = snapshot['tasks'][0]
        self.assertEqual(row['direction'], 'incoming')
        self.assertEqual(row['current']['progress'], 'Checked two files.')
        self.assertEqual(row['current']['questions'][0]['answer'], 'Release 42')
        self.assertEqual(self.snapshot()['digest'], snapshot['digest'])
        self.assertEqual(self.config.with_name('device-tasks.json').read_bytes(), before)
        self.call_worker(task, 'finish', session=session, outcome='succeeded', result='Reviewed release 42.')
        finished = self.snapshot()
        self.assertNotEqual(finished['digest'], snapshot['digest'])
        self.assertEqual(finished['tasks'][0]['current']['result'], 'Reviewed release 42.')
        self.worker(operation='archive', id=task['id'], dispatch=task['dispatch'])
        self.assertEqual(self.snapshot()['tasks'], [])
        route_token = self.h.root/'run-route.token'
        route_token.write_text('c'*64+'\n')
        route_token.chmod(0o600)
        self.cli('connect', '--node', self.caller, '--url', self.url, '--credential-file', route_token)
        run = str(uuid.uuid4())
        def collaboration(operation, **values):
            return self.worker(operation='collaboration', command=dict(operation=operation, id=run, **values))
        collaboration('create', name='Review', goal='Check evidence', members=[self.caller])
        self.assertEqual(self.snapshot()['runs'][0]['revision'], 1)
        collaboration('close', revision=1, result='No remaining work.')
        closed = self.snapshot()
        self.assertEqual(closed['runs'][0]['state'], 'closed')
        self.assertEqual(closed['runs'][0]['revision'], 2)
        self.assertEqual(closed['runs'][0]['result'], 'No remaining work.')
        serialized = json.dumps(closed)
        for private in ('bindings', 'token_file', 'credential', 'next_attempt', str(self.h.root)):
            self.assertNotIn(private, serialized)
        collaboration('archive', revision=2)
        self.assertEqual(self.snapshot()['runs'], [])

    def test_target_revocation_and_corrupt_source_fail_closed_without_rewriting_ledgers(self):
        self.enable('--activity')
        path = f'/api/v1/mesh/nodes/{self.target}/activity'
        self.assertEqual(self.direct('GET', path, headers={'X-Cranibox-Target':'wrong-node'})[0], 409)
        ledger = self.config.with_name('device-tasks.json')
        secret = b'{broken-private-ledger-secret'
        ledger.write_bytes(secret)
        status, response = self.direct('GET', path)
        self.assertEqual(status, 503)
        self.assertNotIn('private-ledger-secret', json.dumps(response))
        self.assertEqual(ledger.read_bytes(), secret)
        ledger.unlink()
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.direct('GET', path)[0], 401)

    def fake_source(self, response, arrived=None, release=None):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path.endswith('/activity'):
                    if arrived: arrived.set()
                    if release and not release.wait(5): return
                    body = response[0]
                else:
                    body = {'nodes':[{'id':response[0]['owner']}]}
                encoded = json.dumps(body).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
            def log_message(self, *args): pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        if release: self.addCleanup(release.set)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)

    def test_remote_source_owner_unknown_fields_and_modified_digest_are_rejected(self):
        self.enable('--messages', '--activity')
        envelope = self.envelope()
        self.assertEqual(self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/messages', envelope)[0], 200)
        expected = self.snapshot()
        response = [expected]
        self.fake_source(response)
        self.assertEqual(self.snapshot(), expected)
        modified = copy.deepcopy(expected)
        modified['messages'][0]['message']['text'] = 'Different content'
        duplicate = copy.deepcopy(expected)
        duplicate['messages'] *= 2
        for invalid in ({**expected, 'owner':'another-source'}, {**expected, 'credential':'secret'}, modified, duplicate):
            response[0] = invalid
            status, error = self.h.post('/api/peers', {'operation':'activity', 'node':self.target})
            self.assertEqual(status, 400, error)
            self.assertNotIn(envelope['text'], json.dumps(error))

    def test_route_removal_and_same_file_credential_rotation_reject_late_reads(self):
        self.enable('--activity')
        response = [self.snapshot()]
        arrived, release = threading.Event(), threading.Event()
        self.fake_source(response, arrived, release)
        for change in ('token', 'forget'):
            with self.subTest(change=change):
                arrived.clear(); release.clear()
                result = []
                worker = threading.Thread(target=lambda: result.append(self.h.post('/api/peers', {'operation':'activity', 'node':self.target})), daemon=True)
                worker.start()
                self.assertTrue(arrived.wait(4))
                if change == 'token':
                    peers = json.loads((self.h.root/'.config/cranibox-node/peers.json').read_text())
                    with open(peers['routes'][self.target]['token_file'], 'w') as token:
                        token.write('b'*64+'\n')
                else:
                    self.api(operation='forget', node=self.target)
                release.set(); worker.join(5)
                self.assertFalse(worker.is_alive())
                self.assertEqual(result[0][0], 400, result)

    def test_bounded_read_supports_records_larger_than_the_generic_api_limit(self):
        self.enable('--messages', '--activity')
        for _ in range(132):
            self.assertEqual(self.direct('POST', f'/api/v1/mesh/nodes/{self.target}/messages', self.envelope(text='x'*8192))[0], 200)
        snapshot = self.snapshot()
        self.assertEqual(len(snapshot['messages']), 132)
        self.assertGreater(len(json.dumps(snapshot)), 1024*1024)
