"""Node-owned programs: observers, deduplication, bounded logs and recovery."""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from pathlib import Path

from workspace_web_harness import CORE, NODE, RUNTIME, WEB, Harness, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class AgentExecutionsTests(unittest.TestCase):
    def setUp(self):
        tools = tempfile.TemporaryDirectory(prefix='cranibox-agent-tools-')
        self.addCleanup(tools.cleanup)
        self.tools = Path(tools.name)
        for name in ['codex', 'claude']:
            path = self.tools / name
            path.write_text('#!' + sys.executable + '\n' + '''import json,sys,pathlib
with pathlib.Path('agent-invocations').open('a') as count: count.write('x')
print('fixture progress',file=sys.stderr,flush=True)
print(json.dumps({'args':sys.argv[1:],'prompt':sys.stdin.read()},ensure_ascii=False),flush=True)
''')
            path.chmod(0o700)
        self.h = Harness(web_env={'PATH': str(self.tools) + os.pathsep + os.environ.get('PATH', '')})
        self.addCleanup(self.h.close)
        self.owner = self.api(operation='identity')['node_id']
        self.ledger = self.h.root / '.config/cranibox-node/agent-executions.json'
        self.logs = self.ledger.with_name('agent-logs')

    def api(self, **command):
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, value)
        return value

    def action(self, operation, **values):
        return self.api(operation='executions', command={'operation': operation, **values})

    def launch(self, code, **values):
        return {'id': str(uuid.uuid4()), 'launch': {'name': 'Background verification', 'harness': 'fixture-cli', 'program': sys.executable, 'args': ['-u', '-c', code], 'cwd': str(self.h.root.resolve()), **values}}

    def get(self, run, **offsets):
        return self.action('get', id=run['id'], **offsets)

    def state(self, run, expected):
        return until(lambda: detail if (detail := self.get(run))['execution']['state'] in expected else None)

    def test_agent_preparation_and_execution_use_stdin_without_implicit_launches(self):
        task = '审查项目\n$(touch unexpected-shell-file)\n--dangerously-skip-permissions'
        for adapter, access in [('codex', 'read_only'), ('codex', 'workspace_write'), ('claude_code', 'read_only'), ('claude_code', 'edit_files')]:
            before = (self.h.root / 'agent-invocations').read_text() if (self.h.root / 'agent-invocations').exists() else ''
            prepared = self.action('prepare_agent', agent={'name': 'Project review', 'adapter': adapter, 'access': access, 'prompt': task, 'cwd': str(self.h.root)})
            self.assertTrue(prepared['available'])
            self.assertEqual(Path(prepared['launch']['program']).parent, self.tools)
            self.assertEqual((self.h.root / 'agent-invocations').read_text() if (self.h.root / 'agent-invocations').exists() else '', before)
            run = {'id': str(uuid.uuid4()), 'launch': prepared['launch']}
            self.action('start', **run)
            self.action('start', **run)
            detail = self.state(run, {'succeeded'})
            result = json.loads(detail['stdout']['text'])
            self.assertEqual(result['prompt'], task)
            self.assertNotIn(task, ' '.join(result['args']))
            self.assertEqual((self.h.root / 'agent-invocations').read_text(), before + 'x')
            self.assertFalse((self.h.root / 'unexpected-shell-file').exists())
            self.assertIn('fixture progress', detail['stderr']['text'])
        self.assertEqual(len(self.action('list')['executions']), 4)
        for command in [
            {'operation': 'prepare_agent', 'agent': {'name': 'Invalid', 'adapter': 'codex', 'access': 'edit_files', 'prompt': task, 'cwd': str(self.h.root)}},
            {'operation': 'prepare_agent', 'agent': {'name': 'Invalid', 'adapter': 'codex', 'access': 'read_only', 'prompt': task, 'cwd': str(self.h.root / 'missing')}},
        ]:
            self.assertEqual(self.h.post('/api/peers', {'operation': 'executions', 'command': command})[0], 400)
        self.assertEqual(len(self.action('list')['executions']), 4)

    def test_agent_input_limit_applies_after_json_decoding(self):
        for prompt in ['x' * 16384, '界' * 5461 + 'x', 'X' + '\x01' * 16383]:
            self.assertEqual(len(prompt.encode()), 16384)
            prepared = self.action('prepare_agent', agent={'name': 'Input limit', 'adapter': 'codex', 'access': 'read_only', 'prompt': prompt, 'cwd': str(self.h.root)})
            self.assertEqual(prepared['launch']['stdin'], prompt)
        self.assertEqual(self.action('list')['executions'], [])
        oversized = self.launch('pass', stdin='x' * (128 * 1024))
        self.assertEqual(self.h.post('/api/peers', {'operation': 'executions', 'command': {'operation': 'start', **oversized}})[0], 413)

    def test_stdin_and_output_are_drained_concurrently_and_input_is_bounded(self):
        task = '开始\n' + 'x' * 15000 + '\n结束'
        run = self.launch("import sys,hashlib; sys.stdout.write('p'*140000); sys.stdout.flush(); data=sys.stdin.buffer.read(); print('\\n'+hashlib.sha256(data).hexdigest())", stdin=task)
        self.action('start', **run)
        self.state(run, {'succeeded'})
        text, offset = '', 0
        while True:
            page = self.get(run, stdout_offset=offset)['stdout']
            if page['next_offset'] == offset:
                break
            text += page['text']
            offset = page['next_offset']
        import hashlib
        self.assertEqual(text, 'p' * 140000 + '\n' + hashlib.sha256(task.encode()).hexdigest() + '\n')
        invalid = self.launch('pass', stdin='x' * 16385)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'executions', 'command': {'operation': 'start', **invalid}})[0], 400)

    def test_observer_disconnect_deduplication_registry_and_restart_history(self):
        marker = self.h.root / 'count'
        run = self.launch("import os,pathlib,time,json,sys; p=pathlib.Path('count'); p.write_text(p.read_text()+'x' if p.exists() else 'x'); print(json.dumps({k:os.environ[k] for k in ['CRANIBOX_EXECUTION_ID','CRANIBOX_AGENT_SESSION_ID','CRANIBOX_NODE_ID']}),flush=True); time.sleep(.7); print('finished'); print('diagnostic',file=sys.stderr)")
        self.action('start', **run)
        self.action('start', **run)
        running = self.state(run, {'running', 'succeeded'})
        session = running['execution']['session_id']
        # All HTTP observers have disconnected; the process must finish on its own.
        until(lambda: marker.exists())
        time.sleep(.9)
        detail = self.state(run, {'succeeded'})
        environment = json.loads(detail['stdout']['text'].splitlines()[0])
        self.assertEqual(environment, {'CRANIBOX_EXECUTION_ID': run['id'], 'CRANIBOX_AGENT_SESSION_ID': session, 'CRANIBOX_NODE_ID': self.owner})
        self.assertIn('finished', detail['stdout']['text'])
        self.assertIn('diagnostic', detail['stderr']['text'])
        self.assertEqual(detail['execution']['exit_code'], 0)
        self.assertEqual(marker.read_text(), 'x')
        reports = self.api(operation='agent_sessions', node=None, command={'operation': 'list'})
        until(lambda: next(item for item in self.api(operation='agent_sessions', node=None, command={'operation': 'list'})['sessions'] if item['id'] == session)['status'] == 'ended')
        self.assertTrue(any(item['id'] == session for item in reports['sessions']))
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.action('start', **run)
        self.assertEqual(self.get(run)['execution']['state'], 'succeeded')
        self.assertEqual(marker.read_text(), 'x')
        changed = {**run, 'launch': {**run['launch'], 'args': ['-c', 'raise SystemExit(17)']}}
        self.assertEqual(self.h.post('/api/peers', {'operation': 'executions', 'command': {'operation': 'start', **changed}})[0], 400)
        self.assertNotIn('launch', self.action('list')['executions'][0])

    def test_stop_reaps_descendants_and_archive_does_not_relaunch(self):
        ticks = self.h.root / 'child-ticks'
        child = "import pathlib,time; p=pathlib.Path('child-ticks');\nwhile True:\n p.open('a').write('x'); time.sleep(.04)"
        run = self.launch("import subprocess,sys,time; subprocess.Popen([sys.executable,'-c'," + repr(child) + "]); print('parent ready',flush=True); time.sleep(120)")
        self.action('start', **run)
        self.state(run, {'running'})
        until(lambda: ticks.exists() and ticks.stat().st_size > 3)
        self.action('stop', id=run['id'])
        self.state(run, {'cancelled'})
        size = ticks.stat().st_size
        time.sleep(.25)
        self.assertEqual(ticks.stat().st_size, size, 'child continued after process-tree cancellation')
        self.action('archive', id=run['id'])
        self.assertEqual(self.action('start', **run)['executions'], [])
        self.assertFalse(list(self.logs.glob(run['id'] + '*')))

    def test_log_pages_limits_utf8_and_private_files(self):
        run = self.launch("import os,time; os.write(1,b'a'*65535); os.write(1,'中'.encode()[:1]); time.sleep(.3); os.write(1,'中'.encode()[1:]); os.write(1,b'z'*1100000); os.write(2,b'e'*1100000); raise SystemExit(7)")
        self.action('start', **run)
        detail = self.state(run, {'failed'})
        self.assertEqual(detail['execution']['exit_code'], 7)
        self.assertTrue(detail['execution']['stdout_truncated'])
        self.assertTrue(detail['execution']['stderr_truncated'])
        text, offset = '', 0
        while True:
            page = self.get(run, stdout_offset=offset)['stdout']
            if page['next_offset'] == offset:
                break
            text += page['text']
            offset = page['next_offset']
        self.assertEqual(offset, 1024 * 1024)
        self.assertIn('a中z', text)
        self.assertNotIn('\ufffd', text)
        for path in [self.ledger, *self.logs.iterdir()]:
            self.assertEqual(path.stat().st_mode & 0o077, 0)
        self.assertEqual(self.logs.stat().st_mode & 0o077, 0)
        self.assertTrue(all(path.stat().st_size <= 1024 * 1024 for path in self.logs.iterdir()))
        status, _ = self.h.post('/api/peers', {'operation': 'executions', 'command': {'operation': 'get', 'id': run['id'], 'stdout_offset': 1024 * 1024 + 1}})
        self.assertEqual(status, 400)

    def test_graceful_shutdown_interrupts_and_restart_never_reexecutes(self):
        marker = self.h.root / 'started'
        run = self.launch("import pathlib,time; p=pathlib.Path('started'); p.write_text(p.read_text()+'x' if p.exists() else 'x'); time.sleep(120)")
        self.action('start', **run)
        self.state(run, {'running'})
        until(lambda: marker.exists())
        self.h.stop(self.h.web)
        data = json.loads(self.ledger.read_text())
        self.assertEqual(data['records'][run['id']]['state'], 'interrupted')
        self.h.start_web()
        self.h.login()
        self.action('start', **run)
        self.assertEqual(self.get(run)['execution']['state'], 'interrupted')
        time.sleep(.35)
        self.assertEqual(marker.read_text(), 'x')

    def test_crash_recovery_does_not_trust_pid_or_repeat_ambiguous_launch(self):
        run = self.launch("import pathlib,time; pathlib.Path('crash-started').write_text('once'); time.sleep(120)")
        self.action('start', **run)
        pid = self.state(run, {'running'})['execution']['pid']
        self.addCleanup(self.kill_group, pid)
        until(lambda: (self.h.root / 'crash-started').exists())
        self.h.stop(self.h.web, force=True)
        # Corrupt a saved PID to point at this test. Recovery must clear it, not signal it.
        data = json.loads(self.ledger.read_text())
        data['records'][run['id']]['pid'] = os.getpid()
        self.ledger.write_text(json.dumps(data))
        self.h.start_web()
        self.h.login()
        detail = self.get(run)
        self.assertEqual(detail['execution']['state'], 'interrupted')
        self.assertIsNone(detail['execution']['pid'])
        self.assertIn('unknown', detail['execution']['error'])
        self.action('start', **run)
        self.assertEqual(self.get(run)['execution']['state'], 'interrupted')
        self.kill_group(pid)

    @staticmethod
    def kill_group(pid):
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def test_standalone_node_cli_and_failed_launch_without_gateway(self):
        config = self.h.root / 'independent/settings.json'

        def cli(*args, success=True):
            result = subprocess.run([str(NODE), '--config', str(config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode == 0, success, result.stderr)
            return json.loads(result.stdout) if success else result

        owner = cli('identity')['node_id']
        # A launch queued while the node is stopped is not executed later without review.
        queued = self.launch("import pathlib; pathlib.Path('unexpected').touch()")
        cli('runs', '--command', json.dumps({'operation': 'start', **queued}))
        self.h.stop(self.h.server)
        service = self.h.start(NODE, '--config', config, 'peer', 'serve')
        self.addCleanup(lambda: self.h.stop(service))

        def read(run):
            return cli('runs', '--command', json.dumps({'operation': 'get', 'id': run['id']}))

        until(lambda: read(queued)['execution']['state'] == 'interrupted')
        self.assertFalse((self.h.root / 'unexpected').exists())
        run = self.launch("import os; print(os.environ['CRANIBOX_NODE_ID'])")
        cli('runs', '--command', json.dumps({'operation': 'start', **run}))
        until(lambda: read(run)['execution']['state'] == 'succeeded')
        self.assertEqual(read(run)['stdout']['text'].strip(), owner)
        missing = self.launch('', program=str(self.h.root / 'no-such-program'))
        cli('runs', '--command', json.dumps({'operation': 'start', **missing}))
        until(lambda: read(missing)['execution']['state'] == 'failed')
        self.assertIsNone(read(missing)['execution']['pid'])
        self.assertNotIn('no-such-program', read(missing)['execution']['error'])

    def test_launch_requires_browser_authorization_and_typed_local_operation(self):
        run = self.launch("raise SystemExit(0)")
        status, _ = self.h.http('POST', '/api/peers', {'operation': 'executions', 'command': {'operation': 'start', **run}}, headers={'Content-Type': 'application/json'})
        self.assertEqual(status, 401)
        status, _ = self.h.http('POST', '/api/peers', {'operation': 'executions', 'command': {'operation': 'start', **run}}, headers={'Cookie': self.h.cookie, 'Content-Type': 'application/json'})
        self.assertEqual(status, 401)
        status, _ = self.h.post('/api/peers', {'operation': 'executions', 'node': 'another-device', 'command': {'operation': 'start', **run}})
        self.assertEqual(status, 400)
        self.assertEqual(self.action('list')['executions'], [])


if __name__ == '__main__':
    unittest.main()
