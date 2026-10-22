"""Portable, reviewed Agent context over real direct-node HTTP and local CLI paths.

The isolated codex fixture validates process ownership and stdin delivery, not model access.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

import test_web_direct_peers as direct
from workspace_web_harness import CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class ProjectHandoffsTests(unittest.TestCase):
    setUp = direct.WebDirectPeersTests.setUp
    api = direct.WebDirectPeersTests.api
    cli = direct.WebDirectPeersTests.cli
    copy_fixture = direct.WebDirectPeersTests.copy_fixture

    def context(self, **changes):
        return {'goal': 'Finish the portable project; preserve user changes.', 'progress': 'Implemented 文件复制. Source Agent remains on its device.', 'next_steps': 'Inspect the files and finish the remaining work.', 'verification': 'Fixture build passed; model credentials were not tested.', **changes}

    def install_fixture(self):
        bin_dir = self.h.root / 'agent-bin'
        bin_dir.mkdir(exist_ok=True)
        cli = bin_dir / 'codex'
        cli.write_text('#!' + sys.executable + '\n' + '''import os,pathlib,sys,time
prompt=sys.stdin.read()
root=pathlib.Path.cwd()
count=root/'fixture-launch-count'
count.write_text(count.read_text()+'x' if count.exists() else 'x')
(root/'fixture-prompt.txt').write_text(prompt)
print('NODE='+os.environ['CRANIBOX_NODE_ID'],flush=True)
print('CWD='+str(root),flush=True)
print('文'*30000,flush=True)
print('HANDOFF_DONE',flush=True)
if 'fixture-wait' in prompt: time.sleep(60)
''')
        cli.chmod(0o700)
        return str(bin_dir)

    def continuation(self, job, operation='copy_agent_prepare', **changes):
        command = {'operation': operation, 'id': job['id']}
        if operation in ('copy_agent_prepare', 'copy_agent_start'):
            command.update(adapter='codex', access='workspace_write')
        return self.api(**{**command, **changes})

    def test_direct_copy_carries_checked_context_and_recovers_one_run_after_web_restart(self):
        code, project, config, cli, target, _, _, process, root = self.copy_fixture()
        cli('copy-root', '--path', root)
        self.h.stop(process)
        self.h.node_env['PATH'] = self.install_fixture()
        self.h.start(NODE, '--config', config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': target})[0] == 200)
        self.h.stop(self.h.server)
        job = self.api(operation='copy_prepare', source=self.target, target=target, replica_id=project['replicas'][0]['id'], handoff=self.context())
        self.assertEqual(job['summary']['handoff'], self.context())
        path = job['summary']['handoff_path']
        self.assertEqual(path, '.cranibox/handoffs/' + job['id'] + '.md')
        entry = next(f for f in job['summary']['files'] if f['path'] == path)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'copy_agent_prepare', 'id': job['id'], 'adapter': 'codex', 'access': 'read_only'})[0], 400)
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'completed', job)
        destination = Path(job['result']['path'])
        body = (destination / path).read_bytes()
        self.assertEqual(entry['sha256'], hashlib.sha256(body).hexdigest())
        self.assertEqual(entry['size'], len(body))
        self.assertIn(self.context()['goal'], body.decode())
        self.assertFalse((code / '.cranibox').exists())
        self.assertFalse((destination / '.env').exists())
        review = self.continuation(job)
        self.assertTrue(review['available'])
        self.assertEqual(review['execution_id'], job['id'])
        self.assertEqual(review['launch']['cwd'], str(destination))
        self.assertIn(path, review['launch']['stdin'])
        self.assertNotIn(self.context()['goal'], json.dumps(review['launch']['args']))
        self.assertFalse((destination / 'fixture-launch-count').exists())
        status, denied = self.h.post('/api/peers', {'operation': 'copy_agent_start', 'id': job['id'], 'adapter': 'codex', 'access': 'workspace_write', 'digest': '0' * 64})
        self.assertEqual(status, 400, denied)
        self.assertFalse((destination / 'fixture-launch-count').exists())
        self.continuation(job, 'copy_agent_start', digest=review['digest'])
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.continuation(job, 'copy_agent_start', digest=review['digest'])
        first = until(lambda: value if (value := self.continuation(job, 'copy_agent_status'))['execution']['state'] == 'succeeded' else None)
        self.assertEqual(first['owner'], target)
        self.assertEqual(first['execution']['id'], job['id'])
        self.assertEqual((destination / 'fixture-launch-count').read_text(), 'x')
        self.assertEqual((destination / 'fixture-prompt.txt').read_text(), review['launch']['stdin'])
        second = self.continuation(job, 'copy_agent_status', stdout_offset=first['stdout']['next_offset'], stderr_offset=first['stderr']['next_offset'])
        output = first['stdout']['text'] + second['stdout']['text']
        self.assertIn('NODE=' + target, output)
        self.assertIn('HANDOFF_DONE', output)
        self.assertEqual(output.count('文'), 30000)
        self.assertNotIn('\ufffd', output)
        self.assertEqual(self.api(operation='copy_list')['copies'][0]['summary']['handoff'], self.context())
        cli('revoke', '--node', self.caller)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'copy_agent_status', 'id': job['id']})[0], 400)

    def test_local_destination_launches_without_gateway_and_stops_only_its_continuation(self):
        _, project, _, _, _, _, _, _, _ = self.copy_fixture()
        root = self.h.root / 'local-handoff'
        root.mkdir()
        self.api(operation='copy_root', path=str(root))
        self.h.stop(self.h.server)
        self.h.stop(self.h.web)
        self.h.web_env['PATH'] = self.install_fixture()
        self.h.start_web()
        self.h.login()
        job = self.api(operation='copy_prepare', source=self.target, target=self.caller, replica_id=project['replicas'][0]['id'], handoff=self.context(next_steps='fixture-wait'))
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        review = self.continuation(job)
        self.continuation(job, 'copy_agent_start', digest=review['digest'])
        until(lambda: (Path(job['result']['path']) / 'fixture-launch-count').exists())
        self.continuation(job, 'copy_agent_stop')
        stopped = until(lambda: value if (value := self.continuation(job, 'copy_agent_status'))['execution']['state'] == 'cancelled' else None)
        self.assertEqual(stopped['owner'], self.caller)
        self.assertEqual((Path(job['result']['path']) / 'fixture-launch-count').read_text(), 'x')
        missing = self.continuation(job, adapter='claude_code', access='read_only')
        self.assertFalse(missing['available'])

    def test_validation_and_plain_copies_never_create_an_implicit_handoff_or_run(self):
        code, project, _, cli, target, _, _, _, root = self.copy_fixture()
        cli('copy-root', '--path', root)
        command = {'operation': 'copy_prepare', 'source': self.target, 'target': target, 'replica_id': project['replicas'][0]['id']}
        for context in [self.context(goal=' '), self.context(goal='文' * 683), self.context(progress='x' * 4097), self.context(verification='bad\x00'), {**self.context(), 'unknown': 'ignored?'}]:
            status, value = self.h.post('/api/peers', {**command, 'handoff': context})
            self.assertEqual(status, 400, value)
        self.assertEqual(self.api(operation='copy_list')['copies'], [])
        (code / '.CRANIBOX').mkdir()
        status, denied = self.h.post('/api/peers', {**command, 'handoff': self.context()})
        self.assertEqual(status, 400, denied)
        job = self.api(**command)
        self.assertNotIn('handoff', job['summary'])
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'completed')
        for operation in ['copy_agent_prepare', 'copy_agent_start', 'copy_agent_status', 'copy_agent_stop']:
            attempt = {'operation': operation, 'id': job['id']}
            if operation in ('copy_agent_prepare', 'copy_agent_start'):
                attempt.update(adapter='codex', access='read_only')
            if operation == 'copy_agent_start':
                attempt['digest'] = '0' * 64
            self.assertEqual(self.h.post('/api/peers', attempt)[0], 400)
        self.assertEqual(self.api(operation='executions', node=target, command={'operation': 'list'})['executions'], [])

    def test_native_cli_accepts_bounded_handoff_file_and_inert_review(self):
        _, project, _, receiver, target, _, port, _, root = self.copy_fixture()
        credential = self.h.root / 'native-handoff.token'
        receiver('grant', '--allow-unverified', '--node', self.target, '--output-token', credential, '--exec', '--projects')
        receiver('copy-root', '--path', root)
        self.cli('connect', '--node', target, '--url', f'http://127.0.0.1:{port}', '--credential-file', credential)
        context = self.h.root / 'handoff.json'
        context.write_text(json.dumps(self.context()))
        job = self.cli('copy', 'prepare', '--source', self.target, '--target', target, '--replica-id', project['replicas'][0]['id'], '--handoff-file', context)
        job = self.cli('copy', 'commit', job['id'], '--digest', job['digest'])
        review = self.cli('copy', 'agent', '--command', json.dumps({'operation': 'copy_agent_prepare', 'id': job['id'], 'adapter': 'codex', 'access': 'read_only'}))
        self.assertEqual(review['execution_id'], job['id'])
        self.assertEqual(review['launch']['cwd'], job['result']['path'])
        self.assertEqual(self.api(operation='executions', node=target, command={'operation': 'list'})['executions'], [])
        context.write_text(' ' * 65537)
        result = subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', 'copy', 'prepare', '--source', self.target, '--target', target, '--replica-id', project['replicas'][0]['id'], '--handoff-file', str(context)], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('64 KiB', result.stderr)
