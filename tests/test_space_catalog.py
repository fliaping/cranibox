"""Read-only space federation through real Rust nodes; never accesses Docker."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import subprocess
import threading
import unittest
import uuid

from workspace_web_harness import Harness, CORE, NODE, WEB, free_port, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class SpaceCatalogTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.caller = self.api(operation='identity')['node_id']
        self.config = self.h.root / 'target/settings.json'
        self.target = self.cli('identity')['node_id']
        self.secret = 'private-password-do-not-export'
        self.space = dict(id=str(uuid.uuid4()), name='Agent lab 中文', provider='docker',
                          image='private-registry.example/image:secret', node='space-node', owner=self.target,
                          password=self.secret, token='a'*64, phase='pulling', error='/private/failure/details',
                          ports={'peer':19020, 'desktop':13001, 'code':18443})
        self.catalog = self.config.with_name('spaces') / 'catalog.json'
        self.catalog.parent.mkdir(parents=True)
        self.save()
        self.token_file = self.h.root / 'spaces.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', self.token_file, '--spaces')
        self.token = self.token_file.read_text().strip()
        self.port = free_port()
        self.cli('listen', '--address', f'127.0.0.1:{self.port}')
        self.process = self.h.start(NODE, '--config', self.config, 'peer', 'serve')
        until(lambda: self.direct('GET', '/api/v1/mesh/nodes')[0] == 200)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{self.port}', token=self.token)

    def save(self):
        self.catalog.write_text(json.dumps(dict(version=1, installation=str(uuid.uuid4()), spaces=[self.space], setup=None)))
        self.catalog.chmod(0o600)

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def api(self, **command):
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, value)
        return value

    def direct(self, method, path, body=None, headers=None):
        return self.h.http(method, path, body, port=self.port, headers={
            'Authorization':'Bearer '+self.token, 'X-Cranibox-Target':self.target,
            'Content-Type':'application/json', **(headers or {})})

    def test_shared_metadata_is_read_only_and_available_from_web_and_cli(self):
        before = self.catalog.read_bytes()
        expected = {'owner':self.target, 'spaces':[{key:self.space[key] for key in ('id','name','provider','node','phase')}]}
        self.assertEqual(self.api(operation='space_catalog', node=self.target), expected)
        caller_config = self.h.root / '.config/cranibox/settings.json'
        call = subprocess.run([str(NODE), '--config', str(caller_config), 'peer', 'call', '--node', self.target, 'spaces'],
                              capture_output=True, text=True, timeout=10)
        self.assertEqual(call.returncode, 0, call.stderr)
        self.assertEqual(json.loads(call.stdout), expected)
        self.assertEqual(self.catalog.read_bytes(), before, 'reading must not recover an operation or touch Docker')
        self.space['name'] = 'Renamed on owner'
        self.save()
        self.assertEqual(self.api(operation='space_catalog', node=self.target)['spaces'][0]['name'], self.space['name'])
        self.h.stop(self.process)
        status, value = self.h.post('/api/peers', {'operation':'space_catalog','node':self.target})
        self.assertEqual(status, 400)
        self.assertNotIn(self.secret, json.dumps(value))

    def test_space_permission_does_not_grant_control_credentials_or_other_capabilities(self):
        base = f'/api/v1/mesh/nodes/{self.target}'
        caps = self.direct('GET', '/api/v1/mesh/nodes')[1]['nodes'][0]
        self.assertTrue(caps['spaces'])
        for key in ('exec','terminal_sessions','projects','desktop','messages'):
            self.assertFalse(caps[key])
        self.assertEqual(self.direct('POST', base+'/spaces', {'operation':'stop','id':self.space['id']})[0], 405)
        self.assertEqual(self.direct('POST', base+'/exec', {'program':'touch','args':[str(self.h.root/'must-not-run')],'timeout_secs':1})[0], 403)
        self.assertEqual(self.direct('POST', base+'/projects', {'operation':'list'})[0], 403)
        self.assertEqual(self.direct('GET', base+'/spaces', headers={'Origin':'https://other.example'})[0], 403)
        self.assertEqual(self.direct('GET', base+'/spaces', headers={'X-Cranibox-Target':'other'})[0], 409)
        self.assertEqual(self.direct('GET', '/api/v1/mesh/nodes/other/spaces')[0], 404)
        self.cli('revoke', '--node', self.caller)
        self.assertEqual(self.direct('GET', base+'/spaces')[0], 401)
        other = self.h.root / 'exec-only.token'
        self.cli('grant', '--allow-unverified', '--node', self.caller, '--output-token', other, '--exec')
        self.token = other.read_text().strip()
        self.assertFalse(self.direct('GET', '/api/v1/mesh/nodes')[1]['nodes'][0]['spaces'])
        self.assertEqual(self.direct('GET', base+'/spaces')[0], 403)
        self.assertFalse((self.h.root/'must-not-run').exists())

    def test_corrupt_or_foreign_catalog_is_preserved_and_private_failures_are_hidden(self):
        for body in (b'{broken-private-data', json.dumps({'version':1,'installation':str(uuid.uuid4()),
                     'spaces':[{**self.space,'owner':'another-owner'}]}).encode()):
            self.catalog.write_bytes(body)
            status, value = self.direct('GET', f'/api/v1/mesh/nodes/{self.target}/spaces')
            self.assertEqual(status, 503)
            self.assertNotIn(self.secret, json.dumps(value))
            self.assertEqual(self.catalog.read_bytes(), body)

    def test_remote_responses_cannot_change_owner_or_export_private_fields(self):
        expected = {'owner':self.target, 'spaces':[{key:self.space[key] for key in ('id','name','provider','node','phase')}]}
        response = [expected]
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                encoded = json.dumps(response[0]).encode()
                self.send_response(200)
                self.send_header('Content-Type','application/json')
                self.send_header('Content-Length',str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)
        self.assertEqual(self.api(operation='space_catalog', node=self.target), expected)
        for value in ({**expected,'owner':'wrong-owner'}, {**expected,'spaces':[{**expected['spaces'][0],'password':self.secret}]},
                      {**expected,'spaces':[expected['spaces'][0]]*33}):
            response[0] = value
            status, error = self.h.post('/api/peers', {'operation':'space_catalog','node':self.target})
            self.assertEqual(status, 400)
            self.assertNotIn(self.secret, json.dumps(error))

    def test_connection_removal_or_same_file_token_rotation_rejects_in_flight_metadata(self):
        arrived, release = threading.Event(), threading.Event()
        expected = {'owner':self.target, 'spaces':[{key:self.space[key] for key in ('id','name','provider','node','phase') }]}
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                arrived.set()
                if not release.wait(5):
                    return
                encoded = json.dumps(expected).encode()
                self.send_response(200)
                self.send_header('Content-Type','application/json')
                self.send_header('Content-Length',str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.addCleanup(release.set)
        for change in ('same-file-token-rotation', 'route-removal'):
            with self.subTest(change=change):
                arrived.clear()
                release.clear()
                self.api(operation='connect', node=self.target, url=f'http://127.0.0.1:{server.server_port}', token=self.token)
                result, failures = [], []
                def fetch():
                    try:
                        result.append(self.h.post('/api/peers', {'operation':'space_catalog','node':self.target}))
                    except Exception as error:
                        failures.append(error)
                worker = threading.Thread(target=fetch, daemon=True)
                worker.start()
                self.assertTrue(arrived.wait(4))
                if change == 'same-file-token-rotation':
                    peers = json.loads((self.h.root/'.config/cranibox/peers.json').read_text())
                    token_path = peers['routes'][self.target]['token_file']
                    with open(token_path, 'w') as token:
                        token.write('b'*64+'\n')
                else:
                    self.api(operation='forget', node=self.target)
                release.set()
                worker.join(timeout=5)
                self.assertFalse(worker.is_alive())
                self.assertEqual(failures, [])
                self.assertEqual(result[0][0], 400, result)
                self.assertNotIn(self.space['name'], json.dumps(result))
                self.assertNotIn(self.secret, json.dumps(result))


if __name__ == '__main__':
    unittest.main()
