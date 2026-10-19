"""Node-to-node end-to-end encryption, key pinning and pairing verification (real processes)."""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
import uuid

from workspace_web_harness import NODE, free_port, sockets_allowed, until


class RecordingProxy:
    """A relay that forwards TCP bytes and records everything it sees, like a gateway would."""

    def __init__(self, upstream, tamper_streams=False):
        self.upstream = upstream
        # Flip one bit of the first target-to-caller chunk after a WebSocket upgrade.
        self.tamper_streams = tamper_streams
        self.tampered = 0
        self.seen = bytearray()
        self.lock = threading.Lock()
        self.server = socket.socket()
        self.server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server.bind(('127.0.0.1', 0))
        self.server.listen(16)
        self.port = self.server.getsockname()[1]
        self.closed = False
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while not self.closed:
            try:
                client, _ = self.server.accept()
            except OSError:
                return
            upstream = socket.create_connection(('127.0.0.1', self.upstream))
            for a, b, back in ((client, upstream, False), (upstream, client, True)):
                threading.Thread(target=self.pipe, args=(a, b, back), daemon=True).start()

    def pipe(self, source, sink, back=False):
        upgraded = False
        try:
            while data := source.recv(65536):
                with self.lock:
                    self.seen += data
                if back and not upgraded and data.startswith(b'HTTP/1.1 101'):
                    upgraded = True
                    head, _, rest = data.partition(b'\r\n\r\n')
                    sink.sendall(head + b'\r\n\r\n')
                    data = rest
                # Only data frames (FIN + binary) are end-to-end protected; control frames
                # such as heartbeat pongs are not, so leave those intact.
                if back and upgraded and self.tamper_streams and not self.tampered and data[:1] == b'\x82' and len(data) > 24:
                    # Flip the first payload byte of that frame (a coalesced chunk may
                    # carry further frames after it).
                    code = data[1] & 127
                    at = 2 + (2 if code == 126 else 8 if code == 127 else 0)
                    data = data[:at] + bytes([data[at] ^ 1]) + data[at + 1:]
                    self.tampered += 1
                if data:
                    sink.sendall(data)
        except OSError:
            pass
        finally:
            for s in (source, sink):
                try:
                    s.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                s.close()

    def close(self):
        self.closed = True
        self.server.close()


@unittest.skipUnless(os.name == 'posix' and NODE.exists() and sockets_allowed(), 'build cranibox-node; real sockets required')
class NodeE2eTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-e2e-')
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.a, self.b = self.root / 'a/settings.json', self.root / 'b/settings.json'
        self.caller = self.cli(self.a, 'identity')['node_id']
        self.target = self.cli(self.b, 'identity')['node_id']
        self.port = free_port()
        self.cli(self.b, 'listen', '--address', f'127.0.0.1:{self.port}')
        self.process = None
        self.proxy = None
        self.grant()

    def tearDown(self):
        if self.proxy:
            self.proxy.close()
        if self.process and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=8)
        if getattr(self, 'log', None):
            self.log.close()

    def grant(self, *extra, strict=False):
        # Verified pairing is the default; most cases here exercise the channel itself.
        mode = () if strict else ('--allow-unverified',)
        self.token_path = self.root / f'token-{uuid.uuid4().hex}'
        self.cli(self.b, 'grant', '--node', self.caller, '--output-token', self.token_path, '--exec', *mode, *extra)
        self.token = self.token_path.read_text().strip()

    def cli(self, config, *args, check=True):
        result = subprocess.run([str(NODE), '--config', str(config), 'peer', *map(str, args)],
                                capture_output=True, text=True, timeout=20)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        return result

    def serve(self):
        self.log = (self.root / 'serve.log').open('wb')
        self.process = subprocess.Popen([str(NODE), '--config', str(self.b), 'peer', 'serve'],
                                        stdout=self.log, stderr=self.log,
                                        env={**os.environ, 'HOME': str(self.root), 'CRANIBOX_HERDR_BIN': '/missing-herdr'})
        until(lambda: self.raw('GET', '/api/v1/mesh/keys')[0] == 200)
        self.proxy = RecordingProxy(self.port)
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.proxy.port}',
                 '--credential-file', self.token_path)

    def raw(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        try:
            connection.request(method, path, body, {'X-Cranibox-Target': self.target, **(headers or {})})
            response = connection.getresponse()
            return response.status, response.read()
        except OSError:
            return 0, b''
        finally:
            connection.close()

    def bearer(self, path='/api/v1/mesh/nodes'):
        return self.raw('GET', path, headers={'Authorization': 'Bearer ' + self.token})[0]

    def run_marker(self, check=True):
        marker = 'E2E_SECRET_' + uuid.uuid4().hex
        result = self.cli(self.a, 'call', '--node', self.target, 'exec', sys.executable, '-c', f'print("{marker}")', check=check)
        if not check:
            return marker, result
        job = until(lambda: (value if (value := self.cli(self.a, 'call', '--node', self.target, 'job', result['id']))['outcome'] != 'running' else None))
        self.assertEqual(job['stdout'], marker + '\n')
        return marker, job

    def views(self):
        return self.cli(self.a, 'identity'), self.cli(self.b, 'identity')

    def test_relay_sees_only_ciphertext_and_keys_are_pinned_on_both_sides(self):
        self.serve()
        marker, _ = self.run_marker()
        with self.proxy.lock:
            seen = bytes(self.proxy.seen)
        self.assertIn(b'/e2e/call', seen, 'calls went through the relay')
        self.assertNotIn(marker.encode(), seen, 'command and output must not cross the relay in plaintext')
        self.assertNotIn(self.token.encode(), seen, 'the bearer secret must not cross the relay')
        self.assertNotIn(b'Bearer', seen)
        caller_view, target_view = self.views()
        route = next(r for r in caller_view['routes'] if r['node'] == self.target)
        grant = next(g for g in target_view['grants'] if g['node'] == self.caller)
        self.assertTrue(grant['signed'])
        self.assertEqual(route['identity']['fingerprint'], target_view['identity']['fingerprint'])
        self.assertEqual(grant['peer']['fingerprint'], caller_view['identity']['fingerprint'])
        # Both owners see the same short code to compare out of band.
        self.assertRegex(route['identity']['pairing_code'], r'^\d{4}-\d{4}-\d{4}$')
        self.assertEqual(route['identity']['pairing_code'], grant['peer']['pairing_code'])
        self.assertFalse(route['identity']['verified'])
        # peers.json stores only a salted digest of the secret.
        stored = (self.b.parent / 'peers.json').read_text()
        self.assertNotIn(self.token, stored)

    def test_tampered_replayed_and_unknown_session_frames_never_run(self):
        self.serve()
        self.run_marker()
        with self.proxy.lock:
            seen = bytes(self.proxy.seen)
        start = seen.rindex(b'POST /api/v1/mesh/nodes/')
        head, _, rest = seen[start:].partition(b'\r\n\r\n')
        headers = dict(line.split(b': ', 1) for line in head.split(b'\r\n')[1:])
        length = int(headers[b'content-length'])
        frame, session = rest[:length], headers[b'x-cranibox-session'].decode()
        path = head.split(b' ')[1].decode()
        call = lambda body, sid=session: self.raw('POST', path, body, {'X-Cranibox-Session': sid, 'Content-Type': 'application/octet-stream'})[0]
        self.assertNotEqual(call(frame), 200, 'an exact replay is refused')
        tampered = bytearray(frame)
        tampered[-1] ^= 1
        self.assertNotEqual(call(bytes(tampered)), 200, 'a modified frame is refused')
        self.assertEqual(call(frame, 'unknown-session'), 410)
        self.assertNotEqual(self.raw('POST', path.replace('/e2e/call', '/e2e/handshake'), b'\0' * 64,
                                     {'Content-Type': 'application/octet-stream'})[0], 200)
        # The genuine client keeps working afterwards.
        self.run_marker()

    def test_default_grant_requires_verified_pairing_and_refuses_plaintext(self):
        self.cli(self.b, 'revoke', '--node', self.caller)
        self.grant(strict=True)  # no flag: strict is the default
        grant = next(g for g in self.views()[1]['grants'] if g['node'] == self.caller)
        self.assertTrue(grant['require_verified'])
        self.serve()
        _, failed = self.run_marker(check=False)
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn('pairing verification pending', failed.stderr)
        self.assertEqual(self.bearer(), 401, 'a copied secret alone is not enough on a strict grant')
        caller_view, target_view = self.views()
        code = next(g for g in target_view['grants'] if g['node'] == self.caller)['peer']['pairing_code']
        wrong = self.cli(self.b, 'verify', '--node', self.caller, '--code', '000000000000', check=False)
        self.assertNotEqual(wrong.returncode, 0)
        self.assertEqual(self.cli(self.b, 'verify', '--node', self.caller, '--code', code)['verified'], ['grant'])
        self.assertEqual(self.cli(self.a, 'verify', '--node', self.target, '--code', code)['verified'], ['route'])
        self.run_marker()
        self.assertEqual(self.bearer(), 401)

    def test_changed_target_key_is_refused_by_a_paired_caller(self):
        self.serve()
        self.run_marker()
        self.process.terminate()
        self.process.wait(timeout=8)
        self.log.close()
        # Simulate a different device answering for the same address and NodeID.
        identity = self.b.parent / 'settings.json.identity'
        if not identity.exists():
            identity = next(p for p in self.b.parent.iterdir() if 'identity' in p.name)
        identity.unlink()
        peers = json.loads((self.b.parent / 'peers.json').read_text())
        peers.pop('identity', None)
        for grant in peers['grants'].values():
            grant.pop('signature', None)
            grant.pop('peer', None)
        peers['version'] = 1
        (self.b.parent / 'peers.json').write_text(json.dumps(peers))
        self.log = (self.root / 'serve2.log').open('wb')
        self.process = subprocess.Popen([str(NODE), '--config', str(self.b), 'peer', 'serve'],
                                        stdout=self.log, stderr=self.log,
                                        env={**os.environ, 'HOME': str(self.root), 'CRANIBOX_HERDR_BIN': '/missing-herdr'})
        until(lambda: self.raw('GET', '/api/v1/mesh/keys')[0] == 200)
        _, failed = self.run_marker(check=False)
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn('handshake rejected', failed.stderr)


    def test_explicit_opt_out_allows_unverified_callers_and_conflicts_with_strict(self):
        self.cli(self.b, 'revoke', '--node', self.caller)
        both = self.cli(self.b, 'grant', '--node', self.caller, '--output-token', self.root / 'both.token', '--exec',
                        '--allow-unverified', '--require-verified', check=False)
        self.assertNotEqual(both.returncode, 0)
        self.grant()  # --allow-unverified
        grant = next(g for g in self.views()[1]['grants'] if g['node'] == self.caller)
        self.assertFalse(grant['require_verified'])
        self.serve()
        self.run_marker()
        # Compatibility: plaintext bearer still works only on an explicit opt-out grant.
        self.assertEqual(self.bearer(), 200)
        self.assertFalse(next(g for g in self.views()[1]['grants'] if g['node'] == self.caller)['peer']['verified'])


if __name__ == '__main__':
    unittest.main()
