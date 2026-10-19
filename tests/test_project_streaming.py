"""Disk-backed project snapshots over real HTTP, including ambiguous chunk replies."""
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import socket
import threading
import unittest

import test_web_direct_peers as direct
from workspace_web_harness import CORE, NODE, RUNTIME, WEB, sockets_allowed, until

CHUNK = 1024 * 1024


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class ProjectStreamingTests(unittest.TestCase):
    setUp = direct.WebDirectPeersTests.setUp
    cli = direct.WebDirectPeersTests.cli
    copy_fixture = direct.WebDirectPeersTests.copy_fixture

    def api(self, **command):
        connection = http.client.HTTPConnection('127.0.0.1', self.h.web_port, timeout=60)
        try:
            connection.request('POST', '/api/peers', json.dumps(command), self.h.headers())
            response = connection.getresponse()
            value = json.loads(response.read())
            self.assertEqual(response.status, 200, value)
            self.assertNotIn(self.token if hasattr(self, 'token') else 'never-a-real-token', json.dumps(value))
            return value
        finally:
            connection.close()

    def proxy(self, port, *, drop=lambda command: False, before=lambda command: None, legacy=False):
        requests = []

        class Proxy(http.server.BaseHTTPRequestHandler):
            def forward(self):
                if self.path.endswith('/api/v1/mesh/keys'):
                    # This fault injector reads plaintext operations, so it poses as an
                    # older target without end-to-end keys (bearer fallback).
                    self.send_response(404)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                command = json.loads(body) if body else {}
                requests.append({key: command[key] for key in ('operation', 'index', 'offset') if key in command} | ({'offset': command['chunk']['offset']} if 'chunk' in command else {}))
                before(command)
                connection = http.client.HTTPConnection('127.0.0.1', port, timeout=60)
                try:
                    connection.request(self.command, self.path, body, {key: value for key, value in self.headers.items() if key.lower() not in ('host', 'connection')})
                    response = connection.getresponse()
                    data = response.read()
                    if legacy and self.path.endswith('/nodes') and response.status == 200:
                        value = json.loads(data)
                        for node in value.get('nodes', []):
                            node.pop('project_copy_protocol', None)
                        data = json.dumps(value).encode()
                    if drop(command):
                        self.connection.shutdown(socket.SHUT_RDWR)
                        self.connection.close()
                        return
                    self.send_response(response.status)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                finally:
                    connection.close()

            do_GET = forward
            do_POST = forward

            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()

        def close():
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)
        self.addCleanup(close)
        return server.server_port, requests

    def test_large_preparation_resumes_the_original_source_snapshot_after_restarts(self):
        code, project, _, receiver, target, _, _, _, root = self.copy_fixture()
        receiver('copy-root', '--path', root)
        content = b'portable source\n' * (20 * CHUNK // 17 + 1)
        (code / 'big.txt').write_bytes(content)
        (code / 'empty').write_bytes(b'')
        dropped = []

        def drop(command):
            if command.get('operation') == 'read' and command['offset'] == CHUNK and not dropped:
                dropped.append(True)
                return True
            return False
        port, requests = self.proxy(self.port, drop=drop)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{port}', token=self.token)
        status, _ = self.h.post('/api/peers', {'operation': 'copy_prepare', 'source': self.target, 'target': target, 'replica_id': project['replicas'][0]['id']})
        self.assertEqual(status, 400)
        job = self.api(operation='copy_list')['copies'][0]
        self.assertEqual(job['protocol'], 2)
        self.assertEqual(job['state'], 'prepare_paused')
        self.assertEqual(job['progress']['bytes'], CHUNK)
        self.assertEqual(self.api(operation='copy_activity')['copies'][0]['progress']['bytes'], CHUNK)
        self.assertNotIn('summary', self.api(operation='copy_activity')['copies'][0])
        (code / 'big.txt').write_bytes(b'changed after export')
        self.h.stop(self.target_process)
        self.h.start(NODE, '--config', self.target_config, 'peer', 'serve')
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        job = self.api(operation='copy_prepare_resume', id=job['id'])
        self.assertEqual(job['state'], 'prepared')
        self.assertGreater(job['summary']['bytes'], 16 * CHUNK)
        manifest = self.path.with_name('project-copy-jobs') / (job['id'] + '.package')
        value = json.loads(manifest.read_text())
        self.assertLess(manifest.stat().st_size, CHUNK)
        self.assertTrue(all(file['data'] == '' for file in value['files']))
        big_index = next(i for i, file in enumerate(value['files']) if file['path'] == 'big.txt')
        self.assertEqual(len([r for r in requests if r.get('operation') == 'read' and r.get('index') == big_index and r.get('offset') == 0]), 1)
        self.h.stop(self.h.server)
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'completed', job)
        destination = Path(job['result']['path'])
        self.assertEqual(hashlib.sha256((destination / 'big.txt').read_bytes()).digest(), hashlib.sha256(content).digest())
        self.assertFalse((destination / '.env').exists())
        self.assertEqual((destination / 'empty').stat().st_size, 0)
        self.assertFalse(manifest.exists())
        self.assertFalse(manifest.with_suffix('.files').exists())

    def test_lost_write_reply_resumes_from_receiver_offset_after_both_processes_restart(self):
        code, project, config, receiver, target, token, receiver_port, process, root = self.copy_fixture()
        receiver('copy-root', '--path', root)
        content = b'z' * (CHUNK * 5 + 37)
        (code / 'big.txt').write_bytes(content)
        dropped = []

        def drop(command):
            if command.get('operation') == 'write' and command['chunk']['offset'] == CHUNK and not dropped:
                dropped.append(True)
                return True
            return False
        port, requests = self.proxy(receiver_port, drop=drop)
        self.api(operation='connect', node=target, url=f'http://127.0.0.1:{port}', token=token)
        job = self.api(operation='copy_prepare', source=self.target, target=target, replica_id=project['replicas'][0]['id'])
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'uncertain', job)
        self.assertEqual(len([r for r in requests if r.get('operation') == 'write']), 2)
        self.assertFalse(list(root.glob('cranibox-*/project')))
        self.h.stop(process)
        self.h.start(NODE, '--config', config, 'peer', 'serve')
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        job = self.api(operation='copy_status', id=job['id'])
        self.assertEqual(job['state'], 'receiving')
        self.assertEqual(job['progress']['bytes'], 2 * CHUNK)
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'completed', job)
        self.assertEqual((Path(job['result']['path']) / 'big.txt').read_bytes(), content)
        offsets = [r['offset'] for r in requests if r.get('operation') == 'write']
        self.assertEqual(offsets.count(0), 3)  # big.txt, large.txt and main.rs
        self.assertEqual(offsets.count(CHUNK), 1)
        self.assertEqual(len(list(root.iterdir())), 1)

    def test_closing_the_http_request_keeps_the_owned_transfer_running(self):
        _, project, _, receiver, target, token, receiver_port, _, root = self.copy_fixture()
        receiver('copy-root', '--path', root)
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)

        def before(command):
            if command.get('operation') == 'write':
                entered.set()
                release.wait(timeout=5)
        port, _ = self.proxy(receiver_port, before=before)
        self.api(operation='connect', node=target, url=f'http://127.0.0.1:{port}', token=token)
        job = self.api(operation='copy_prepare', source=self.target, target=target, replica_id=project['replicas'][0]['id'])
        connection = http.client.HTTPConnection('127.0.0.1', self.h.web_port, timeout=10)
        try:
            connection.request('POST', '/api/peers', json.dumps({'operation': 'copy_commit', 'id': job['id'], 'digest': job['digest']}), self.h.headers())
            self.assertTrue(entered.wait(timeout=5))
            connection.sock.shutdown(socket.SHUT_RDWR)
        finally:
            connection.close()
            release.set()
        until(lambda: self.api(operation='copy_activity')['copies'][0]['state'] == 'completed')
        result = self.api(operation='copy_list')['copies'][0]
        self.assertTrue(Path(result['result']['path']).is_dir())
        self.assertEqual(len(list(root.iterdir())), 1)

    def test_legacy_capability_negotiation_retains_the_bounded_v1_protocol(self):
        _, project, _, receiver, target, token, receiver_port, _, root = self.copy_fixture()
        receiver('copy-root', '--path', root)
        port, requests = self.proxy(receiver_port, legacy=True)
        self.api(operation='connect', node=target, url=f'http://127.0.0.1:{port}', token=token)
        job = self.api(operation='copy_prepare', source=self.target, target=target, replica_id=project['replicas'][0]['id'])
        self.assertEqual(job['protocol'], 1)
        package = self.path.with_name('project-copy-jobs') / (job['id'] + '.package')
        self.assertEqual(json.loads(package.read_text())['version'], 1)
        job = self.api(operation='copy_commit', id=job['id'], digest=job['digest'])
        self.assertEqual(job['state'], 'completed')
        self.assertFalse(any(request.get('operation') == 'begin' for request in requests))
