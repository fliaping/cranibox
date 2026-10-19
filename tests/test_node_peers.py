"""Real direct-node HTTP/PTY integration, including operation without a Workspace."""
import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import uuid

from test_cranibox_terminal import Socket
from workspace_web_harness import CORE, NODE, Harness, free_port, sockets_allowed, until


class DirectSocket(Socket):
    def __init__(self, peer, session):
        self.socket = socket.create_connection(('127.0.0.1', peer.port), timeout=5)
        self.socket.settimeout(5)
        self.reader = self.socket.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        headers = [f'GET /api/v1/mesh/nodes/{peer.target}/terminal/ws?session={session} HTTP/1.1',
                   f'Host: 127.0.0.1:{peer.port}', 'Upgrade: websocket', 'Connection: Upgrade',
                   'Sec-WebSocket-Version: 13', f'Sec-WebSocket-Key: {key}',
                   f'Authorization: Bearer {peer.token}', f'X-Cranibox-Target: {peer.target}']
        self.socket.sendall(('\r\n'.join(headers) + '\r\n\r\n').encode())
        self.status = int(self.reader.readline().split()[1])
        response = {}
        while line := self.reader.readline():
            if line == b'\r\n':
                break
            name, value = line.decode().split(':', 1)
            response[name.lower()] = value.strip()
        if self.status == 101:
            expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            assert response['sec-websocket-accept'] == expected

    def command(self, line):
        self.send({'type': 'input', 'data': base64.b64encode((line + '\r').encode()).decode()})

    def expect(self, text):
        data = b''
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            opcode, payload = self.recv()
            if opcode == 8:
                raise AssertionError('Direct terminal closed: ' + repr(data))
            value = json.loads(payload)
            if value['type'] == 'output':
                data += base64.b64decode(value['data'])
                if text.encode() in data:
                    return data.decode(errors='replace')
        raise AssertionError('Missing output: ' + text)


@unittest.skipUnless(os.name == 'posix' and NODE.exists() and sockets_allowed(), 'build cranibox-node; real sockets and Unix PTY required')
class PeerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-peers-')
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.processes = []
        self.logs = []
        self.addCleanup(self.close)
        self.a, self.b = self.root / 'a/settings.json', self.root / 'b/settings.json'
        self.caller = self.cli(self.a, 'identity')['node_id']
        self.target = self.cli(self.b, 'identity')['node_id']
        self.token_path = self.root / 'b-to-a.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', self.caller, '--output-token', self.token_path, '--exec', '--terminals')
        self.token = self.token_path.read_text().strip()
        self.port = free_port()
        self.cli(self.b, 'listen', '--address', f'127.0.0.1:{self.port}')
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.port}', '--credential-file', self.token_path)

    def close(self):
        for process in reversed(self.processes):
            self.stop(process)
        for log in self.logs:
            log.close()

    def stop(self, process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    def cli(self, config, *args, input=None, check=True):
        result = subprocess.run([str(NODE), '--config', str(config), 'peer', *map(str, args)],
                                input=input, capture_output=True, text=True, timeout=15)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout) if input is None else [json.loads(line) for line in result.stdout.splitlines()]
        return result

    def start(self):
        log = (self.root / f'peer-{len(self.logs)}.log').open('wb')
        self.logs.append(log)
        process = subprocess.Popen([str(NODE), '--config', str(self.b), 'peer', 'serve'], stdout=log, stderr=log,
                                   env={**os.environ, 'HOME': str(self.root), 'CRANIBOX_HERDR_BIN': '/missing-herdr'})
        self.processes.append(process)
        until(lambda: self.http('GET', '/nodes')[0] == 200)
        return process

    def http(self, method, path, data=None, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        credentials = {'Authorization': 'Bearer ' + self.token, 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'}
        credentials.update(headers or {})
        try:
            connection.request(method, '/api/v1/mesh' + path, json.dumps(data) if data is not None else None, credentials)
            response = connection.getresponse()
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def manage(self, **request):
        status, value = self.http('POST', f'/nodes/{self.target}/terminal/sessions', request)
        self.assertEqual(status, 200, value)
        return value

    def execute(self, code, headers=None, timeout=10):
        return self.http('POST', f'/nodes/{self.target}/exec', {
            'program': sys.executable, 'args': ['-c', code], 'timeout_secs': timeout}, headers)

    def finished(self, job):
        return until(lambda: (value if value['outcome'] != 'running' else None)
                     if (value := self.http('GET', '/jobs/' + job)[1]) else None)

    def open(self, session):
        ws = DirectSocket(self, session)
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        return ws

    def test_stable_identity_private_credentials_cli_and_mcp_without_workspace(self):
        self.start()
        self.assertFalse(self.a.exists())
        self.assertFalse(self.b.exists())
        self.assertEqual(self.cli(self.a, 'identity')['node_id'], self.caller)
        self.assertEqual(self.token_path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.b.with_name('peers.json').stat().st_mode & 0o777, 0o600)
        self.assertNotIn(self.token, json.dumps(self.cli(self.b, 'identity')))
        result = self.cli(self.a, 'call', '--node', self.target, 'exec', sys.executable, '-c', 'print("DIRECT_OK")')
        self.assertEqual((result['stdout'], result['exit_code']), ('DIRECT_OK\n', 0))
        self.assertEqual(self.cli(self.a, 'call', '--node', self.target, 'job', result['id'])['stdout'], 'DIRECT_OK\n')
        messages = [
            {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18'}},
            {'jsonrpc':'2.0','id':2,'method':'tools/list'},
            {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'workspace_exec','arguments':{
                'node':self.target,'program':sys.executable,'args':['-c','print("MCP_DIRECT")']}}}]
        responses = self.cli(self.a, 'call', '--node', self.target, 'mcp', input=''.join(json.dumps(m) + '\n' for m in messages))
        self.assertEqual(len(responses[1]['result']['tools']), 5)
        job = json.loads(responses[2]['result']['content'][0]['text'])
        self.assertEqual(self.finished(job['id'])['stdout'], 'MCP_DIRECT\n')
        session = self.cli(self.a, 'call', '--node', self.target, 'sessions', 'create', '--name', 'Direct')['session']['id']
        ws = self.open(session)
        ws.command("export PEER_VALUE=kept; printf '%s%s\\n' READY _PTY")
        ws.expect('READY_PTY')
        ws.close()
        again = self.open(session)
        again.command('printf "%s\\n" "$PEER_VALUE"')
        again.expect('kept')
        self.cli(self.a, 'call', '--node', self.target, 'sessions', 'end', session)

    def test_auth_scopes_target_binding_and_job_ownership(self):
        self.start()
        self.assertEqual(self.http('GET', '/nodes', headers={'Authorization':'Bearer wrong'})[0], 401)
        self.assertEqual(self.http('GET', '/nodes', headers={'Origin':'https://evil.invalid'})[0], 403)
        self.assertEqual(self.http('GET', '/nodes', headers={'Sec-Fetch-Site':'cross-site'})[0], 403)
        marker = self.root / 'must-not-exist'
        self.assertEqual(self.execute(f'open({str(marker)!r}, "w").close()', {'X-Cranibox-Target':'wrong-target'})[0], 409)
        self.assertFalse(marker.exists())
        self.assertEqual(self.http('POST', '/nodes/another-node/exec', {'program':'true'})[0], 404)
        self.assertEqual(self.http('POST', f'/nodes/{self.target}/exec', {'program':'','timeout_secs':0})[0], 400)
        original = self.execute('print("private")')[1]['id']
        for name, flag, denied in [('exec-only','--exec','terminal/sessions'), ('terminal-only','--terminals','exec')]:
            token_file = self.root / f'{name}.token'
            self.cli(self.b, 'grant', '--allow-unverified', '--node', name, '--output-token', token_file, flag)
            headers = {'Authorization':'Bearer ' + token_file.read_text().strip()}
            payload = {'operation':'list'} if flag == '--exec' else {'program':'true'}
            self.assertEqual(self.http('POST', f'/nodes/{self.target}/{denied}', payload, headers)[0], 403)
            if flag == '--exec':
                self.assertEqual(self.http('GET', '/jobs/' + original, headers=headers)[0], 404)
        self.assertEqual(self.cli(self.b, 'listen', '--address', '0.0.0.0:9999', check=False).returncode, 1)
        self.assertNotEqual(self.cli(self.a, 'connect', '--node', self.target, '--url','http://10.0.0.1:9000', '--credential-file',self.token_path,check=False).returncode,0)
        same_file = self.root / 'new/peers.json'
        self.assertNotEqual(self.cli(same_file.with_name('settings.json'), 'grant', '--allow-unverified', '--node', self.caller, '--output-token', same_file, '--exec', check=False).returncode, 0)
        self.assertFalse(same_file.exists())

    def test_node_owned_projects_cli_mcp_and_listener_restart(self):
        self.cli(self.b, 'revoke', '--node', self.caller)
        token_file = self.root / 'catalog.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', self.caller, '--output-token', token_file, '--projects', '--terminals')
        self.token = token_file.read_text().strip()
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.port}', '--credential-file', token_file)
        process = self.start()
        project_id = str(uuid.uuid4())
        catalog = self.cli(self.a, 'call', '--node', self.target, 'projects', 'register', '--name', 'Portable project', '--path', self.root, '--project-id', project_id)
        self.assertEqual(catalog['owner'], self.target)
        self.assertEqual(catalog['projects'][0]['id'], project_id)
        replica = catalog['projects'][0]['replicas'][0]
        self.assertEqual(replica['node'], self.target)
        shell = self.manage(operation='create', name='Project shell', cwd=str(self.root))['session']
        bound = self.cli(self.a, 'call', '--node', self.target, 'projects', 'bind', replica['id'], shell['id'])
        binding = bound['projects'][0]['replicas'][0]['sessions'][0]
        self.assertEqual(binding['terminal_id'], shell['id'])
        self.assertEqual(self.cli(self.a, 'call', '--node', self.target, 'projects', 'bind', replica['id'], shell['id'])['revision'], bound['revision'])
        status, _ = self.http('POST', f'/nodes/{self.target}/projects', {'operation':'bind', 'replica_id':replica['id'], 'terminal_id':str(uuid.uuid4())})
        self.assertEqual(status, 400)
        request = {'jsonrpc':'2.0', 'id':1, 'method':'tools/call', 'params':{'name':'workspace_projects', 'arguments':{'command':{'operation':'list'}}}}
        response = self.cli(self.a, 'call', '--node', self.target, 'mcp', input=json.dumps(request) + '\n')[0]
        self.assertNotIn('isError', response['result'])
        self.assertEqual(json.loads(response['result']['content'][0]['text']), bound)
        request['params']['arguments']['command'] = {'operation':'register', 'name':'MCP copy', 'path':str(self.root / 'copy'), 'project_id':project_id}
        response = self.cli(self.a, 'call', '--node', self.target, 'mcp', input=json.dumps(request) + '\n')[0]
        copies = json.loads(response['result']['content'][0]['text'])['projects'][0]['replicas']
        self.assertEqual(len(copies), 2)
        self.assertTrue(all(r['node'] == self.target for r in copies))
        self.assertFalse(self.a.exists())
        self.assertFalse(self.b.exists())
        self.assertEqual(self.b.with_name('node-projects.json').stat().st_mode & 0o777, 0o600)
        self.stop(process)
        self.start()
        reopened = self.cli(self.a, 'call', '--node', self.target, 'projects', 'list')
        self.assertEqual(reopened['projects'][0]['replicas'][0]['sessions'][0], binding)
        # Node restart ends native shells, but never silently creates a replacement.
        self.assertFalse(self.manage(operation='list')['sessions'])
        self.cli(self.a, 'call', '--node', self.target, 'projects', 'unbind', replica['id'], shell['id'])
        kept = self.cli(self.a, 'call', '--node', self.target, 'projects', 'forget', replica['id'])
        self.assertEqual(len(kept['projects'][0]['replicas']), 1)
        self.assertTrue(self.root.exists())

    def test_projects_require_explicit_scope_and_bind_requires_terminal_access(self):
        self.start()
        endpoint = f'/nodes/{self.target}/projects'
        self.assertEqual(self.http('POST', endpoint, {'operation':'list'})[0], 403)
        token_file = self.root / 'projects-only.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', 'catalog-only', '--output-token', token_file, '--projects')
        headers = {'Authorization':'Bearer ' + token_file.read_text().strip()}
        status, catalog = self.http('POST', endpoint, {'operation':'register', 'name':'Catalog only', 'node':self.target, 'path':str(self.root)}, headers)
        self.assertEqual(status, 200, catalog)
        self.assertTrue(catalog['permissions']['projects'])
        self.assertFalse(catalog['permissions']['terminals'])
        replica = catalog['projects'][0]['replicas'][0]
        for node in [None, self.caller]:
            status, _ = self.http('POST', endpoint, {'operation':'register', 'name':'Wrong owner', 'node':node, 'path':str(self.root / 'other')}, headers)
            self.assertEqual(status, 400)
        status, _ = self.http('POST', endpoint, {'operation':'bind', 'replica_id':replica['id'], 'terminal_id':str(uuid.uuid4())}, headers)
        self.assertEqual(status, 403)
        self.assertEqual(self.http('POST', endpoint, {'operation':'list'}, {**headers, 'X-Cranibox-Target':self.caller})[0], 409)
        self.cli(self.b, 'revoke', '--node', 'catalog-only')
        self.assertEqual(self.http('POST', endpoint, {'operation':'list'}, headers)[0], 401)

    def test_idempotency_output_limits_capacity_timeout_and_cancel(self):
        self.start()
        marker = self.root / 'once'
        code = f'from pathlib import Path; p=Path({str(marker)!r}); p.write_text(p.read_text()+"x" if p.exists() else "x")'
        key = {'Idempotency-Key': str(uuid.uuid4())}
        first = self.execute(code, key)[1]['id']
        self.assertEqual(self.execute(code, key)[1]['id'], first)
        self.assertEqual(self.execute('print("changed")', key)[0], 409)
        self.finished(first)
        self.assertEqual(marker.read_text(), 'x')
        output = self.finished(self.execute('import os; os.write(1,b"a"*100000); os.write(2,b"b"*100000)')[1]['id'])
        self.assertEqual(len(base64.b64decode(output['stdout_base64'])) + len(base64.b64decode(output['stderr_base64'])), 65536)
        self.assertTrue(output['truncated'])
        timeout = self.finished(self.execute('import time; time.sleep(60)', timeout=1)[1]['id'])
        self.assertEqual(timeout['outcome'], 'timed_out')
        jobs = [self.execute('import time; time.sleep(60)')[1]['id'] for _ in range(4)]
        self.assertEqual(self.execute('print("no slot")')[0], 429)
        for job in jobs:
            self.assertEqual(self.http('POST', '/jobs/' + job + '/cancel', {})[0], 200)
        for job in jobs:
            self.assertEqual(self.finished(job)['outcome'], 'cancelled')
        self.assertEqual(self.finished(self.execute('print("slot freed")')[1]['id'])['stdout'], 'slot freed\n')

    def test_live_revoke_cancels_command_detaches_shell_and_rotation_hides_old_jobs(self):
        self.start()
        session = self.manage(operation='create', name='Survives revoke')['session']['id']
        ws = self.open(session)
        ws.command("export REVOKED_SHELL=survived; printf '%s%s\\n' SHELL _READY")
        ws.expect('SHELL_READY')
        pidfile = self.root / 'child.pid'
        job = self.execute(f'import os,time; open({str(pidfile)!r},"w").write(str(os.getpid())); time.sleep(60)')[1]['id']
        until(pidfile.exists)
        pid = int(pidfile.read_text())
        self.cli(self.b, 'revoke', '--node', self.caller)
        self.assertEqual(self.http('GET', '/nodes')[0], 401)
        while ws.recv()[0] != 8:
            pass
        ws.close()
        until(lambda: not Path(f'/proc/{pid}/stat').exists() or Path(f'/proc/{pid}/stat').read_text().split(') ')[1][0] == 'Z')
        replacement = self.root / 'replacement.token'
        self.cli(self.b, 'grant', '--allow-unverified', '--node', self.caller, '--output-token', replacement, '--exec', '--terminals')
        self.token = replacement.read_text().strip()
        self.assertEqual(self.http('GET', '/jobs/' + job)[0], 404)
        again = self.open(session)
        again.command('printf "%s\\n" "$REVOKED_SHELL"')
        again.expect('survived')
        self.manage(operation='end', id=session)

    def test_restart_retains_identity_and_grants_but_not_processes_and_corruption_fails_closed(self):
        process = self.start()
        session = self.manage(operation='create', name='Ephemeral')['session']['id']
        self.stop(process)
        self.start()
        self.assertEqual(self.cli(self.b, 'identity')['node_id'], self.target)
        self.assertNotIn(session, [s['id'] for s in self.manage(operation='list')['sessions']])
        path = self.b.with_name('peers.json')
        original = path.read_bytes()
        path.write_bytes(b'broken')
        self.assertEqual(self.http('GET', '/nodes')[0], 503)
        self.assertNotEqual(self.cli(self.b, 'identity', check=False).returncode, 0)
        self.assertEqual(path.read_bytes(), b'broken')
        path.write_bytes(original)
        self.assertEqual(self.http('GET', '/nodes')[0], 200)

    def test_legacy_grants_migrate_on_start_and_identity_loss_fails_closed(self):
        path = self.b.with_name('peers.json')
        key_path = path.with_name('node-identity.json')
        legacy = json.loads(path.read_text())
        legacy['version'] = 1
        legacy.pop('identity')
        permissions = legacy['grants'][self.caller]['permissions']
        legacy['grants'][self.caller] = {'token': self.token, 'permissions': permissions}
        path.write_text(json.dumps(legacy))
        key_path.unlink()
        process = self.start()
        saved = path.read_bytes()
        configuration = json.loads(saved)
        key = key_path.read_bytes()
        self.assertEqual(configuration['version'], 2)
        self.assertEqual(configuration['node_id'], self.target)
        self.assertEqual(configuration['grants'][self.caller]['credential'],
                         hashlib.sha256(self.token.encode()).hexdigest())
        self.assertNotIn(self.token, saved.decode())
        self.assertEqual(json.loads(key)['minimum_config_version'], 2)
        self.assertEqual(self.finished(self.execute('print("MIGRATED")')[1]['id'])['stdout'], 'MIGRATED\n')

        key_path.unlink()
        self.assertEqual(self.http('GET', '/nodes')[0], 503)
        self.assertNotEqual(self.cli(self.b, 'identity', check=False).returncode, 0)
        self.assertFalse(key_path.exists(), 'a missing key must not be silently replaced')
        self.assertEqual(path.read_bytes(), saved)
        key_path.write_bytes(key)
        key_path.chmod(0o600)
        self.assertEqual(self.http('GET', '/nodes')[0], 200)

        downgraded = json.dumps(legacy).encode()
        path.write_bytes(downgraded)
        self.assertEqual(self.http('GET', '/nodes')[0], 503)
        self.assertNotEqual(self.cli(self.b, 'identity', check=False).returncode, 0)
        self.assertEqual(path.read_bytes(), downgraded)
        self.assertEqual(key_path.read_bytes(), key)
        path.write_bytes(saved)
        self.stop(process)
        self.start()
        view = self.cli(self.b, 'identity')
        self.assertEqual(view['identity']['public'], configuration['identity'])
        self.assertEqual(self.finished(self.execute('print("RESTORED")')[1]['id'])['stdout'], 'RESTORED\n')
        self.assertEqual(key_path.read_bytes(), key)

    @unittest.skipUnless(CORE.exists(), 'build cranibox server')
    def test_gateway_and_direct_share_shell_and_capacity_and_work_when_server_stops(self):
        h = Harness(unified=True, node_env={'CRANIBOX_HERDR_BIN':'/missing-herdr'})
        self.addCleanup(h.close)
        h.stop(h.node)
        h.node = h.start(NODE, '--config', self.b, '--id', 'test-node', '--server', f'ws://127.0.0.1:{h.gateway_port}', '--token-file', h.root / 'node.token')
        until(lambda: self.http('GET', '/nodes')[0] == 200)
        until(lambda: h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        status, value = h.post('/api/terminal/sessions', {'node':'test-node','operation':'create','name':'Shared engine'})
        self.assertEqual(status, 200, value)
        session = value['session']['id']
        status, ticket = h.post('/api/terminal/ticket', {'node':'test-node','session_id':session,'cols':80,'rows':24})
        self.assertEqual(status, 200, ticket)
        ws = Socket(h, ticket['ticket'])
        self.addCleanup(ws.close)
        self.assertEqual(ws.status, 101)
        ws.command("export SERVER_INDEPENDENT=yes; printf '%s%s\\n' GATEWAY _READY")
        ws.expect('GATEWAY_READY')
        ws.close()
        time.sleep(.15)
        jobs = [self.execute('import time; time.sleep(60)')[1]['id'] for _ in range(4)]
        refused = h.remote_job('print("no extra capacity")')
        until(lambda: (value if value['outcome'] != 'running' else None) if (value := h.gateway('/api/v1/jobs/' + refused)) else None)
        self.assertEqual(h.gateway('/api/v1/jobs/' + refused)['outcome'], 'failed')
        for job in jobs:
            self.http('POST', '/jobs/' + job + '/cancel', {})
            self.finished(job)
        h.stop(h.server)
        direct = self.open(session)
        direct.command('printf "%s\\n" "$SERVER_INDEPENDENT"')
        direct.expect('yes')
        self.assertEqual(self.finished(self.execute('print("SERVER_DOWN")')[1]['id'])['stdout'], 'SERVER_DOWN\n')
        self.manage(operation='create', name='Created while offline')
        direct.close()
        time.sleep(.1)
        h.start_web()
        h.login()
        until(lambda: h.gateway('/api/v1/nodes')['nodes'][0]['online'])
        status, catalog = h.post('/api/terminal/sessions', {'node':'test-node','operation':'list'})
        self.assertEqual(status, 200, catalog)
        self.assertIn(session, [s['id'] for s in catalog['sessions']])
        self.assertEqual(len(catalog['sessions']), 2)


if __name__ == '__main__':
    unittest.main()
