"""Network management through the real Rust Web API, without changing host services."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import socket
from pathlib import Path
import sys
import tempfile
import threading
import unittest
import urllib.request
from unittest.mock import patch

from workspace_web_harness import Harness, CORE, NODE, WEB, sockets_allowed, until


@unittest.skipUnless(sys.platform == 'linux' and all(p.exists() for p in (CORE, NODE, WEB)) and sockets_allowed(), 'Linux Rust binaries and real sockets required')
class DeviceNetworkTests(unittest.TestCase):
    def setUp(self):
        fixture = tempfile.TemporaryDirectory(prefix='cranibox-network-api-')
        self.addCleanup(fixture.cleanup)
        self.root = Path(fixture.name)
        tailscale = self.root / 'tailscale'
        tailscale.write_text('#!' + sys.executable + '\nimport json\nprint(json.dumps({"BackendState":"Stopped","PrivateKey":"MUST_NOT_LEAK","Peer":{}}))\n')
        tailscale.chmod(0o700)
        environment = patch.dict(os.environ, {'PATH': str(self.root) + os.pathsep + os.environ.get('PATH', '')})
        environment.start()
        self.addCleanup(environment.stop)
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.config = self.h.root / '.config/cranibox-node/networks.json'
        self.token = 'fixture-network-secret-' * 3

    def network(self, **command):
        code, body = self.h.post('/api/network', command)
        self.assertEqual(code, 200, body)
        self.assertNotIn(self.token, json.dumps(body))
        self.assertNotIn('MUST_NOT_LEAK', json.dumps(body))
        return body

    def provision(self, config=None):
        tools = (config or self.config).parent / 'tools/easytier-2.6.4'
        tools.mkdir(parents=True)
        core = tools / 'easytier-core'
        core.write_text('#!' + sys.executable + '\nimport time\nwhile True: time.sleep(1)\n')
        cli = tools / 'easytier-cli'
        cli.write_text('#!' + sys.executable + '\nprint("[]")\n')
        for program in (core, cli):
            program.chmod(0o700)

    def test_network_api_requires_session_csrf_and_same_origin(self):
        commands = [{'operation': 'status'}, {'operation': 'plan'}, {'operation': 'install'}, {'operation': 'easy_tier_stop'}, {'operation': 'tailscale_login'}, {'operation': 'tailscale_setup_plan'}, {'operation': 'tailscale_setup', 'plan_id': 'unreviewed'}, {'operation': 'tailscale_discovery', 'port': 1234}, {'operation': 'tailscale_discovery', 'ports': [1234, 4321]}, {'operation': 'tailscale_discover', 'address': '100.64.0.2', 'port': 1234}, {'operation': 'tailscale_connect', 'address': '100.64.0.2', 'port': 1234, 'node': 'remote', 'token': 'a' * 64}]
        for command in commands:
            for headers in ({'Content-Type': 'application/json'}, {'Cookie': self.h.cookie, 'Content-Type': 'application/json'}, {**self.h.headers(), 'Origin': 'https://untrusted.example'}):
                with self.subTest(command=command, headers=list(headers)):
                    code, _ = self.h.http('POST', '/api/network', command, headers=headers)
                    self.assertIn(code, (401, 403))
        self.assertFalse(self.config.exists())
        self.assertFalse((self.config.parent / 'tools').exists())

    def test_plan_and_invalid_requests_never_change_configuration(self):
        plan = self.network(operation='plan')
        self.assertEqual(plan['version'], '2.6.4')
        self.assertEqual(plan['sha256'], '61b659eaedba658fa66fe47d17e1426cdd77e5d02fa15fed447bb4357c09dfd6')
        self.assertFalse(plan['requires_admin'] or plan['system_routes'] or plan['system_dns'])
        self.assertTrue(plan['url'].startswith('https://github.com/EasyTier/EasyTier/releases/download/v2.6.4/'))
        self.assertFalse(self.config.exists())
        self.provision()
        for command in [
            {'operation': 'execute', 'program': 'touch /tmp/never'},
            {'operation': 'install', 'url': 'https://untrusted.example/payload'},
            {'operation': 'tailscale_setup_plan', 'program': '/bin/sh'},
            {'operation': 'tailscale_setup', 'plan_id': 'unreviewed'},
            {'operation': 'tailscale_setup', 'plan_id': 'unreviewed', 'url': 'https://untrusted.example/payload'},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peer': 'tcp://user:password@host:11010', 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': 'short', 'peer': 'tcp://host:11010', 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peer': '', 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peers': ['tcp://host:11010', 'tcp://user:password@backup:11010'], 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peers': ['tcp://host:11010', 'tcp://host:11010/'], 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peers': [f'tcp://host:{11010+i}' for i in range(9)], 'listen_port': None},
            {'operation': 'easy_tier_join', 'network': 'home', 'token': self.token, 'peer': 'tcp://host:11010', 'peers': ['tcp://backup:11010'], 'listen_port': None},
            {'operation': 'tailscale_login'},
            {'operation': 'tailscale_discovery', 'port': 0},
            {'operation': 'tailscale_discovery', 'port': 65536},
            {'operation': 'tailscale_discovery', 'port': 1234, 'address': '127.0.0.1'},
            {'operation': 'tailscale_discovery', 'ports': [0]},
            {'operation': 'tailscale_discovery', 'ports': [65536]},
            {'operation': 'tailscale_discovery', 'ports': [1234, 1234]},
            {'operation': 'tailscale_discovery', 'ports': list(range(1, 10))},
            {'operation': 'tailscale_discovery', 'ports': [1234], 'port': 4321},
            {'operation': 'tailscale_discovery', 'ports': [1234], 'address': '127.0.0.1'},
        ]:
            with self.subTest(operation=command['operation']):
                code, _ = self.h.post('/api/network', command)
                self.assertEqual(code, 400)
                self.assertFalse(self.config.exists())

    def test_tailscale_setup_does_not_install_or_change_an_existing_system_service(self):
        plan = self.network(operation='tailscale_setup_plan')
        self.assertFalse(plan['available'])
        self.assertIn('already available', plan['reason'])
        self.assertNotIn('id', plan)
        self.assertEqual(self.network(operation='status')['tailscale']['setup']['phase'], 'idle')
        self.assertFalse((self.config.parent / 'tailscale-setup.json').exists())
        self.assertFalse(self.config.exists())
        if Path('/.dockerenv').exists() or Path('/run/.containerenv').exists():
            (self.root / 'tailscale').write_text('#!' + sys.executable + '\nraise SystemExit(1)\n')
            plan = self.network(operation='tailscale_setup_plan')
            self.assertFalse(plan['available'])
            self.assertIn('container host', plan['reason'])
            self.assertNotIn('id', plan)
            code, _ = self.h.post('/api/network', dict(operation='tailscale_setup', plan_id='unreviewed'))
            self.assertEqual(code, 400)
            self.assertFalse((self.config.parent / 'tailscale-setup.json').exists())

    def test_continuous_tailscale_discovery_persists_without_an_open_view(self):
        calls = self.root / 'status-calls.txt'
        (self.root / 'tailscale').write_text('#!' + sys.executable + '\nfrom pathlib import Path\nimport json\np=Path(' + repr(str(calls)) + ')\nwith p.open("a") as f: f.write("status\\n")\nprint(json.dumps({"BackendState":"Running","PrivateKey":"MUST_NOT_LEAK","Peer":{"unsafe":{"HostName":"not-a-tailnet-address","Online":True,"TailscaleIPs":["127.0.0.1"]},"offline":{"HostName":"offline","Online":False,"TailscaleIPs":["100.64.0.2"]}}}))\n')
        status = self.network(operation='tailscale_discovery', port=1234)
        self.assertTrue(status['tailscale']['discovery']['enabled'])
        until(lambda: self.network(operation='status')['tailscale']['discovery']['phase'] == 'ready')
        discovered = self.network(operation='status')['tailscale']['discovery']
        self.assertEqual(discovered['devices'], [])
        self.assertEqual(discovered['checked'], 0)
        self.assertIsInstance(discovered['observed_at'], int)
        self.assertEqual(self.config.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(self.config.read_text())['tailscale_discovery_port'], 1234)
        self.h.stop(self.h.web)
        before = len(calls.read_text().splitlines())
        self.h.start_web()
        # No network status request after restart: the saved setting drives polling.
        until(lambda: len(calls.read_text().splitlines()) >= before + 2)
        self.h.login()
        restored = self.network(operation='status')['tailscale']['discovery']
        self.assertEqual(restored['port'], 1234)
        self.assertEqual(restored['devices'], [])
        self.assertEqual(self.h.post('/api/peers', {'operation': 'list'})[1]['routes'], [])
        stopped = self.network(operation='tailscale_discovery', port=None)['tailscale']['discovery']
        self.assertEqual(stopped['phase'], 'disabled')
        self.assertFalse(stopped['enabled'])
        self.assertIsNone(stopped['observed_at'])
        self.assertNotIn('tailscale_discovery_port', json.loads(self.config.read_text()))

    def test_multiple_discovery_ports_survive_restart_and_legacy_updates(self):
        ports = [4321, 1234, 65535]
        status = self.network(operation='tailscale_discovery', ports=ports)['tailscale']['discovery']
        self.assertEqual(status['ports'], ports)
        self.assertEqual(status['port'], ports[0])
        saved = json.loads(self.config.read_text())
        self.assertEqual(saved['tailscale_discovery_ports'], ports)
        self.assertNotIn('tailscale_discovery_port', saved)
        before = self.config.read_bytes()
        for command in [dict(ports=[1234, 1234]), dict(ports=[1234], port=4321), dict(ports=[0]), dict(ports=list(range(1, 10)))]:
            code, _ = self.h.post('/api/network', dict(operation='tailscale_discovery', **command))
            self.assertEqual(code, 400)
            self.assertEqual(self.config.read_bytes(), before)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        restored = self.network(operation='status')['tailscale']['discovery']
        self.assertEqual(restored['ports'], ports)
        self.assertTrue(restored['enabled'])
        legacy = self.network(operation='tailscale_discovery', port=2345)['tailscale']['discovery']
        self.assertEqual(legacy['ports'], [2345])
        self.assertNotIn('tailscale_discovery_ports', json.loads(self.config.read_text()))
        disabled = self.network(operation='tailscale_discovery', ports=[])['tailscale']['discovery']
        self.assertFalse(disabled['enabled'])
        self.assertIsNone(disabled['port'])
        self.assertEqual(disabled['ports'], [])
        self.assertNotIn('tailscale_discovery_port', json.loads(self.config.read_text()))
        self.assertNotIn('tailscale_discovery_ports', json.loads(self.config.read_text()))
        self.assertEqual(self.h.post('/api/peers', {'operation': 'list'})[1]['routes'], [])

    def test_tailscale_lookup_rechecks_membership_and_rejects_unlisted_addresses(self):
        status_file = self.root / 'tailscale-state.json'
        state = {'BackendState': 'Running', 'PrivateKey': 'MUST_NOT_LEAK', 'Peer': {
            'online': {'HostName': 'Online laptop', 'Online': True, 'TailscaleIPs': ['100.64.0.2', 'fd7a:115c:a1e0::2']},
            'offline': {'HostName': 'Offline laptop', 'Online': False, 'TailscaleIPs': ['100.64.0.3']},
            'unsafe': {'HostName': 'Invalid address', 'Online': True, 'TailscaleIPs': ['127.0.0.1']},
        }}
        status_file.write_text(json.dumps(state))
        (self.root / 'tailscale').write_text('#!' + sys.executable + '\nfrom pathlib import Path\nprint(Path(' + repr(str(status_file)) + ').read_text())\n')
        until(lambda: self.network(operation='status')['tailscale']['state'] == 'Running')
        for address in ['100.64.0.3', '100.64.0.4', '127.0.0.1', '169.254.169.254', '::ffff:100.64.0.2']:
            with self.subTest(address=address):
                for command in [dict(operation='tailscale_discover', address=address, port=1234),
                                dict(operation='tailscale_connect', address=address, port=1234, node='remote', token='b' * 64)]:
                    code, body = self.h.post('/api/network', command)
                    self.assertEqual(code, 400, body)
                    self.assertNotIn('MUST_NOT_LEAK', json.dumps(body))
        # The next lookup must use fresh system status, not the view's earlier online snapshot.
        state['Peer']['online']['Online'] = False
        status_file.write_text(json.dumps(state))
        code, body = self.h.post('/api/network', dict(operation='tailscale_discover', address='100.64.0.2', port=1234))
        self.assertEqual(code, 400)
        self.assertIn('no longer an online Tailscale peer', body['error'])
        catalog = self.h.post('/api/peers', {'operation': 'list'})[1]
        self.assertEqual(catalog['routes'], [])
        self.assertFalse(self.config.exists())
        self.assertFalse(list(self.config.parent.glob('.route-*.token')))

    def test_multiple_entries_are_private_persisted_and_restored(self):
        self.provision()
        peers = ['tcp://127.0.0.1:11010', 'wss://backup.example.com/mesh']
        joined = self.network(operation='easy_tier_join', network='multi-entry', token=self.token, peers=peers, listen_port=None)
        self.assertEqual(joined['easytier']['peers'], peers)
        self.assertEqual(joined['easytier']['peer'], peers[0])
        until(lambda: self.network(operation='status')['easytier']['phase'] == 'running')
        saved = json.loads(self.config.read_text())
        self.assertEqual(saved['easytier']['peers'], peers)
        self.assertNotIn('peer', saved['easytier'])
        generated = self.config.with_name('easytier.toml')
        self.assertEqual(generated.read_text().count('[[peer]]'), 2)
        for address in peers:
            self.assertIn('uri=' + json.dumps(address), generated.read_text())
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        until(lambda: self.network(operation='status')['easytier']['phase'] == 'running')
        self.assertEqual(self.network(operation='status')['easytier']['peers'], peers)
        self.assertEqual(generated.read_text().count('[[peer]]'), 2)
        self.assertEqual(json.loads(self.config.read_text()), saved)

    def test_private_state_restores_after_web_restart_and_explicit_disconnect(self):
        self.provision()
        self.network(operation='easy_tier_join', network='api-test', token=self.token, peer='tcp://127.0.0.1:11010', listen_port=None)
        until(lambda: self.network(operation='status')['easytier']['phase'] == 'running')
        self.assertEqual(self.config.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(self.config.read_text())['easytier']['token'], self.token)
        generated = self.config.with_name('easytier.toml')
        self.assertEqual(generated.stat().st_mode & 0o777, 0o600)
        text = generated.read_text()
        for required in ('no_tun=true', 'use_smoltcp=true', 'accept_dns=false', 'private_mode=true'):
            self.assertIn(required, text.replace(' ', ''))
        self.h.stop(self.h.web)
        until(lambda: not generated.exists())
        self.h.start_web()
        self.h.login()
        until(lambda: self.network(operation='status')['easytier']['phase'] == 'running')
        stopped = self.network(operation='easy_tier_stop')
        self.assertFalse(stopped['easytier']['enabled'])
        self.assertEqual(stopped['easytier']['phase'], 'stopped')
        self.assertIsNone(json.loads(self.config.read_text())['easytier'])
        self.assertFalse(generated.exists())
        self.assertTrue((generated.parent / 'tools/easytier-2.6.4/easytier-core').exists())

    def test_eight_long_entries_fit_the_bounded_network_request(self):
        self.provision()
        peers = [f'wss://entry{i}.example.com/' + 'a' * 2020 for i in range(8)]
        self.assertTrue(all(len(p) <= 2048 for p in peers))
        command = dict(operation='easy_tier_join', network='many-entries', token=self.token, peers=peers, listen_port=None)
        self.assertGreater(len(json.dumps(command)), 16384)
        self.assertEqual(self.network(**command)['easytier']['peers'], peers)

    def test_invitation_enrolls_a_second_workspace_without_granting_capabilities(self):
        self.provision()
        entries = ['tcp://entry.example.com:11010', 'wss://backup.example.com/mesh']
        self.network(operation='easy_tier_join', network='invited-home', token=self.token, peers=entries, listen_port=None)
        issued = self.network(operation='easy_tier_invite', peers=entries)['invitation']
        self.assertTrue(issued['token'].startswith('cbi_'))
        private = self.config.with_name('network-invitations.json')
        self.assertEqual(private.stat().st_mode & 0o777, 0o600)
        self.assertNotIn(issued['token'], private.read_text())
        self.assertNotIn(self.token, private.read_text())
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        receiver = Harness()
        self.addCleanup(receiver.close)
        receiver_config = receiver.root / '.config/cranibox-node/networks.json'
        command = {'operation': 'easy_tier_enroll', 'url': f'http://127.0.0.1:{self.h.web_port}', 'token': issued['token'], 'listen_port': None}
        code, body = receiver.post('/api/network', command)
        self.assertEqual(code, 400, body)
        self.assertIn('Install EasyTier first', body['error'])
        self.assertEqual(self.network(operation='status')['easytier']['invitations'][0]['id'], issued['id'])
        self.provision(receiver_config)
        for changes in [{'url': 'http://example.com'}, {'url': 'https://user:secret@example.com'}, {'url': command['url'] + '/?token=secret'}, {'listen_port': 1}]:
            code, body = receiver.post('/api/network', {**command, **changes})
            self.assertEqual(code, 400, body)
            self.assertFalse(receiver_config.exists())
            self.assertEqual(len(self.network(operation='status')['easytier']['invitations']), 1)
        code, body = receiver.post('/api/network', command)
        self.assertEqual(code, 200, body)
        self.assertEqual(body['easytier']['network'], 'invited-home')
        self.assertEqual(body['easytier']['peers'], entries)
        self.assertNotIn(self.token, json.dumps(body))
        self.assertNotIn(issued['token'], json.dumps(body))
        self.assertEqual(json.loads(receiver_config.read_text())['easytier']['token'], self.token)
        self.assertEqual(receiver_config.stat().st_mode & 0o777, 0o600)
        claimed = self.network(operation='status')['easytier']['invitations']
        self.assertEqual(len(claimed), 1)
        self.assertEqual(claimed[0]['id'], issued['id'])
        self.assertIsInstance(claimed[0]['redeemed_at'], int)
        self.assertFalse(receiver_config.with_name('network-enrollment.json').exists())
        for harness in (self.h, receiver):
            peers = json.loads((harness.root / '.config/cranibox-node/peers.json').read_text())
            self.assertFalse(peers['grants'])
            self.assertFalse(peers['routes'])
        code, body = self.h.http('POST', '/api/network/bootstrap', {}, headers={'Authorization': 'Bearer ' + issued['token'], 'Content-Type': 'application/json'})
        self.assertEqual(code, 401, body)
        receiver.stop(receiver.web)
        receiver.start_web()
        receiver.login()
        self.assertEqual(receiver.post('/api/network', {'operation': 'status'})[1]['easytier']['peers'], entries)

    def test_bootstrap_is_narrow_bounded_and_revocable(self):
        self.provision()
        entries = ['tcp://entry.example.com:11010']
        self.network(operation='easy_tier_join', network='home', token=self.token, peers=entries, listen_port=None)
        issued = self.network(operation='easy_tier_invite', peers=entries)['invitation']
        headers = {'Authorization': 'Bearer ' + issued['token'], 'Content-Type': 'application/json'}
        for method, path, body, supplied, expected in [
            ('GET', '/api/network/bootstrap', None, headers, 405),
            ('POST', '/api/network/bootstrap', {}, {'Content-Type': 'application/json'}, 401),
            ('POST', '/api/network/bootstrap', {}, {**headers, 'Authorization': 'Bearer ' + self.token}, 401),
            ('POST', '/api/network/bootstrap?token=secret', {}, headers, 400),
            ('POST', '/api/network/bootstrap', {'operation': 'install'}, headers, 400),
            ('POST', '/api/network/bootstrap', {'padding': 'x' * 300}, headers, 413),
            ('POST', '/api/network/bootstrap', {'recovery': 'x' * 64}, headers, 400),
            ('POST', '/api/network/bootstrap', {'recovery': 'a' * 63}, headers, 400),
            ('POST', '/api/network/bootstrap', {'recovery': 123}, headers, 400),
            ('POST', '/api/network/bootstrap', {}, {**headers, 'Origin': 'https://untrusted.example'}, 403),
            ('POST', '/api/network/bootstrap', {}, {**headers, 'Host': 'untrusted.example'}, 403),
        ]:
            with self.subTest(method=method, supplied=list(supplied), expected=expected):
                code, body = self.h.http(method, path, body, headers=supplied)
                self.assertEqual(code, expected, body)
                self.assertEqual(self.h.last_headers.get('cache-control'), 'no-store')
                self.assertEqual(len(self.network(operation='status')['easytier']['invitations']), 1)
        self.network(operation='easy_tier_revoke_invitation', id=issued['id'])
        code, body = self.h.http('POST', '/api/network/bootstrap', {}, headers=headers)
        self.assertEqual(code, 401, body)
        old = self.network(operation='easy_tier_invite', peers=entries)['invitation']
        self.network(operation='easy_tier_stop')
        self.network(operation='easy_tier_join', network='home', token=self.token, peers=entries, listen_port=None)
        code, body = self.h.http('POST', '/api/network/bootstrap', {}, headers={**headers, 'Authorization': 'Bearer ' + old['token']})
        self.assertEqual(code, 401, body)

    def test_lost_invitation_response_recovers_after_both_workspaces_restart(self):
        self.provision()
        entries = ['tcp://entry.example.com:11010']
        self.network(operation='easy_tier_join', network='recover-home', token=self.token, peers=entries, listen_port=None)
        issued = self.network(operation='easy_tier_invite', peers=entries)['invitation']
        receiver = Harness()
        self.addCleanup(receiver.close)
        receiver_config = receiver.root / '.config/cranibox-node/networks.json'
        self.provision(receiver_config)
        upstream = f'http://127.0.0.1:{self.h.web_port}/api/network/bootstrap'
        claims = []

        class Proxy(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                data = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                claims.append(json.loads(data))
                request = urllib.request.Request(upstream, data=data, headers={'Authorization': self.headers['Authorization'], 'Content-Type': 'application/json'})
                with urllib.request.urlopen(request, timeout=10) as response:
                    offer = response.read()
                if self.server.drop:
                    self.connection.shutdown(socket.SHUT_RDWR)
                    self.connection.close()
                    return
                if self.server.block_save:
                    receiver_config.mkdir()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(offer)))
                self.end_headers()
                self.wfile.write(offer)

        proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        proxy.drop = True
        proxy.block_save = False
        thread = threading.Thread(target=proxy.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
        thread.start()
        self.addCleanup(proxy.server_close)
        self.addCleanup(lambda: thread.join(timeout=2))
        self.addCleanup(proxy.shutdown)
        command = {'operation': 'easy_tier_enroll', 'url': f'http://127.0.0.1:{proxy.server_port}', 'token': issued['token'], 'listen_port': None}
        code, body = receiver.post('/api/network', command)
        self.assertEqual(code, 400, body)
        self.assertIn('same address and invitation', body['error'])
        self.assertFalse(receiver_config.exists())
        pending = receiver_config.with_name('network-enrollment.json')
        recovery = json.loads(pending.read_text())['recovery']
        self.assertEqual(pending.stat().st_mode & 0o777, 0o600)
        self.assertEqual(claims, [{'recovery': recovery}])
        claimed = self.network(operation='status')['easytier']['invitations'][0]
        private = self.config.with_name('network-invitations.json').read_text()
        for secret in (self.token, issued['token'], recovery):
            self.assertNotIn(secret, private)
        headers = {'Authorization': 'Bearer ' + issued['token'], 'Content-Type': 'application/json'}
        for claim in ({}, {'recovery': 'a' * 64}):
            code, body = self.h.http('POST', '/api/network/bootstrap', claim, headers=headers)
            self.assertEqual(code, 401, body)
        for harness in (self.h, receiver):
            harness.stop(harness.web)
            harness.start_web()
            harness.login()
        proxy.drop = False
        proxy.block_save = True
        code, body = receiver.post('/api/network', command)
        self.assertEqual(code, 400, body)
        self.assertTrue(receiver_config.is_dir())
        self.assertEqual(json.loads(pending.read_text())['recovery'], recovery)
        self.assertEqual(self.network(operation='status')['easytier']['invitations'][0], claimed)
        receiver_config.rmdir()
        receiver.stop(receiver.web)
        receiver.start_web()
        receiver.login()
        proxy.block_save = False
        code, body = receiver.post('/api/network', command)
        self.assertEqual(code, 200, body)
        self.assertEqual(body['easytier']['network'], 'recover-home')
        self.assertEqual(body['easytier']['peers'], entries)
        self.assertEqual(claims, [{'recovery': recovery}] * 3)
        self.assertEqual(self.network(operation='status')['easytier']['invitations'][0], claimed)
        self.assertFalse(pending.exists())
        self.assertEqual(json.loads(receiver_config.read_text())['easytier']['token'], self.token)
        self.network(operation='easy_tier_revoke_invitation', id=issued['id'])
        code, body = self.h.http('POST', '/api/network/bootstrap', {'recovery': recovery}, headers=headers)
        self.assertEqual(code, 401, body)
        self.assertEqual(self.network(operation='status')['easytier']['invitations'], [])

    def test_corrupt_client_recovery_is_preserved_without_consuming_invitation(self):
        self.provision()
        entries = ['tcp://entry.example.com:11010']
        self.network(operation='easy_tier_join', network='home', token=self.token, peers=entries, listen_port=None)
        issued = self.network(operation='easy_tier_invite', peers=entries)['invitation']
        receiver = Harness()
        self.addCleanup(receiver.close)
        receiver_config = receiver.root / '.config/cranibox-node/networks.json'
        self.provision(receiver_config)
        pending = receiver_config.with_name('network-enrollment.json')
        pending.write_text('preserve corrupt recovery')
        code, body = receiver.post('/api/network', {'operation': 'easy_tier_enroll', 'url': f'http://127.0.0.1:{self.h.web_port}', 'token': issued['token'], 'listen_port': None})
        self.assertEqual(code, 400, body)
        self.assertIn('network-enrollment.json', body['error'])
        self.assertEqual(pending.read_text(), 'preserve corrupt recovery')
        self.assertFalse(receiver_config.exists())
        self.assertNotIn('redeemed_at', self.network(operation='status')['easytier']['invitations'][0])

    def test_enrollment_does_not_follow_redirects_or_accept_unbounded_responses(self):
        self.provision()
        requests = []
        invitation = 'cbi_' + 'd' * 64

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                requests.append((self.path, self.headers.get('Authorization')))
                self.rfile.read(int(self.headers.get('Content-Length', 0)))
                if self.server.mode == 'redirect':
                    self.send_response(307)
                    self.send_header('Location', '/redirected')
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                self.send_response(400 if self.server.mode == 'legacy' else 200)
                self.send_header('Content-Type', 'application/json')
                data = b'{' + b'x' * 40000 if self.server.mode in ('sized', 'streamed') else b'{"invalid":"response"}'
                if self.server.mode != 'streamed':
                    self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(lambda: thread.join(timeout=2))
        self.addCleanup(server.shutdown)
        for mode, expected in [('redirect', 'HTTP 307'), ('sized', 'too large'), ('streamed', 'too large'), ('invalid', 'invalid network invitation'), ('legacy', 'Upgrade the inviting Workspace')]:
            server.mode = mode
            code, body = self.h.post('/api/network', {'operation': 'easy_tier_enroll', 'url': f'http://127.0.0.1:{server.server_port}', 'token': invitation, 'listen_port': None})
            self.assertEqual(code, 400, body)
            self.assertIn(expected, body['error'])
            self.assertNotIn(invitation, json.dumps(body))
            self.assertFalse(self.config.exists())
        self.assertEqual(requests, [('/api/network/bootstrap', 'Bearer ' + invitation)] * 5)

    def test_corrupt_saved_state_is_preserved(self):
        self.h.stop(self.h.web)
        self.config.write_text('corrupt state to preserve')
        self.h.start_web()
        self.h.login()
        status = self.network(operation='status')
        self.assertEqual(status['easytier']['phase'], 'failed')
        code, _ = self.h.post('/api/network', {'operation': 'easy_tier_stop'})
        self.assertEqual(code, 400)
        self.assertEqual(self.config.read_text(), 'corrupt state to preserve')

    def test_headless_peer_restores_and_cleans_up_without_workspace(self):
        self.provision()
        self.network(operation='easy_tier_join', network='api-test', token=self.token, peer='tcp://127.0.0.1:11010', listen_port=None)
        until(lambda: self.network(operation='status')['easytier']['phase'] == 'running')
        self.assertIsNone(json.loads(self.config.with_name('peers.json').read_text())['listener'])
        generated = self.config.with_name('easytier.toml')
        self.h.stop(self.h.web)
        self.h.stop(self.h.server)
        self.assertFalse(generated.exists())
        peer = self.h.start(NODE, '--config', self.config.with_name('settings.json'), 'peer', 'serve')
        until(generated.exists)
        self.assertIsNone(peer.poll())
        self.h.stop(peer)
        self.assertFalse(generated.exists())
        self.assertIsNotNone(json.loads(self.config.read_text())['easytier'])

    def test_occupied_saved_port_never_receives_target_credentials(self):
        self.h.stop(self.h.web)
        peers = self.config.with_name('peers.json')
        saved = json.loads(peers.read_text())
        credential = self.config.with_name('test-target.token')
        credential.write_text('c' * 64)
        with socket.socket() as foreign:
            foreign.bind(('127.0.0.1', 0))
            foreign.listen()
            port = foreign.getsockname()[1]
            saved['routes']['offline-target'] = {'url': f'http://127.0.0.1:{port}', 'token_file': str(credential), 'allow_insecure': False}
            peers.write_text(json.dumps(saved))
            self.config.write_text(json.dumps({'version': 1, 'easytier': None, 'forwards': {'offline-target': port}}))
            self.h.start_web()
            self.h.login()
            status = self.network(operation='status')
            self.assertIn('unavailable', status['easytier']['error'])
            self.assertEqual(status['easytier']['phase'], 'failed')
            code, body = self.h.post('/api/peers', {'operation': 'probe', 'node': 'offline-target'})
            self.assertEqual(code, 400)
            self.assertIn('local network forward', body['error'])
            foreign.settimeout(.2)
            with self.assertRaises(TimeoutError):
                foreign.accept()
