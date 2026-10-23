"""Exercise the shared Rust space store via authenticated Web and CLI, using no Docker daemon."""
import json
import os
from pathlib import Path
import random
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.parse import urlsplit
from unittest.mock import patch

from workspace_web_harness import Harness, CORE, NODE, WEB, free_port, sockets_allowed, until


@unittest.skipUnless(sys.platform == 'linux' and sockets_allowed() and all(path.exists() for path in (CORE, NODE, WEB)), 'Linux binaries and real sockets required')
class VirtualSpacesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='cranibox-space-engine-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        docker = self.root / 'docker'
        fixture = Path(__file__).with_name('space_docker_fixture.py')
        docker.write_text('#!' + sys.executable + '\n' + fixture.read_text())
        docker.chmod(0o700)
        # Each test has its own loopback IP, so container routing can use its real 9020 port.
        if Path('/.dockerenv').exists():
            address, port = f'127.118.{random.randrange(1,250)}.{random.randrange(1,250)}', 9020
            with socket.socket() as sock:
                sock.bind((address, port))
        else:
            address, port = '127.0.0.1', free_port()
        self.state_file = self.root / 'engine.json'
        self.write_state({'containers': {}, 'volumes': {}, 'events': [], 'pids': [], 'node': str(NODE.resolve()), 'address': address, 'port': port})
        environment = patch.dict(os.environ, {'PATH': str(self.root) + os.pathsep + os.environ['PATH'], 'CRANIBOX_SPACE_FIXTURE': str(self.root)}, clear=False)
        environment.start()
        self.addCleanup(environment.stop)
        self.addCleanup(self.stop_nodes)
        self.h = Harness()
        self.addCleanup(self.h.close)

    def write_state(self, state):
        self.state_file.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_file.read_text())

    def stop_nodes(self):
        for pid in self.state()['pids']:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    def api(self, operation='list', **fields):
        deadline = time.monotonic() + 15
        while True:
            status, value = self.h.post('/api/spaces', {'operation': operation, **fields})
            # A terminal phase can be visible before its background job releases
            # the operation lock. Only retry an explicitly rejected busy request;
            # other errors (including transport failures) must still fail the test.
            busy = status == 400 and value.get('error', '').startswith('another space operation is running;')
            if not busy or time.monotonic() >= deadline:
                break
            time.sleep(.04)
        self.assertEqual(status, 200, value)
        return value

    def wait(self, phase):
        return until(lambda: next((s for s in self.api()['spaces'] if s['phase'] == phase), None), seconds=15)

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.h.root / '.config/cranibox/settings.json'), *map(str, args)], text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_create_enroll_execute_stop_and_restore_keep_data_and_identity(self):
        check = self.api('preflight', provider='docker')
        self.assertTrue(check['ready'], check)
        self.h.stop(self.h.server)  # Enrollment and peer execution do not need a gateway.
        initial = self.api('create', name='Agent lab', provider='docker')['spaces'][0]
        space = self.wait('ready')
        self.assertEqual(space['id'], initial['id'])
        self.assertNotIn('password', space)
        self.assertNotIn('token', space)
        credentials = self.api('credentials', id=space['id'])
        self.assertEqual(len(credentials['password']), 64)
        self.assertNotIn(credentials['password'], json.dumps(self.api()))
        result = self.cli('peer', 'call', '--node', space['node'], 'exec', '--', sys.executable, '-c', 'print("space-execution-ok")')
        self.assertIn('space-execution-ok', json.dumps(result))
        data_file = self.root / space['volume'] / 'user-project.txt'
        data_file.write_text('persistent work')
        self.cli('space', 'stop', space['id'])
        self.assertFalse(any(self.wait('stopped')['ports'].values()))
        self.api('resume', id=space['id'])
        self.assertEqual(self.wait('ready')['node'], space['node'])
        self.cli('space', 'remove', space['id'])
        self.assertFalse(any(self.wait('archived')['ports'].values()))
        self.assertEqual(self.state()['containers'], {})
        self.assertIn(space['volume'], self.state()['volumes'])
        self.assertEqual(data_file.read_text(), 'persistent work')
        self.cli('space', 'resume', space['id'])
        restored = self.wait('ready')
        self.assertEqual(restored['node'], space['node'])
        self.assertEqual(self.api('credentials', id=space['id']), credentials)
        self.assertEqual(data_file.read_text(), 'persistent work')
        events = self.state()['events']
        self.assertEqual(sum(event[:2] == ['image', 'pull'] for event in events), 2)
        self.assertFalse(any(event[:2] == ['volume', 'rm'] for event in events))
        self.assertEqual(self.cli('space', 'list')['spaces'][0]['phase'], 'ready')

    def test_failure_retry_and_foreign_container_never_overwrite_user_resources(self):
        (self.root / 'fail-pull').touch()
        self.api('create', name='Offline first', provider='docker')
        failed = self.wait('failed')
        self.assertIn('image download failed', failed['error'])
        identity = failed['node']
        (self.root / 'fail-pull').unlink()
        self.api('resume', id=failed['id'])
        space = self.wait('ready')
        self.assertEqual(space['node'], identity)
        self.api('stop', id=space['id'])
        self.wait('stopped')
        state = self.state()
        state['containers'][space['container']]['Config']['Labels'] = {'owned': 'someone else'}
        self.write_state(state)
        event_index = len(state['events'])
        self.api('remove', id=space['id'])
        rejected = self.wait('failed')
        self.assertIn('another installation', rejected['error'])
        self.assertIn(space['container'], self.state()['containers'])
        self.assertFalse(any(event[:2] in (['container', 'stop'], ['container', 'rm']) for event in self.state()['events'][event_index:]))

    def test_validation_csrf_and_remote_engine_recovery(self):
        before = len(self.state()['events'])
        for command in [
            {'operation': 'create', 'name': 'bad', 'provider': 'docker', 'image': '--privileged'},
            {'operation': 'create', 'name': '', 'provider': 'docker'},
            {'operation': 'create', 'name': 'bad', 'provider': 'docker', 'mount': '/etc'},
            {'operation': 'remove', 'id': 'foreign-container'},
        ]:
            self.assertEqual(self.h.post('/api/spaces', command)[0], 400)
        self.assertEqual(len(self.state()['events']), before)
        self.assertEqual(self.h.http('POST', '/api/spaces', {'operation': 'list'}, headers={'Cookie': self.h.cookie, 'Content-Type': 'application/json'})[0], 401)
        self.assertEqual(self.h.http('POST', '/api/spaces', {'operation': 'list'}, headers={'Content-Type': 'application/json'})[0], 401)
        (self.root / 'remote-context').touch()
        check = self.api('preflight', provider='docker')
        self.assertFalse(check['ready'])
        self.assertIn('local Docker engine', check['detail'])
        self.assertEqual(self.state()['containers'], {})

    def test_setup_without_review_never_starts_an_installer(self):
        for fields in [{}, {'plan_id': '11111111-1111-4111-8111-111111111111'}]:
            status, value = self.h.post('/api/spaces', {'operation': 'install', 'action': 'linux_docker', **fields})
            self.assertEqual(status, 400, value)
            self.assertIn('plan', json.dumps(value).lower())
        result = subprocess.run([str(NODE), '--config', str(self.h.root / '.config/cranibox/settings.json'), 'space', 'install', 'linux-docker'], capture_output=True, text=True, timeout=20)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('--plan-id', result.stderr)
        self.assertEqual(self.state()['events'], [])
        self.assertIsNone(self.api()['setup'])

    def test_application_links_are_session_bound_one_use_and_require_owned_running_containers(self):
        self.api('create', name='Browser applications', provider='docker')
        space = self.wait('ready')
        status, value = self.h.post('/api/spaces/application', {'id': space['id'], 'application': 'desktop'})
        self.assertEqual(status, 200, value)
        link = urlsplit(value['url'])
        self.assertEqual(link.netloc, 'localhost:3002')
        self.assertNotIn(self.api('credentials', id=space['id'])['password'], value['url'])
        status, _ = self.h.http('GET', link.path + '?' + link.query, headers={'Host': 'localhost:3002'})
        self.assertEqual(status, 303)
        self.assertIn('HttpOnly', self.h.last_headers['set-cookie'])
        self.assertIn('Secure', self.h.last_headers['set-cookie'])
        self.assertEqual(self.h.last_headers['location'], link.path)
        self.assertEqual(self.h.http('GET', link.path + '?' + link.query, headers={'Host': 'localhost:3002'})[0], 403)
        status, value = self.h.post('/api/spaces/application', {'id': space['id'], 'application': 'desktop'})
        self.assertEqual(status, 200, value)
        stale = urlsplit(value['url'])
        state = self.state()
        original = state['containers'][space['container']]['Config']['Labels']
        state['containers'][space['container']]['Config']['Labels'] = {'foreign': 'container'}
        self.write_state(state)
        self.assertEqual(self.h.post('/api/spaces/application', {'id': space['id'], 'application': 'desktop'})[0], 400)
        state['containers'][space['container']]['Config']['Labels'] = original
        self.write_state(state)
        self.api('stop', id=space['id'])
        self.wait('stopped')
        self.assertEqual(self.h.http('GET', stale.path + '?' + stale.query, headers={'Host': 'localhost:3002'})[0], 403)
        self.assertEqual(self.h.post('/api/spaces/application', {'id': space['id'], 'application': 'desktop'})[0], 400)

    def test_incompatible_image_fails_before_creating_resources_and_can_retry(self):
        (self.root / 'old-image').touch()
        self.api('create', name='Needs current image', provider='docker')
        failed = self.wait('failed')
        self.assertIn('does not support Cranibox virtual-space enrollment', failed['error'])
        self.assertEqual(self.state()['containers'], {})
        self.assertEqual(self.state()['volumes'], {})
        credentials = self.api('credentials', id=failed['id'])
        event_index = len(self.state()['events'])
        self.assertEqual(self.h.post('/api/spaces', {'operation': 'resume', 'id': failed['id'], 'image': '--privileged'})[0], 400)
        self.assertEqual(len(self.state()['events']), event_index)
        self.assertEqual(self.api()['spaces'][0], failed)
        (self.root / 'old-image').unlink()
        self.api('resume', id=failed['id'], image='cranibox-current:test', pull_policy='never')
        recovered = self.wait('ready')
        for field in ['id', 'node', 'volume']:
            self.assertEqual(recovered[field], failed[field])
        self.assertEqual(recovered['image'], 'cranibox-current:test')
        self.assertEqual(recovered['pull_policy'], 'never')
        self.assertEqual(self.api('credentials', id=failed['id']), credentials)
        events = self.state()['events'][event_index:]
        self.assertTrue(any(event[:2] == ['image', 'inspect'] and event[-1] == 'cranibox-current:test' for event in events))
        self.assertFalse(any(event[:2] == ['image', 'pull'] for event in events))

    def test_image_change_requires_archive_and_retains_existing_data(self):
        self.api('create', name='Retained workspace', provider='docker')
        space = self.wait('ready')
        credentials = self.api('credentials', id=space['id'])
        data_file = self.root / space['volume'] / 'user-project.txt'
        data_file.write_text('retain across image change')
        for phase in ['ready', 'stopped']:
            if phase == 'stopped':
                self.cli('space', 'stop', space['id'])
                space = self.wait('stopped')
            event_index = len(self.state()['events'])
            for fields in [{'image': 'cranibox-current:test'}, {'pull_policy': 'never'}]:
                # Like api(): a finishing job may still hold the operation lock briefly.
                def attempt():
                    result = self.h.post('/api/spaces', {'operation': 'resume', 'id': space['id'], **fields})
                    return None if 'another space operation is running' in str(result[1]) else result
                status, error = until(attempt, seconds=15)
                self.assertEqual(status, 400, error)
                self.assertIn('Archive this space', str(error))
                self.assertEqual(self.api()['spaces'][0], space)
            self.assertTrue(all(event[:2] in (['container', 'ls'], ['container', 'inspect']) for event in self.state()['events'][event_index:]))
        self.cli('space', 'remove', space['id'])
        self.wait('archived')
        result = self.cli('space', 'resume', space['id'], '--image', 'cranibox-current:test', '--pull-policy', 'never')
        restored = result['spaces'][0]
        self.assertEqual(restored['phase'], 'ready')
        self.assertEqual(restored['image'], 'cranibox-current:test')
        self.assertEqual(restored['pull_policy'], 'never')
        for field in ['id', 'node', 'volume']:
            self.assertEqual(restored[field], space[field])
        self.assertEqual(self.api('credentials', id=space['id']), credentials)
        self.assertEqual(data_file.read_text(), 'retain across image change')
        self.assertFalse(any(event[:2] == ['volume', 'rm'] for event in self.state()['events']))

    def test_image_change_does_not_treat_engine_failure_as_absent_container(self):
        (self.root / 'fail-pull').touch()
        self.api('create', name='Engine interrupted', provider='docker')
        space = self.wait('failed')
        (self.root / 'fail-list').touch()
        status, error = self.h.post('/api/spaces', {'operation': 'resume', 'id': space['id'], 'image': 'cranibox-current:test', 'pull_policy': 'never'})
        self.assertEqual(status, 400, error)
        self.assertIn('engine unavailable during inspection', str(error))
        self.assertEqual(self.api()['spaces'][0], space)
        self.assertEqual(self.state()['containers'], {})
        self.assertEqual(self.state()['volumes'], {})

    def test_local_image_policy_stays_offline_through_archive_and_restore(self):
        (self.root / 'fail-pull').touch()
        self.api('create', name='Local build', provider='docker', image='cranibox-local:test', pull_policy='never')
        space = self.wait('ready')
        self.assertEqual(space['pull_policy'], 'never')
        self.api('remove', id=space['id'])
        self.wait('archived')
        self.api('resume', id=space['id'])
        self.assertEqual(self.wait('ready')['node'], space['node'])
        events = self.state()['events']
        self.assertFalse(any(event[:2] == ['image', 'pull'] for event in events))
        for event in events:
            if event[0] == 'run' or event[:2] == ['container', 'create']:
                self.assertIn('--pull=never', event)

    def test_missing_local_image_never_creates_resources_or_downloads_and_can_retry(self):
        (self.root / 'missing-image').touch()
        self.api('create', name='Not loaded yet', provider='docker', pull_policy='never')
        failed = self.wait('failed')
        self.assertIn('Build or load the image', failed['error'])
        self.assertEqual(self.state()['volumes'], {})
        self.assertEqual(self.state()['containers'], {})
        self.assertFalse(any(event[:2] == ['image', 'pull'] for event in self.state()['events']))
        (self.root / 'missing-image').unlink()
        self.api('resume', id=failed['id'])
        self.assertEqual(self.wait('ready')['node'], failed['node'])

    def test_pull_if_missing_downloads_once_and_reuses_image_for_restore(self):
        (self.root / 'missing-image').touch()
        self.api('create', name='Reuse cache', provider='docker', pull_policy='missing')
        space = self.wait('ready')
        self.api('remove', id=space['id'])
        self.wait('archived')
        (self.root / 'fail-pull').touch()
        self.api('resume', id=space['id'])
        self.assertEqual(self.wait('ready')['node'], space['node'])
        self.assertEqual(sum(event[:2] == ['image', 'pull'] for event in self.state()['events']), 1)


if __name__ == '__main__':
    unittest.main()
