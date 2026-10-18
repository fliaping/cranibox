"""Durable device mailboxes over real direct HTTP, CLI and MCP, without a relay."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import select
import subprocess
import time
import threading
import unittest
import uuid

from workspace_web_harness import Harness, CORE, NODE, RUNTIME, WEB, free_port, sockets_allowed, until


@unittest.skipUnless(os.name == 'posix' and all(p.exists() for p in (CORE, NODE, RUNTIME, WEB)) and sockets_allowed(), 'build Rust binaries; real sockets required')
class DeviceMessagesTests(unittest.TestCase):
    def setUp(self):
        self.h = Harness()
        self.addCleanup(self.h.close)
        self.caller = self.api(operation='identity')['node_id']
        self.config = self.h.root / 'target/settings.json'
        self.target = self.cli('identity')['node_id']
        self.token_file = self.h.root / 'messages.token'
        self.cli('grant', '--node', self.caller, '--output-token', self.token_file, '--messages', '--inbound', 'accept')
        self.token = self.token_file.read_text().strip()
        self.port = free_port()
        self.url = f'http://127.0.0.1:{self.port}'
        self.cli('listen', '--address', f'127.0.0.1:{self.port}')
        self.start_target()
        self.api(operation='connect', node=self.target, url=self.url, token=self.token)
        self.path = self.h.root / '.config/cranibox-node/device-messages.json'

    def cli(self, *args):
        result = subprocess.run([str(NODE), '--config', str(self.config), 'peer', *map(str, args)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def api(self, **command):
        status, value = self.h.post('/api/peers', command)
        self.assertEqual(status, 200, value)
        if hasattr(self, 'token'):
            self.assertNotIn(self.token, json.dumps(value))
        return value

    def mailbox(self, **command):
        return self.api(operation='messages', command=command)

    def start_target(self):
        self.process = self.h.start(NODE, '--config', self.config, 'peer', 'serve')
        until(lambda: self.direct('GET', '/api/v1/mesh/nodes')[0] == 200)

    def direct(self, method, path, body=None, headers=None):
        return self.h.http(method, path, body, port=self.port, headers={
            'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json',
            'X-Cranibox-Target': self.target, **(headers or {})})

    def send(self, text='Please review the project. 不要自动执行。', **extra):
        message_id = str(uuid.uuid4())
        view = self.mailbox(operation='send', id=message_id, node=self.target, text=text, **extra)
        return message_id, next(item for item in view['outgoing'] if item['message']['id'] == message_id)

    def outgoing(self, message_id):
        return next(item for item in self.mailbox(operation='list')['outgoing'] if item['message']['id'] == message_id)

    def delivered(self, message_id, status='received'):
        return until(lambda: item if (item := self.outgoing(message_id))['status'] == status else None)

    def envelope(self):
        created = int(time.time())
        message_id = str(uuid.uuid4())
        return dict(id=message_id, thread=message_id, in_reply_to=None, sender=self.caller,
                    recipient=self.target, text='Untrusted plain text', created_at=created, expires_at=created+7*24*3600)

    def test_offline_queue_survives_restart_and_acknowledgements_are_explicit(self):
        self.h.stop(self.h.server)
        self.h.stop(self.process)
        message_id, queued = self.send()
        self.assertEqual(queued['status'], 'queued')
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        until(lambda: self.outgoing(message_id)['attempts'] > 0)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.outgoing(message_id)['message'], queued['message'])
        self.start_target()
        self.mailbox(operation='retry', id=message_id)
        self.delivered(message_id)
        inbox = self.cli('messages', 'list')['incoming']
        self.assertEqual(len(inbox), 1)
        self.assertEqual(inbox[0]['message'], queued['message'])
        self.cli('messages', 'acknowledge', message_id)
        self.mailbox(operation='retry', id=message_id)
        self.delivered(message_id, 'acknowledged')
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(self.cli('messages', 'list')['incoming'][0]['status'], 'acknowledged')
        self.assertEqual(self.path.with_name('peers.json').stat().st_mode & 0o777, 0o600)
        self.assertNotIn(self.token, self.path.read_text())

    def test_local_idempotency_and_remote_dedup_survive_archiving(self):
        message_id, first = self.send('Exactly one local submission')
        same = self.mailbox(operation='send', id=message_id, node=self.target, text=first['message']['text'])
        self.assertEqual(len(same['outgoing']), 1)
        self.assertEqual(same['outgoing'][0]['message'], first['message'])
        status, _ = self.h.post('/api/peers', {'operation':'messages', 'command':{
            'operation':'send', 'id':message_id, 'node':self.target, 'text':'Different content'}})
        self.assertEqual(status, 400)
        self.delivered(message_id)
        path = f'/api/v1/mesh/nodes/{self.target}/messages'
        self.assertEqual(self.direct('POST', path, first['message'])[0], 200)
        self.assertEqual(len(self.cli('messages', 'list')['incoming']), 1)
        self.cli('messages', 'archive', message_id)
        self.assertEqual(self.cli('messages', 'list')['incoming'], [])
        self.assertEqual(self.direct('POST', path, first['message'])[1]['status'], 'received')
        changed = {**first['message'], 'text':'Changed payload'}
        self.assertEqual(self.direct('POST', path, changed)[0], 400)
        self.assertEqual(self.cli('messages', 'list')['incoming'], [])

    def test_message_permission_is_independent_and_cannot_execute_or_read_other_capabilities(self):
        caps = self.direct('GET', '/api/v1/mesh/nodes')[1]['nodes'][0]
        self.assertTrue(caps['messages'])
        self.assertFalse(caps['exec'])
        self.assertFalse(caps['terminal_sessions'])
        base = f'/api/v1/mesh/nodes/{self.target}'
        marker = self.h.root / 'must-not-execute'
        self.assertEqual(self.direct('POST', base+'/exec', {'program':'touch', 'args':[str(marker)], 'timeout_secs':1})[0], 403)
        self.assertEqual(self.direct('POST', base+'/projects', {'operation':'list'})[0], 403)
        self.assertFalse(marker.exists())
        message_id, _ = self.send('touch ' + str(marker))
        self.delivered(message_id)
        self.assertFalse(marker.exists())
        self.cli('revoke', '--node', self.caller)
        other = self.h.root / 'exec-only.token'
        self.cli('grant', '--node', self.caller, '--output-token', other, '--exec')
        self.token = other.read_text().strip()
        self.assertEqual(self.direct('POST', base+'/messages', self.envelope())[0], 403)
        self.assertEqual(self.direct('GET', base+'/messages/'+message_id)[0], 403)

    def test_remote_auth_identity_and_payload_limits_are_checked_before_persisting(self):
        path = f'/api/v1/mesh/nodes/{self.target}/messages'
        message = self.envelope()
        self.assertEqual(self.direct('POST', path, message, {'Authorization':''})[0], 401)
        self.assertEqual(self.direct('POST', path, message, {'Origin':'https://untrusted.example'})[0], 403)
        self.assertEqual(self.direct('POST', path, message, {'X-Cranibox-Target':'wrong-node'})[0], 409)
        self.assertEqual(self.direct('POST', path, {**message, 'sender':'spoofed'})[0], 400)
        self.assertEqual(self.direct('POST', path, {**message, 'recipient':'wrong-target'})[0], 400)
        for text in ['', '🙂'*2049, '\x1b[2J']:
            self.assertEqual(self.direct('POST', path, {**message, 'text':text})[0], 400)
        self.assertEqual(self.direct('POST', path, {**message, 'expires_at':message['created_at']+1})[0], 400)
        self.assertEqual(self.cli('messages', 'list')['incoming'], [])
        self.assertFalse(self.config.with_name('device-messages.json').exists())
        self.assertEqual(self.direct('POST', path, message)[0], 200)
        other = self.h.root / 'other-sender.token'
        self.cli('grant', '--node', 'other-sender', '--output-token', other, '--messages', '--inbound', 'accept')
        self.assertEqual(self.direct('GET', path+'/'+message['id'], headers={'Authorization':'Bearer '+other.read_text().strip()})[0], 400)

    def test_changed_credentials_block_queued_messages_instead_of_sending_to_new_route(self):
        self.h.stop(self.process)
        message_id, _ = self.send('Do not send with replacement credentials')
        self.api(operation='connect', node=self.target, url=self.url, token='a'*64)
        until(lambda: self.outgoing(message_id)['status'] == 'blocked')
        status, _ = self.h.post('/api/peers', {'operation':'messages','command':{'operation':'retry','id':message_id}})
        self.assertEqual(status, 400)
        self.start_target()
        self.assertEqual(self.cli('messages', 'list')['incoming'], [])
        self.mailbox(operation='archive', id=message_id)
        self.assertEqual(self.mailbox(operation='list')['outgoing'], [])

    def test_bidirectional_threads_work_without_a_gateway_and_do_not_implicitly_acknowledge(self):
        self.h.stop(self.h.server)
        source_session, target_session = str(uuid.uuid4()), str(uuid.uuid4())
        message_id, original = self.send('Can you review this?', sender_session=source_session, target_session=target_session)
        self.delivered(message_id)
        grant = self.api(operation='grant', node=self.target, permissions={
            'exec':False, 'terminals':False, 'projects':False, 'desktop':False, 'messages':True}, inbound='accept')
        listener = self.api(operation='listener', listener={'address':'127.0.0.1:0','allow_insecure':False})
        address = listener['listener_runtime']['address']
        reverse = self.h.root / 'reverse.token'
        reverse.write_text(grant['token'])
        self.cli('connect', '--node', self.caller, '--url', 'http://'+address, '--credential-file', reverse)
        reply_id = str(uuid.uuid4())
        sent = self.cli('messages', 'send', '--message-id', reply_id, '--node', self.caller,
                        '--text', 'Reviewed; please check the patch.', '--thread', original['message']['thread'], '--in-reply-to', message_id,
                        '--sender-session', target_session, '--target-session', source_session)
        self.assertEqual(sent['outgoing'][0]['message']['id'], reply_id)
        inbox = until(lambda: view['incoming'] if (view := self.mailbox(operation='list'))['incoming'] else None)
        self.assertEqual(inbox[0]['message']['sender_session'], target_session)
        self.assertEqual(inbox[0]['message']['target_session'], source_session)
        self.assertEqual(len(self.cli('messages', 'list', '--session', target_session)['outgoing']), 1)
        self.assertEqual(self.cli('messages', 'list', '--session', source_session)['outgoing'], [])
        self.assertEqual(inbox[0]['message']['thread'], original['message']['thread'])
        self.assertEqual(inbox[0]['message']['in_reply_to'], message_id)
        self.assertEqual(self.outgoing(message_id)['status'], 'received')
        self.mailbox(operation='acknowledge', id=reply_id)
        self.cli('messages', 'retry', reply_id)
        until(lambda: self.cli('messages','list')['outgoing'][0]['status'] == 'acknowledged')

    def test_corrupt_mailbox_is_preserved_and_invalid_mutations_do_not_reset_it(self):
        self.send()
        self.h.stop(self.h.web)
        self.path.write_text('broken mailbox - preserve this')
        self.h.start_web()
        self.h.login()
        for command in [{'operation':'list'}, {'operation':'send','node':self.target,'text':'New content'}, {'operation':'archive','id':str(uuid.uuid4())}]:
            status, _ = self.h.post('/api/peers', {'operation':'messages','command':command})
            self.assertEqual(status, 400)
            self.assertEqual(self.path.read_text(), 'broken mailbox - preserve this')

    def test_mailbox_mcp_uses_local_identity_without_relay_settings_or_exec_tools(self):
        self.assertFalse(self.config.exists())
        self.h.stop(self.h.server)
        message_id = str(uuid.uuid4())
        requests = [
            {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18'}},
            {'jsonrpc':'2.0','method':'notifications/initialized'},
            {'jsonrpc':'2.0','id':2,'method':'tools/list'},
            {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'device_messages','arguments':{'operation':'list'}}},
            {'jsonrpc':'2.0','id':4,'method':'tools/call','params':{'name':'device_messages','arguments':{'operation':'send','id':message_id,'node':self.caller,'text':'Agent can leave a durable message'}}},
            {'jsonrpc':'2.0','id':5,'method':'tools/call','params':{'name':'workspace_exec','arguments':{}}},
            {'jsonrpc':'2.0','id':6,'method':'tools/call','params':{'name':'device_messages','arguments':{'operation':'send','node':self.caller,'text':'Missing stable message ID'}}},
        ]
        # A missing reverse route is a tool error, not a request to execute or use the gateway.
        result = subprocess.run([str(NODE),'--config',str(self.config),'peer','messages','mcp'],
                                input='\n'.join(json.dumps(r) for r in requests)+'\n', capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        responses = {value['id']:value for line in result.stdout.splitlines() if (value := json.loads(line))}
        self.assertEqual(responses[1]['result']['serverInfo']['name'], 'cranibox-device-messages')
        self.assertEqual([tool['name'] for tool in responses[2]['result']['tools']], ['device_messages', 'device_tasks', 'device_agent_sessions'])
        schema = responses[2]['result']['tools'][0]['inputSchema']
        self.assertEqual(schema['type'], 'object')
        self.assertEqual(schema['oneOf'][0]['properties']['session']['format'], 'uuid')
        self.assertEqual(schema['oneOf'][1]['properties']['target_session']['format'], 'uuid')
        self.assertEqual(schema['oneOf'][1]['properties']['sender_session']['format'], 'uuid')
        self.assertEqual(json.loads(responses[3]['result']['content'][0]['text'])['owner'], self.target)
        self.assertTrue(responses[4]['result']['isError'])
        self.assertTrue(responses[5]['result']['isError'])
        self.assertTrue(responses[6]['result']['isError'])
        self.assertIn('stable message ID', responses[6]['result']['content'][0]['text'])
        self.assertNotIn(self.token, result.stdout+result.stderr)
        self.assertFalse(self.config.exists())


    def test_standalone_mcp_delivers_and_checks_acknowledgements_without_a_node_daemon(self):
        self.h.stop(self.h.server)
        grant = self.api(operation='grant', node=self.target, permissions={
            'exec':False, 'terminals':False, 'projects':False, 'desktop':False, 'messages':True}, inbound='accept')
        listener = self.api(operation='listener', listener={'address':'127.0.0.1:0','allow_insecure':False})
        reverse = self.h.root / 'mcp-reverse.token'
        reverse.write_text(grant['token'])
        self.cli('connect', '--node', self.caller, '--url', 'http://'+listener['listener_runtime']['address'],
                 '--credential-file', reverse)
        self.h.stop(self.process)
        process = subprocess.Popen([str(NODE), '--config', str(self.config), 'peer', 'messages', 'mcp'],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        def close_mcp():
            if process.poll() is None:
                process.kill()
                process.wait(timeout=8)
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()
        self.addCleanup(close_mcp)
        sequence = 0

        def call(command):
            nonlocal sequence
            sequence += 1
            request = {'jsonrpc':'2.0','id':sequence,'method':'tools/call',
                       'params':{'name':'device_messages','arguments':command}}
            process.stdin.write(json.dumps(request)+'\n')
            process.stdin.flush()
            self.assertTrue(select.select([process.stdout], [], [], 8)[0], 'MCP response timed out')
            response = json.loads(process.stdout.readline())
            self.assertEqual(response['id'], sequence)
            self.assertNotIn('error', response)
            self.assertFalse(response['result'].get('isError'), response)
            return json.loads(response['result']['content'][0]['text'])

        message_id = str(uuid.uuid4())
        source_session, target_session = str(uuid.uuid4()), str(uuid.uuid4())
        command = {'operation':'send','id':message_id,'node':self.caller,'text':'Sent by an independent Agent MCP', 'sender_session':source_session, 'target_session':target_session}
        self.assertEqual(call(command)['outgoing'][0]['message']['id'], message_id)
        incoming = until(lambda: view['incoming'] if (view := self.mailbox(operation='list'))['incoming'] else None)
        self.assertEqual(incoming[0]['message']['id'], message_id)
        self.assertEqual(incoming[0]['message']['target_session'], target_session)
        self.assertEqual(len(call({'operation':'list','session':source_session})['outgoing']), 1)
        self.assertEqual(call({'operation':'list','session':target_session})['outgoing'], [])
        call(command)
        self.assertEqual(len(self.mailbox(operation='list')['incoming']), 1)
        self.mailbox(operation='acknowledge', id=message_id)
        call({'operation':'retry','id':message_id})
        until(lambda: call({'operation':'list'})['outgoing'][0]['status'] == 'acknowledged')
        process.stdin.close()
        self.assertEqual(process.wait(timeout=8), 0, process.stderr.read())
        self.assertFalse(self.config.exists())


    def test_session_addressed_queues_survive_restart_and_filters_do_not_retarget_messages(self):
        source, target, other = str(uuid.uuid4()), str(uuid.uuid4()), str(uuid.uuid4())
        self.h.stop(self.process)
        first, addressed = self.send('For the first Agent', sender_session=source, target_session=target)
        second, _ = self.send('For another Agent', sender_session=source, target_session=other)
        third, _ = self.send('Device-wide message')
        filtered = self.mailbox(operation='list', session=source)
        self.assertEqual(len(filtered['outgoing']), 2)
        self.assertEqual(filtered['used'], 3)
        self.assertEqual(self.mailbox(operation='list', session=target)['outgoing'], [])
        for change in [{'sender_session':other}, {'target_session':other}]:
            status, _ = self.h.post('/api/peers', {'operation':'messages', 'command':{
                'operation':'send', 'id':first, 'node':self.target, 'text':addressed['message']['text'],
                'sender_session':source, 'target_session':target, **change}})
            self.assertEqual(status, 400)
        self.h.stop(self.h.web)
        self.h.start_web()
        self.h.login()
        self.assertEqual(self.outgoing(first)['message'], addressed['message'])
        self.start_target()
        for message_id in [first, second, third]:
            self.mailbox(operation='retry', id=message_id)
            self.delivered(message_id)
        inbox = self.cli('messages', 'list', '--session', target)
        self.assertEqual(inbox['session_filter'], target)
        self.assertEqual(len(inbox['incoming']), 1)
        self.assertEqual(inbox['incoming'][0]['message'], addressed['message'])
        self.assertEqual(len(self.cli('messages', 'list')['incoming']), 3)
        self.assertEqual(self.cli('messages', 'list', '--session', source)['incoming'], [])
        self.h.stop(self.process)
        self.start_target()
        self.assertEqual(len(self.cli('messages', 'list', '--session', other)['incoming']), 1)
        # Filters are local routing labels. No active registration or isolated mailbox is implied.
        self.assertEqual(self.cli('agent-sessions')['sessions'], [])
        for action in [{'operation':'list', 'session':str(uuid.UUID(int=0))},
                       {'operation':'list', 'unexpected':True},
                       {'operation':'send', 'node':self.target, 'text':'Invalid address', 'target_session':str(uuid.UUID(int=0))}]:
            status, _ = self.h.post('/api/peers', {'operation':'messages','command':action})
            self.assertEqual(status, 400)
        self.assertEqual(len(self.mailbox(operation='list')['outgoing']), 3)

    def test_target_receipts_cannot_drop_an_agent_address(self):
        class LegacyReceipt(BaseHTTPRequestHandler):
            def do_POST(inner):
                payload = json.loads(inner.rfile.read(int(inner.headers['Content-Length'])))
                body = json.dumps({key:payload[key] for key in ['id', 'sender', 'recipient']} | {'status':'received'}).encode()
                inner.send_response(200)
                inner.send_header('Content-Type', 'application/json')
                inner.send_header('Content-Length', str(len(body)))
                inner.end_headers()
                inner.wfile.write(body)
            def log_message(self, *_args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), LegacyReceipt)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.addCleanup(close)
        self.api(operation='connect', node='legacy-target', url='http://127.0.0.1:'+str(server.server_port), token='c'*64)
        message_id = str(uuid.uuid4())
        target_session = str(uuid.uuid4())
        self.mailbox(operation='send', id=message_id, node='legacy-target', text='Keep my address', target_session=target_session)
        item = self.delivered(message_id, 'blocked')
        self.assertIn('incompatible receipt', item['error'])
        self.assertEqual(item['message']['target_session'], target_session)
        self.assertEqual(self.mailbox(operation='list')['outgoing'][0]['message']['id'], message_id)


if __name__ == '__main__':
    unittest.main()
