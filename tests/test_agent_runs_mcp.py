"""Opt-in execution MCP over real stdio and independent nodes, without a relay."""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
import uuid

import test_web_direct_peers as direct
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class AgentRunsMcpTests(unittest.TestCase):
    cli = direct.WebDirectPeersTests.cli
    api = direct.WebDirectPeersTests.api

    def setUp(self):
        direct.WebDirectPeersTests.setUp(self)
        self.h.stop(self.h.server)
        self.caller_config = self.path.with_name('settings.json')

    def rpc(self, requests, config=None, endpoint='runs'):
        result = subprocess.run([str(NODE), '--config', str(config or self.caller_config), 'peer', endpoint, 'mcp'],
                                input=''.join(json.dumps(request) + '\n' for request in requests),
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(self.token, result.stdout + result.stderr)
        return [json.loads(line) for line in result.stdout.splitlines()]

    def tool(self, name='device_agent_runs', arguments=None, error=False, **options):
        result = self.rpc([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                            'params': {'name': name, 'arguments': arguments if arguments is not None else {}}}], **options)[0]['result']
        if error:
            self.assertTrue(result.get('isError'), result)
            return result['content'][0]['text']
        self.assertFalse(result.get('isError'), result)
        return json.loads(result['content'][0]['text'])

    def action(self, operation, **values):
        return self.tool(arguments={'operation': operation, 'node': self.target, **values})

    def launch(self, code, **values):
        return {'id': str(uuid.uuid4()), 'launch': {'name': 'MCP remote worker', 'harness': 'fixture-cli',
                'program': sys.executable, 'args': ['-u', '-c', code], 'cwd': str(self.h.root), **values}}

    def detail(self, run, **offsets):
        return self.action('get', id=run['id'], **offsets)

    def state(self, run, states):
        return until(lambda: detail if (detail := self.detail(run))['execution']['state'] in states else None)

    def test_discovery_and_execution_are_opt_in_and_require_an_explicit_device(self):
        responses = self.rpc([
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
        ])
        self.assertEqual(responses[0]['result']['serverInfo']['name'], 'cranibox-device-runs')
        tools = responses[1]['result']['tools']
        self.assertEqual([item['name'] for item in tools], ['device_nodes', 'device_agent_runs'])
        for variant in tools[1]['inputSchema']['oneOf']:
            self.assertIn('node', variant['required'])
            self.assertFalse(variant['additionalProperties'])
        devices = self.tool('device_nodes')
        self.assertEqual(devices, {'local_node': self.caller, 'devices': [
            {'id': self.caller, 'local': True}, {'id': self.target, 'local': False}]})
        self.assertFalse(self.caller_config.exists(), 'MCP must not require Workspace settings')
        self.assertIn('explicit node ID', self.tool(arguments={'operation': 'list'}, error=True))
        for node in [None, '', 'local', 'unknown-device', 123, '../other-device']:
            self.tool(arguments={'operation': 'list', 'node': node}, error=True)
        self.tool('device_nodes', {'node': self.target}, error=True)
        self.tool('workspace_exec', {}, error=True)
        self.tool('device_messages', {'operation': 'list'}, error=True)
        self.tool(arguments={'operation': 'list', 'node': self.target, 'extra': 'rejected'}, error=True)
        message_tools = self.rpc([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list'}], endpoint='messages')[0]
        self.assertEqual([item['name'] for item in message_tools['result']['tools']], ['device_messages', 'device_tasks', 'device_agent_sessions'])
        self.tool(arguments={'operation': 'list', 'node': self.target}, endpoint='messages', error=True)
        self.assertEqual(self.action('list')['executions'], [])

    def test_remote_worker_survives_mcp_and_web_exit_and_same_id_never_relaunches(self):
        run = self.launch("import pathlib,time,os; p=pathlib.Path('mcp-count'); p.write_text(p.read_text()+'x' if p.exists() else 'x'); print(os.environ['CRANIBOX_NODE_ID'],flush=True); time.sleep(1); print('worker finished 中文')")
        self.action('start', **run)
        self.action('start', **run)
        self.state(run, {'running', 'succeeded'})
        self.h.stop(self.h.web)
        detail = self.state(run, {'succeeded'})
        self.assertEqual(detail['owner'], self.target)
        self.assertEqual(detail['execution']['requester'], self.caller)
        self.assertIn(self.target, detail['stdout']['text'])
        self.assertIn('worker finished 中文', detail['stdout']['text'])
        self.assertEqual((self.h.root / 'mcp-count').read_text(), 'x')
        self.action('start', **run)
        changed = {**run, 'launch': {**run['launch'], 'args': ['-c', 'raise SystemExit(7)']}}
        self.tool(arguments={'operation': 'start', 'node': self.target, **changed}, error=True)
        self.assertEqual((self.h.root / 'mcp-count').read_text(), 'x')
        self.assertEqual(len(self.action('list')['executions']), 1)

    def test_prepare_agent_is_inert_then_explicit_start_delivers_stdin_once(self):
        tools = self.h.root / 'tools'
        tools.mkdir()
        executable = tools / 'codex'
        executable.write_text('#!' + sys.executable + '\n' +
                              "import json,pathlib,sys\nwith pathlib.Path('agent-count').open('a') as file: file.write('x')\nprint(json.dumps({'args':sys.argv[1:],'prompt':sys.stdin.read()},ensure_ascii=False))\n")
        executable.chmod(0o700)
        self.h.stop(self.target_process)
        self.h.node_env['PATH'] = str(tools) + os.pathsep + os.environ.get('PATH', '')
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': self.target})[0] == 200)
        prompt = '检查项目\n$(touch must-not-execute)\n--dangerously-skip-permissions'
        prepared = self.action('prepare_agent', agent={'name': 'Review worker', 'adapter': 'codex',
                               'access': 'read_only', 'prompt': prompt, 'cwd': str(self.h.root)})
        self.assertTrue(prepared['available'])
        self.assertEqual(Path(prepared['launch']['program']), executable)
        self.assertFalse((self.h.root / 'agent-count').exists())
        self.assertEqual(self.action('list')['executions'], [])
        run = {'id': str(uuid.uuid4()), 'launch': prepared['launch']}
        self.action('start', **run)
        self.action('start', **run)
        detail = self.state(run, {'succeeded'})
        output = json.loads(detail['stdout']['text'])
        self.assertEqual(output['prompt'], prompt)
        self.assertNotIn(prompt, output['args'])
        self.assertIn('read-only', output['args'])
        self.assertEqual((self.h.root / 'agent-count').read_text(), 'x')
        self.assertFalse((self.h.root / 'must-not-execute').exists())

    def test_large_escaped_input_output_pagination_and_process_failure(self):
        run = self.launch("import sys; value=sys.stdin.read(); assert value=='\\x01'*16384; print('z'*140000); print('stderr 中文',file=sys.stderr); raise SystemExit(9)", stdin='\x01' * 16384)
        self.action('start', **run)
        detail = self.state(run, {'failed'})
        self.assertEqual(detail['execution']['exit_code'], 9)
        text, offset = '', 0
        while True:
            output = self.detail(run, stdout_offset=offset)['stdout']
            if output['next_offset'] == offset:
                break
            text += output['text']
            offset = output['next_offset']
        self.assertEqual(text, 'z' * 140000 + '\n')
        self.assertIn('stderr 中文', detail['stderr']['text'])
        self.tool(arguments={'operation': 'get', 'node': self.target, 'id': run['id'], 'stdout_offset': -1}, error=True)
        invalid = self.launch('pass', stdin='x' * 16385)
        self.tool(arguments={'operation': 'start', 'node': self.target, **invalid}, error=True)
        result = subprocess.run([str(NODE), '--config', str(self.caller_config), 'peer', 'runs', 'mcp'],
                                input='x' * (128 * 1024 + 1), capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('exceeds 128 KiB', result.stderr)
        self.assertEqual(len(self.action('list')['executions']), 1)

    def test_stop_archive_and_local_target_are_explicit(self):
        run = self.launch("import time; print('running',flush=True); time.sleep(90)")
        self.action('start', **run)
        self.state(run, {'running'})
        self.tool(arguments={'operation': 'archive', 'node': self.target, 'id': run['id']}, error=True)
        self.action('stop', id=run['id'])
        self.state(run, {'cancelled'})
        self.action('archive', id=run['id'])
        self.action('start', **run)
        self.assertEqual(self.action('list')['executions'], [])
        local = self.launch("print('local explicitly selected')")
        self.action('start', node=self.caller, **local)
        detail = until(lambda: value if (value := self.action('get', node=self.caller, id=local['id']))['execution']['state'] == 'succeeded' else None)
        self.assertEqual(detail['owner'], self.caller)
        self.assertIn('local explicitly selected', detail['stdout']['text'])
        self.assertEqual(self.action('list')['executions'], [])

    def test_messages_grant_cannot_execute_and_rotated_grant_cannot_read_old_runs(self):
        run = self.launch("import time; print('private result',flush=True); time.sleep(90)")
        self.action('start', **run)
        self.state(run, {'running'})
        self.cli('revoke', '--node', self.caller)
        token_path = self.h.root / 'messages-only.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_path, '--messages', '--inbound', 'accept')
        self.api(operation='connect', node=self.target, url=self.url, token=token_path.read_text().strip())
        self.tool(arguments={'operation': 'list', 'node': self.target}, error=True)
        self.tool(arguments={'operation': 'start', 'node': self.target, **self.launch('pass')}, error=True)
        stopped = until(lambda: value if (value := self.cli('runs', '--command', json.dumps({'operation': 'get', 'id': run['id']})))['execution']['state'] == 'cancelled' else None)
        self.assertIn('revoked', stopped['execution']['error'])
        self.cli('revoke', '--node', self.caller)
        token_path = self.h.root / 'rotated-exec.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_path, '--exec')
        self.api(operation='connect', node=self.target, url=self.url, token=token_path.read_text().strip())
        self.assertEqual(self.action('list')['executions'], [])
        self.tool(arguments={'operation': 'get', 'node': self.target, 'id': run['id']}, error=True)


if __name__ == '__main__':
    unittest.main()
