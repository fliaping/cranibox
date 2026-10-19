"""Real direct HTTP transport through the browser API, without a Workspace relay."""
import json
import http.client
import http.server
import os
import socket
from pathlib import Path
import subprocess
import sys
import threading
import time
import unittest

from workspace_web_harness import Harness, CORE, NODE, RUNTIME, WEB, free_port, sockets_allowed, until
from test_cranibox_terminal import Socket


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class WebDirectPeersTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.path = self.h.root / '.config/cranibox-node/peers.json'
        self.assertEqual(self.api(operation='list')['routes'], [])
        self.assertTrue(self.path.exists())
        self.caller = self.api(operation='identity')['node_id']
        self.target_config = self.h.root / 'target/settings.json'
        self.target = self.cli('identity')['node_id']
        self.token_path = self.h.root / 'issued.token'
        self.cli('grant', '--node', self.caller, '--output-token', self.token_path, '--exec')
        self.token = self.token_path.read_text().strip()
        self.port = free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.cli('listen', '--address', f'127.0.0.1:{self.port}')
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': self.target})[0] == 200)

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def api(self, **command):
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, value)
        self.assertNotIn(self.token if hasattr(self, 'token') else 'never-a-real-token', json.dumps(value))
        return value

    def execute(self, code, **extra):
        return self.api(operation='exec', node=self.target, request={'program': sys.executable, 'args': ['-c', code], 'timeout_secs': 15, **extra})

    def test_commands_and_saved_routes_work_with_gateway_offline(self):
        self.h.stop(self.h.server)
        peer = self.api(operation='probe', node=self.target)['nodes'][0]
        self.assertTrue(peer['exec'])
        self.assertFalse(peer['terminal_sessions'])
        job = self.execute('import sys; print("direct output"); print("direct error", file=sys.stderr)')
        value = until(lambda: (v if (v := self.api(operation='job', node=self.target, id=job['id']))['outcome'] != 'running' else None))
        self.assertEqual(value['exit_code'], 0)
        self.assertIn('direct output', value['stdout'])
        self.assertIn('direct error', value['stderr'])
        self.assertNotIn('stdout_base64', value)
        self.h.login()
        self.assertEqual(self.api(operation='job', node=self.target, id=job['id'])['exit_code'], 0)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.api(operation='list')['routes'][0]['node'], self.target)
        self.assertEqual(self.api(operation='job', node=self.target, id=job['id'])['exit_code'], 0)

    def test_cancel_revoke_and_wrong_target_fail_closed(self):
        job = self.execute('import time; print("running",flush=True); time.sleep(30)')
        self.api(operation='cancel', node=self.target, id=job['id'])
        value = until(lambda: (v if (v := self.api(operation='job', node=self.target, id=job['id']))['outcome'] != 'running' else None))
        self.assertEqual(value['outcome'], 'cancelled')
        self.api(operation='connect', node='wrong-node', url=self.url, token=self.token)
        marker = self.h.root / 'must-not-exist'
        status, value = self.h.post('/api/peers', {'operation':'exec', 'node':'wrong-node', 'request':{'program':sys.executable, 'args':['-c', f'open({str(marker)!r},"w").close()'], 'timeout_secs':5}})
        self.assertEqual(status, 400, value)
        self.assertIn('409', value['error'])
        self.assertFalse(marker.exists())
        self.cli('revoke', '--node', self.caller)
        status, _ = self.h.post('/api/peers', {'operation':'probe','node':self.target})
        self.assertEqual(status, 400)

    def test_direct_terminal_preserves_shell_without_gateway(self):
        self.h.stop(self.h.server)
        self.cli('revoke', '--node', self.caller)
        token_path = self.h.root / 'terminal.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_path, '--terminals')
        self.token = token_path.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)

        def manage(**command):
            status, value = self.h.post('/api/terminal/sessions', {'node': self.target, 'direct': True, **command})
            self.assertEqual(status, 200, value)
            return value

        shell = manage(operation='create', name='Direct browser Shell', cwd=str(self.h.root))['session']

        def attach():
            status, value = self.h.post('/api/terminal/ticket', {'node': self.target, 'direct': True, 'session_id': shell['id'], 'cols': 80, 'rows': 24})
            self.assertEqual(status, 200, value)
            socket = Socket(self.h, value['ticket'])
            self.addCleanup(socket.close)
            self.assertEqual(socket.status, 101)
            socket.expect('ready')
            return socket

        first = attach()
        first.command("export CRANIBOX_SAVED=preserved; printf '%s%s\\n' DIRECT _READY")
        first.expect('DIRECT_READY')
        first.command('echo $$ > direct-shell.pid')
        pid_file = self.h.root / 'direct-shell.pid'
        until(pid_file.exists)
        pid = pid_file.read_text().strip()
        first.close()
        time.sleep(.25)
        manage(operation='rename', id=shell['id'], name='Preserved Shell')
        self.assertEqual(manage(operation='list')['sessions'][0]['name'], 'Preserved Shell')

        second = attach()
        second.command("printf '%s:%s\\n' SAME_SHELL \"$CRANIBOX_SAVED:$$\"")
        second.expect('SAME_SHELL:preserved:' + pid)
        second.send({'type': 'resize', 'cols': 101, 'rows': 31})
        second.command('stty size > direct-size')
        size_file = self.h.root / 'direct-size'
        until(lambda: size_file.exists() and size_file.read_text().strip() == '31 101')
        self.cli('revoke', '--node', self.caller)
        until(lambda: second.recv()[0] == 8)
        self.assertTrue(Path('/proc/' + pid).exists())
        status, _ = self.h.post('/api/terminal/sessions', {'node': self.target, 'direct': True, 'operation': 'list'})
        self.assertEqual(status, 400)

    def test_direct_terminal_requires_explicit_scope_and_node(self):
        status, value = self.h.post('/api/terminal/sessions', {'node': self.target, 'direct': True, 'operation': 'list'})
        self.assertEqual(status, 400, value)
        self.assertIn('403', value['error'])
        for endpoint, command in [
            ('/api/terminal/sessions', {'operation': 'list'}),
            ('/api/terminal/ticket', {'session_id': '11111111-1111-4111-8111-111111111111', 'cols': 80, 'rows': 24}),
        ]:
            status, _ = self.h.post(endpoint, {'direct': True, **command})
            self.assertEqual(status, 400)

    def test_project_catalog_and_shell_reopen_without_gateway_or_web_process(self):
        self.h.stop(self.h.server)
        status, value = self.h.post('/api/peers', {'operation':'projects', 'node':self.target, 'command':{'operation':'list'}})
        self.assertEqual(status, 400, value)
        self.assertIn('403', value['error'])
        self.cli('revoke', '--node', self.caller)
        token_path = self.h.root / 'projects.token'
        self.cli('grant', '--node', self.caller, '--output-token', token_path, '--projects', '--terminals')
        self.token = token_path.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)

        def projects(**command):
            return self.api(operation='projects', node=self.target, command=command)

        def sessions(**command):
            return self.api(operation='sessions', node=self.target, command=command)

        def attach(shell):
            status, value = self.h.post('/api/terminal/ticket', {'node':self.target, 'direct':True, 'session_id':shell['id'], 'cols':80, 'rows':24})
            self.assertEqual(status, 200, value)
            ws = Socket(self.h, value['ticket'])
            self.addCleanup(ws.close)
            self.assertEqual(ws.status, 101)
            ws.expect('ready')
            return ws

        catalog = projects(operation='register', name='Direct project', node=self.target, path=str(self.h.root))
        self.assertEqual(catalog['owner'], self.target)
        replica = catalog['projects'][0]['replicas'][0]
        shell = sessions(operation='create', name='Catalog shell', cwd=replica['path'])['session']
        bound = projects(operation='bind', replica_id=replica['id'], terminal_id=shell['id'])
        self.assertEqual(bound['projects'][0]['replicas'][0]['sessions'][0]['terminal_id'], shell['id'])
        first = attach(shell)
        first.command("export DIRECT_PROJECT=preserved; printf '%s%s\\n' CATALOG _READY")
        first.expect('CATALOG_READY')
        first.close()
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(projects(operation='list'), bound)
        self.assertEqual(sessions(operation='list')['sessions'][0]['id'], shell['id'])
        second = attach(shell)
        second.command("printf '%s%s\\n' CATALOG_ \"$DIRECT_PROJECT\"")
        second.expect('CATALOG_preserved')
        second.close()
        projects(operation='unbind', replica_id=replica['id'], terminal_id=shell['id'])
        self.assertEqual(projects(operation='forget', replica_id=replica['id'])['projects'], [])
        self.assertEqual(sessions(operation='list')['sessions'][0]['status'], 'running')
        sessions(operation='end', id=shell['id'])

    def test_credentials_csrf_and_corrupt_config(self):
        view = self.api(operation='list')
        self.assertNotIn('token_file', json.dumps(view))
        route_file = Path(json.loads(self.path.read_text())['routes'][self.target]['token_file'])
        self.assertEqual(route_file.stat().st_mode & 0o777, 0o600)
        status, _ = self.h.http('POST', '/api/peers', {'operation':'list'}, headers={'Content-Type':'application/json'})
        self.assertEqual(status, 401)
        status, _ = self.h.http('POST', '/api/peers', {'operation':'list'}, headers={**self.h.headers(), 'X-Workspace-CSRF':'wrong'})
        self.assertEqual(status, 401)
        status, _ = self.h.http('POST', '/api/peers', {'operation':'list'}, headers={**self.h.headers(), 'Origin':'https://other.example'})
        self.assertEqual(status, 403)
        status, _ = self.h.post('/api/peers', {'operation':'connect','node':self.target,'url':'http://127.0.0.1:1','token':''})
        self.assertEqual(status, 400)
        self.assertEqual(self.api(operation='list')['routes'], view['routes'])
        self.api(operation='forget', node=self.target)
        self.assertFalse(route_file.exists())
        target_peers = json.loads(self.target_config.with_name('peers.json').read_text())
        self.assertIn(self.caller, target_peers['grants'])
        self.path.write_text('broken')
        status, _ = self.h.post('/api/peers', {'operation':'identity'})
        self.assertEqual(status, 400)
        self.assertEqual(self.path.read_text(), 'broken')

    def project_grant(self):
        self.cli('revoke', '--node', self.caller)
        credential = self.h.root / 'project-cache.token'
        self.cli('grant', '--node', self.caller, '--output-token', credential, '--projects', '--terminals')
        self.token = credential.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)

    def copy_fixture(self):
        self.cli('revoke', '--node', self.caller)
        credential = self.h.root / 'copy-source.token'
        self.cli('grant', '--node', self.caller, '--output-token', credential, '--exec', '--projects')
        self.token = credential.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        code = self.h.root / 'source-code'
        code.mkdir()
        (code / 'main.rs').write_text('uncommitted source\n')
        (code / 'large.txt').write_bytes(b'x' * (128 * 1024))
        (code / '.env').write_text('must not transfer')
        (code / '.git').mkdir()
        (code / 'escape').symlink_to(self.h.root, target_is_directory=True)
        project = self.api(operation='projects', node=self.target, command={
            'operation': 'register', 'node': self.target, 'name': 'Copy project', 'path': str(code)})['projects'][0]
        config = self.h.root / 'receiver/settings.json'
        def cli(*args):
            result = subprocess.run([str(NODE), '--config', str(config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        target = cli('identity')['node_id']
        token_file = self.h.root / 'copy-target.token'
        cli('grant', '--node', self.caller, '--output-token', token_file, '--exec', '--projects')
        token = token_file.read_text().strip()
        port = free_port()
        cli('listen', '--address', f'127.0.0.1:{port}')
        process = self.h.start(NODE, '--config', config, 'peer', 'serve')
        self.api(operation='connect', node=target, url=f'http://127.0.0.1:{port}', token=token)
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': target})[0] == 200)
        root = self.h.root / 'received-code'
        root.mkdir()
        return code, project, config, cli, target, token, port, process, root

    def test_web_host_copies_both_directions_without_a_listener_or_gateway(self):
        code, project, _, _, _, _, _, _, _ = self.copy_fixture()
        self.h.stop(self.h.server)
        root = self.h.root / 'host-received'
        root.mkdir()
        command = {'operation': 'copy_prepare', 'source': self.target, 'target': self.caller, 'replica_id': project['replicas'][0]['id']}
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 400, value)
        self.api(operation='copy_root', path=str(root))
        job = self.api(**command)
        self.api(operation='copy_root', path=None)
        failed = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(failed['state'], 'uncertain')
        self.assertEqual(list(root.iterdir()), [])
        self.api(operation='copy_root', path=str(root))
        completed = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(completed['state'], 'completed', completed)
        destination = Path(completed['result']['path'])
        self.assertEqual((destination / 'main.rs').read_text(), 'uncommitted source\n')
        self.assertFalse((destination / '.env').exists())
        status, catalog = self.h.post('/api/projects', {'operation': 'list'})
        self.assertEqual(status, 200, catalog)
        imported = next(p for p in catalog['projects'] if p['id'] == project['id'])
        replica = imported['replicas'][0]
        self.assertEqual(replica['node'], self.caller)
        self.assertEqual(replica['path'], str(destination))
        self.assertEqual(replica['sessions'], [])
        self.assertNotEqual(replica['id'], project['replicas'][0]['id'])
        (destination / 'main.rs').write_text('edited locally after receiving\n')
        # Recover a persisted local receipt without the removed staging package.
        record = self.path.with_name('project-copy-jobs') / (job['id'] + '.json')
        value = json.loads(record.read_text())
        value['state'] = 'uncertain'
        record.write_text(json.dumps(value))
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.api(operation='copy_status', id=job['id'])['state'], 'completed')
        self.assertEqual((destination / 'main.rs').read_text(), 'edited locally after receiving\n')
        remote_root = self.h.root / 'remote-received'
        remote_root.mkdir()
        self.cli('copy-root', '--path', remote_root)
        reverse = self.api(operation='copy_prepare', source=self.caller, target=self.target, replica_id=replica['id'])
        result = self.api(operation='copy_commit', id=reverse['id'], digest=reverse['digest'])
        self.assertEqual(result['state'], 'completed', result)
        self.assertEqual((Path(result['result']['path']) / 'main.rs').read_text(), 'edited locally after receiving\n')
        self.assertEqual((code / 'main.rs').read_text(), 'uncommitted source\n')
        self.assertIsNone(self.api(operation='list')['listener'])
        for source, target, replica_id in [(self.caller, self.caller, replica['id']), (self.caller, self.target, project['replicas'][0]['id'])]:
            self.assertEqual(self.h.post('/api/peers', {'operation': 'copy_prepare', 'source': source, 'target': target, 'replica_id': replica_id})[0], 400)

    def test_native_node_cli_copies_its_local_catalog_both_directions(self):
        _, project, _, receiver_cli, receiver, _, port, _, root = self.copy_fixture()
        credential = self.h.root / 'native-copy.token'
        receiver_cli('grant', '--node', self.target, '--output-token', credential, '--exec', '--projects')
        receiver_cli('copy-root', '--path', root)
        self.cli('connect', '--node', receiver, '--url', f'http://127.0.0.1:{port}', '--credential-file', credential)
        job = self.cli('copy', 'prepare', '--source', self.target, '--target', receiver, '--replica-id', project['replicas'][0]['id'])
        completed = self.cli('copy', 'commit', job['id'], '--digest', job['digest'])
        self.assertEqual(completed['state'], 'completed', completed)
        received = self.api(operation='projects', node=receiver, command={'operation': 'list'})['projects'][0]
        local_root = self.h.root / 'native-received'
        local_root.mkdir()
        self.cli('copy-root', '--path', local_root)
        reverse = self.cli('copy', 'prepare', '--source', receiver, '--target', self.target, '--replica-id', received['replicas'][0]['id'])
        result = self.cli('copy', 'commit', reverse['id'], '--digest', reverse['digest'])
        self.assertEqual(result['state'], 'completed', result)
        destination = Path(result['result']['path'])
        self.assertEqual((destination / 'main.rs').read_text(), 'uncommitted source\n')
        (destination / 'main.rs').write_text('native target changes\n')
        self.assertEqual(self.cli('copy', 'commit', reverse['id'], '--digest', reverse['digest'])['state'], 'completed')
        self.assertEqual((destination / 'main.rs').read_text(), 'native target changes\n')
        self.assertEqual(len(list(local_root.iterdir())), 1)
        copies = self.api(operation='projects', node=self.target, command={'operation': 'list'})['projects'][0]['replicas']
        self.assertEqual(len(copies), 2)
        self.assertTrue(any(r['path'] == str(destination) for r in copies))
        status, denied = self.h.http('POST', f'/api/v1/mesh/nodes/{self.target}/projects/copy-status', {'id': reverse['id']}, port=self.port, headers={
            'Authorization': 'Bearer ' + self.token, 'X-Cranibox-Target': self.target, 'Content-Type': 'application/json'})
        self.assertEqual(status, 404, denied)
        self.assertNotIn(str(destination), json.dumps(denied))

    def test_reviewed_project_copy_preserves_files_and_recovers_receipts_after_restart(self):
        code, project, config, cli, target, token, port, process, root = self.copy_fixture()
        self.h.stop(self.h.server)
        command = {'operation': 'copy_prepare', 'source': self.target, 'target': target, 'replica_id': project['replicas'][0]['id']}
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 400, value)
        self.assertIn('receiving directory', value['error'])
        cli('copy-root', '--path', root)
        job = self.api(**command)
        self.assertEqual(job['state'], 'prepared')
        self.assertEqual(sorted(f['path'] for f in job['summary']['files']), ['large.txt', 'main.rs'])
        self.assertEqual(job['summary']['skipped'], ['.env', '.git', 'escape'])
        self.assertFalse(any('data' in file for file in job['summary']['files']))
        self.assertNotIn(token, json.dumps(job))
        package_path = self.path.with_name('project-copy-jobs') / (job['id'] + '.package')
        package = json.loads(package_path.read_text())
        self.assertEqual(package_path.stat().st_mode & 0o777, 0o600)
        (code / 'main.rs').write_text('changed after review\n')
        status, _ = self.h.post('/api/peers', {'operation': 'copy_commit', 'id': job['id'], 'digest': 'wrong'})
        self.assertEqual(status, 400)
        self.assertEqual(list(root.iterdir()), [])
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.api(operation='copy_list')['copies'][0]['digest'], job['digest'])
        completed = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(completed['state'], 'completed', completed)
        destination = Path(completed['result']['path'])
        self.assertEqual((destination / 'main.rs').read_text(), 'uncommitted source\n')
        self.assertEqual((destination / 'large.txt').stat().st_size, 128 * 1024)
        self.assertFalse((destination / '.env').exists())
        self.assertFalse(package_path.exists())
        catalog = self.api(operation='projects', node=target, command={'operation': 'list'})
        self.assertEqual(catalog['projects'][0]['id'], project['id'])
        self.assertNotEqual(catalog['projects'][0]['replicas'][0]['id'], project['replicas'][0]['id'])
        self.assertEqual(catalog['projects'][0]['replicas'][0]['sessions'], [])
        (destination / 'main.rs').write_text('target edits\n')
        self.h.stop(process)
        process = self.h.start(NODE, '--config', config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': target})[0] == 200)
        headers = {'Authorization': 'Bearer ' + token, 'X-Cranibox-Target': target, 'Content-Type': 'application/json'}
        status, receipt = self.h.http('POST', f'/api/v1/mesh/nodes/{target}/projects/transfer', {
            'operation': 'begin', 'id': job['id'], 'expected_root': str(root), 'digest': job['digest'], 'manifest': package}, port=port, headers=headers)
        self.assertEqual(status, 200, receipt)
        self.assertEqual(receipt['state'], 'completed')
        self.assertEqual((destination / 'main.rs').read_text(), 'target edits\n')
        # Simulate a sender restart after the receiver committed but before its
        # response was journaled. Read-only status recovers without resending data.
        record = package_path.with_suffix('.json')
        value = json.loads(record.read_text())
        value['state'] = 'uncertain'
        record.write_text(json.dumps(value))
        self.assertEqual(self.api(operation='copy_status', id=job['id'])['state'], 'completed')
        self.api(operation='copy_discard', id=job['id'])
        self.assertEqual(self.api(operation='copy_list')['copies'], [])
        self.assertTrue(destination.is_dir())

    def test_copy_recovers_a_lost_http_reply_without_resending_and_rejects_late_routes(self):
        _, project, _, cli, target, token, port, _, root = self.copy_fixture()
        cli('copy-root', '--path', root)
        state = {'imports': 0, 'block_status': False}
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)

        class Proxy(http.server.BaseHTTPRequestHandler):
            def forward(self):
                body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                operation = json.loads(body).get('operation') if body else None
                if (self.path.endswith('/copy-status') or operation == 'status') and state['block_status']:
                    entered.set()
                    release.wait(timeout=5)
                connection = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
                try:
                    headers = {k: v for k, v in self.headers.items() if k.lower() not in ('host', 'connection')}
                    connection.request(self.command, self.path, body=body, headers=headers)
                    response = connection.getresponse()
                    data = response.read()
                    if self.path.endswith('/import') or operation == 'finish':
                        state['imports'] += 1
                        # The receiver committed successfully, but the sender never
                        # sees its HTTP acknowledgement.
                        self.connection.shutdown(socket.SHUT_RDWR)
                        self.connection.close()
                        return
                    self.send_response(response.status)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                finally:
                    connection.close()

            do_GET = forward
            do_POST = forward

            def log_message(self, *args):
                pass

        proxy = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        thread = threading.Thread(target=proxy.serve_forever, daemon=True)
        thread.start()
        def close():
            proxy.shutdown()
            proxy.server_close()
            thread.join(timeout=3)
        self.addCleanup(close)
        self.api(operation='connect', node=target, url=f'http://127.0.0.1:{proxy.server_port}', token=token)
        job = self.api(operation='copy_prepare', source=self.target, target=target, replica_id=project['replicas'][0]['id'])
        unknown = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(unknown['state'], 'uncertain', unknown)
        self.assertEqual(state['imports'], 1)
        destination = next(root.glob('cranibox-*/project'))
        (destination / 'main.rs').write_text('edited after receipt\n')
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        restored = self.api(operation='copy_status', id=job['id'])
        self.assertEqual(restored['state'], 'completed', restored)
        self.assertEqual((destination / 'main.rs').read_text(), 'edited after receipt\n')
        self.assertEqual(state['imports'], 1)
        self.assertEqual(len(list(root.iterdir())), 1)
        self.assertFalse((self.path.with_name('project-copy-jobs') / (job['id'] + '.package')).exists())

        record = self.path.with_name('project-copy-jobs') / (job['id'] + '.json')
        value = json.loads(record.read_text())
        value['state'] = 'uncertain'
        record.write_text(json.dumps(value))
        state['block_status'] = True
        pending = []
        request = threading.Thread(target=lambda: pending.append(self.h.post('/api/peers', {'operation': 'copy_status', 'id': job['id']})))
        request.start()
        try:
            self.assertTrue(entered.wait(timeout=3))
            self.api(operation='forget', node=target)
        finally:
            release.set()
            request.join(timeout=5)
        self.assertFalse(request.is_alive())
        self.assertEqual(pending[0][0], 400)
        self.assertNotIn(str(destination), json.dumps(pending[0][1]))
        self.assertEqual(self.api(operation='copy_list')['copies'], [])
        self.assertEqual(self.api(operation='copy_list')['unavailable'], [job['id']])

    def test_project_copy_rechecks_roots_revocation_and_route_credentials(self):
        _, project, _, cli, target, _, _, _, root = self.copy_fixture()
        cli('copy-root', '--path', root)
        command = {'operation': 'copy_prepare', 'source': self.target, 'target': target, 'replica_id': project['replicas'][0]['id']}
        job = self.api(**command)
        cli('copy-root')
        value = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(value['state'], 'uncertain')
        self.assertEqual(list(root.iterdir()), [])
        cli('copy-root', '--path', root)
        cli('revoke', '--node', self.caller)
        value = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(value['state'], 'uncertain')
        self.assertEqual(list(root.iterdir()), [])
        status, _ = self.h.post('/api/peers', {'operation': 'copy_status', 'id': job['id']})
        self.assertEqual(status, 400)
        self.api(operation='connect', node=target, url='http://127.0.0.1:1', token='a' * 64)
        self.assertEqual(self.api(operation='copy_list')['copies'], [])
        self.assertEqual(self.api(operation='copy_list')['unavailable'], [job['id']])
        status, _ = self.h.post('/api/peers', {'operation': 'copy_commit', 'id': job['id'], 'digest': job['digest']})
        self.assertEqual(status, 400)
        self.assertEqual(list(root.iterdir()), [])
        self.api(operation='copy_discard', id=job['id'])
        self.assertEqual(self.api(operation='copy_list')['unavailable'], [])

    def test_offline_project_snapshot_survives_restart_and_refreshes_from_owner(self):
        self.project_grant()
        catalog = self.api(operation='projects', node=self.target, command={
            'operation': 'register', 'name': 'Offline project', 'node': self.target, 'path': str(self.h.root)})
        replica = catalog['projects'][0]['replicas'][0]
        shell = self.api(operation='sessions', node=self.target, command={
            'operation': 'create', 'name': 'Cached binding', 'cwd': str(self.h.root)})['session']
        catalog = self.api(operation='projects', node=self.target, command={
            'operation': 'bind', 'replica_id': replica['id'], 'terminal_id': shell['id']})
        cache = next(self.path.with_name(self.path.name + '.project-snapshots').glob('*.json'))
        self.assertEqual(cache.stat().st_mode & 0o777, 0o600)
        self.assertNotIn(self.token, cache.read_text())
        self.h.stop(self.target_process)
        offline = self.api(operation='projects', node=self.target, command={'operation': 'list'})
        self.assertEqual(offline['projects'], catalog['projects'])
        self.assertEqual(offline['sync']['state'], 'offline')
        self.assertGreater(offline['sync']['synced_at'], 0)
        self.assertEqual(offline['permissions'], {'projects': True, 'exec': False, 'terminals': False, 'desktop': False})
        before = cache.read_bytes()
        status, _ = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target,
            'command': {'operation': 'forget', 'replica_id': replica['id']}})
        self.assertEqual(status, 400)
        self.assertEqual(cache.read_bytes(), before)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.api(operation='projects', node=self.target, command={'operation': 'list'}), offline)
        cache.write_text('broken derived cache')
        status, _ = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})
        self.assertEqual(status, 400)
        self.assertEqual(cache.read_text(), 'broken derived cache')
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': self.target})[0] == 200)
        fresh = self.api(operation='projects', node=self.target, command={'operation': 'list'})
        self.assertNotIn('sync', fresh)
        self.assertEqual(fresh['projects'], catalog['projects'])
        # Fresh authority repairs a disposable corrupt cache; nothing is written
        # back to the owner from that cache, and no missing Shell is recreated.
        self.assertEqual(json.loads(cache.read_text())['catalog']['projects'], catalog['projects'])
        self.assertEqual(self.api(operation='sessions', node=self.target, command={'operation': 'list'})['sessions'], [])
        self.api(operation='forget', node=self.target)
        self.assertFalse(cache.exists())

    def test_revoked_and_rotated_routes_cannot_reuse_project_snapshots(self):
        self.project_grant()
        self.api(operation='projects', node=self.target, command={
            'operation': 'register', 'name': 'Private project', 'node': self.target, 'path': str(self.h.root)})
        cache = next(self.path.with_name(self.path.name + '.project-snapshots').glob('*.json'))
        self.cli('revoke', '--node', self.caller)
        status, value = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})
        self.assertEqual(status, 400)
        self.assertIn('401', value['error'])
        self.assertFalse(cache.exists())
        self.h.stop(self.target_process)
        self.assertEqual(self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})[0], 400)
        credential = self.h.root / 'replacement-project.token'
        self.cli('grant', '--node', self.caller, '--output-token', credential, '--projects')
        self.token = credential.read_text().strip()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        self.target_process = self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        until(lambda: self.h.post('/api/peers', {'operation': 'probe', 'node': self.target})[0] == 200)
        self.api(operation='projects', node=self.target, command={'operation': 'list'})
        self.assertTrue(cache.exists())
        self.h.stop(self.target_process)
        self.api(operation='connect', node=self.target, url=self.url, token='f' * 64)
        self.assertFalse(cache.exists())
        self.assertEqual(self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})[0], 400)

    def test_malformed_denial_invalidates_cache_and_peer_cannot_forge_sync_state(self):
        state = {'status': 200}
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        target = self.target
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length', 0)))
                if state.get('block'):
                    entered.set()
                    release.wait(timeout=5)
                body = json.dumps({'owner': target, 'revision': 4, 'projects': [],
                    'permissions': {'projects': True}, 'sync': {'state': 'offline', 'synced_at': 123}}).encode() if state['status'] == 200 else b'<html>Access denied</html>'
                self.send_response(state['status'])
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)
        self.addCleanup(close)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)
        self.assertNotIn('sync', self.api(operation='projects', node=self.target, command={'operation': 'list'}))
        cache = next(self.path.with_name(self.path.name + '.project-snapshots').glob('*.json'))
        state['status'] = 403
        status, value = self.h.post('/api/peers', {'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})
        self.assertEqual(status, 400)
        self.assertIn('403', value['error'])
        self.assertFalse(cache.exists())
        state['status'] = 200
        self.api(operation='projects', node=self.target, command={'operation': 'list'})
        state['block'] = True
        pending = []
        request = threading.Thread(target=lambda: pending.append(self.h.post('/api/peers', {
            'operation': 'projects', 'node': self.target, 'command': {'operation': 'list'}})))
        request.start()
        try:
            self.assertTrue(entered.wait(timeout=3))
            self.api(operation='forget', node=self.target)
            self.assertFalse(cache.exists())
        finally:
            release.set()
            request.join(timeout=5)
        self.assertFalse(request.is_alive())
        self.assertEqual(pending[0][0], 400)
        self.assertIn('removed', pending[0][1]['error'])
        self.assertFalse(cache.exists(), 'A late response must not restore a forgotten connection snapshot')

    def test_incoming_grants_authorize_a_real_peer_without_gateway(self):
        self.h.stop(self.h.server)
        before = self.path.read_bytes()
        for command in [
            {'operation': 'grant', 'node': self.caller, 'permissions': {'exec': True}},
            {'operation': 'grant', 'node': self.target, 'permissions': {}},
            {'operation': 'grant', 'node': 'invalid/node', 'permissions': {'exec': True}},
        ]:
            status, _ = self.h.post('/api/peers', command)
            self.assertEqual(status, 400)
            self.assertEqual(self.path.read_bytes(), before)
        issued = self.api(operation='grant', node=self.target, permissions={'terminals': True})
        secret = issued['token']
        self.assertRegex(secret, r'^[a-fA-F0-9]{64}$')
        self.assertNotIn(secret, json.dumps(issued['catalog']))
        self.assertNotIn(secret, json.dumps(self.api(operation='list')))
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        saved = self.path.read_bytes()
        status, _ = self.h.post('/api/peers', {'operation': 'grant', 'node': self.target, 'permissions': {'exec': True}})
        self.assertEqual(status, 400)
        self.assertEqual(self.path.read_bytes(), saved)

        listener = self.api(operation='listener', listener={'address': '127.0.0.1:0'})
        self.assertTrue(listener['listener_runtime']['active'])
        self.assertTrue(listener['listener_runtime']['applied'])
        port = int(listener['listener_runtime']['address'].rsplit(':', 1)[1])
        token_file = self.h.root / 'inbound-grant.token'
        token_file.write_text(secret)
        token_file.chmod(0o600)
        self.cli('connect', '--node', self.caller, '--url', f'http://127.0.0.1:{port}', '--credential-file', token_file)
        until(lambda: subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', 'call', '--node', self.caller, 'nodes'], capture_output=True, timeout=5).returncode == 0)
        nodes = self.cli('call', '--node', self.caller, 'nodes')['nodes']
        self.assertTrue(nodes[0]['terminal_sessions'])
        self.assertFalse(nodes[0]['exec'])
        shell = self.cli('call', '--node', self.caller, 'sessions', 'create', '--name', 'Incoming browser grant')['session']
        self.assertEqual(shell['status'], 'running')
        self.cli('call', '--node', self.caller, 'sessions', 'end', shell['id'])
        result = subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', 'call', '--node', self.caller, 'exec', '--', sys.executable, '-c', 'print("must be denied")'], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('403', result.stderr)

        self.api(operation='revoke', node=self.target)
        self.assertEqual(self.api(operation='list')['grants'], [])
        result = subprocess.run([str(NODE), '--config', str(self.target_config), 'peer', 'call', '--node', self.caller, 'nodes'], capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('401', result.stderr)
        rotated = self.api(operation='grant', node=self.target, permissions={'exec': True})
        self.assertNotEqual(rotated['token'], secret)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        view = self.api(operation='list')
        self.assertEqual(view['grants'], [{'node': self.target, 'permissions': {'exec': True, 'terminals': False, 'projects': False, 'desktop': False, 'computer': False, 'browser': False, 'messages': False, 'activity': False, 'spaces': False}}])
        self.assertNotIn(secret, json.dumps(view))
        self.assertNotIn(rotated['token'], json.dumps(view))

    def test_listener_changes_preserve_jobs_and_shells_and_reject_failed_updates(self):
        self.h.stop(self.h.server)
        self.assertFalse(self.api(operation='list')['listener_runtime']['active'])
        issued = self.api(operation='grant', node=self.target, permissions={'exec': True, 'terminals': True})
        headers = {'Authorization': 'Bearer ' + issued['token'], 'X-Cranibox-Target': self.caller, 'Content-Type': 'application/json'}
        port = None

        def direct(method, path, body=None):
            status, value = self.h.http(method, '/api/v1/mesh' + path, body, port=port, headers=headers)
            self.assertEqual(status, 200, value)
            return value

        def configure(address):
            view = self.api(operation='listener', listener={'address': address})
            self.assertTrue(view['listener_runtime']['active'])
            self.assertTrue(view['listener_runtime']['applied'])
            return int(view['listener_runtime']['address'].rsplit(':', 1)[1])

        port = configure('127.0.0.1:0')
        self.assertEqual(direct('GET', '/nodes')['nodes'][0]['id'], self.caller)
        shell = direct('POST', f'/nodes/{self.caller}/terminal/sessions', {'operation': 'create', 'name': 'Survives listener changes'})['session']
        marker = self.h.root / 'release-listener-job'
        job = direct('POST', f'/nodes/{self.caller}/exec', {'program': sys.executable, 'args': ['-c', f'import os,time; print("job started",flush=True)\nwhile not os.path.exists({str(marker)!r}): time.sleep(.02)\nprint("same job finished")'], 'timeout_secs': 30})
        until(lambda: 'job started' in direct('GET', f'/jobs/{job["id"]}')['stdout'])
        before = self.path.read_bytes()
        with socket.socket() as occupied:
            occupied.bind(('127.0.0.1', 0))
            occupied.listen()
            status, _ = self.h.post('/api/peers', {'operation': 'listener', 'listener': {'address': '127.0.0.1:' + str(occupied.getsockname()[1])}})
            self.assertEqual(status, 400)
        status, _ = self.h.post('/api/peers', {'operation': 'listener', 'listener': {'address': '0.0.0.0:0'}})
        self.assertEqual(status, 400)
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(direct('GET', f'/jobs/{job["id"]}')['outcome'], 'running')

        old_port = port
        port = configure('127.0.0.1:' + str(free_port()))
        with self.assertRaises(OSError):
            socket.create_connection(('127.0.0.1', old_port), timeout=1)
        self.assertEqual(direct('GET', f'/jobs/{job["id"]}')['outcome'], 'running')
        disabled = self.api(operation='listener', listener=None)
        self.assertIsNone(disabled['listener'])
        self.assertFalse(disabled['listener_runtime']['active'])
        self.assertTrue(disabled['listener_runtime']['applied'])
        self.assertEqual(disabled['grants'], issued['catalog']['grants'])
        port = configure('127.0.0.1:0')
        marker.touch()
        result = until(lambda: (v if (v := direct('GET', f'/jobs/{job["id"]}'))['outcome'] != 'running' else None))
        self.assertEqual(result['exit_code'], 0)
        self.assertIn('same job finished', result['stdout'])
        self.assertIn(shell['id'], [s['id'] for s in direct('POST', f'/nodes/{self.caller}/terminal/sessions', {'operation': 'list'})['sessions']])
        direct('POST', f'/nodes/{self.caller}/terminal/sessions', {'operation': 'end', 'id': shell['id']})

        self.h.stop(self.h.web)
        with socket.socket() as occupied:
            occupied.bind(('127.0.0.1', 0))
            occupied.listen()
            config = json.loads(self.path.read_text())
            config['listener']['address'] = '127.0.0.1:' + str(occupied.getsockname()[1])
            self.path.write_text(json.dumps(config))
            self.h.start_web()
            self.h.login()
            view = self.api(operation='list')
            self.assertEqual(view['node_id'], self.caller)
            self.assertFalse(view['listener_runtime']['active'])
            self.assertFalse(view['listener_runtime']['applied'])
            self.assertIn('bind direct node listener', view['listener_runtime']['error'])
            port = configure('127.0.0.1:0')
            self.assertEqual(direct('GET', '/nodes')['nodes'][0]['id'], self.caller)


if __name__ == '__main__':
    unittest.main()
