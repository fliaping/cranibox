"""Opt-in real Chromium + official MCP. Uses only a temporary profile and test page."""
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import threading
import time
import unittest
from urllib.parse import parse_qs, urlsplit

import test_node_peers as peers
from workspace_web_harness import NODE, sockets_allowed, until


@unittest.skipUnless(os.environ.get('CRANIBOX_TEST_REAL_BROWSER') == '1' and NODE.exists() and sockets_allowed(),
                     'set CRANIBOX_TEST_REAL_BROWSER=1; Chromium, Node.js and npm access required')
class RealBrowserTests(unittest.TestCase):
    cli, http, start = peers.PeerTests.cli, peers.PeerTests.http, peers.PeerTests.start
    close, stop = peers.PeerTests.close, peers.PeerTests.stop

    def setUp(self):
        peers.PeerTests.setUp(self)
        self.buffer, self.sequence, self.submitted = b'', 0, []

    def rpc(self, method, params=None):
        self.sequence += 1
        request_id = self.sequence
        self.mcp.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': request_id, 'method': method, 'params': params or {}}) + '\n').encode())
        self.mcp.stdin.flush()
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if b'\n' not in self.buffer:
                self.assertTrue(select.select([self.mcp.stdout], [], [], max(0, deadline - time.monotonic()))[0], 'MCP response timed out')
                data = os.read(self.mcp.stdout.fileno(), 65536)
                self.assertTrue(data, 'MCP exited without a response')
                self.buffer += data
                continue
            line, self.buffer = self.buffer.split(b'\n', 1)
            value = json.loads(line)
            if value.get('id') == request_id:
                self.assertNotIn('error', value, value)
                return value['result']
        self.fail('MCP response timed out')

    def tool(self, name, **arguments):
        value = self.rpc('tools/call', {'name': name, 'arguments': arguments})
        self.assertFalse(value.get('isError'), value)
        return value

    def test_browser_mcp_controls_real_target_and_stops_without_replay(self):
        executable = os.environ.get('CRANIBOX_TEST_CHROMIUM') or shutil.which('chromium') or shutil.which('google-chrome')
        self.assertTrue(executable, 'Set CRANIBOX_TEST_CHROMIUM to an installed Chromium executable')
        profile = self.root / 'browser-profile'
        log = (self.root / 'chromium.log').open('wb'); self.logs.append(log)
        chrome = subprocess.Popen([executable, '--headless=new', '--no-sandbox', '--disable-gpu', '--disable-background-networking',
                                   '--no-first-run', '--no-default-browser-check', '--remote-debugging-address=127.0.0.1',
                                   '--remote-debugging-port=0', f'--user-data-dir={profile}', 'about:blank'], stdout=log, stderr=log)
        self.processes.append(chrome)
        until(lambda: (profile / 'DevToolsActivePort').exists())
        port = int((profile / 'DevToolsActivePort').read_text().splitlines()[0])
        owner = self

        class Page(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                url = urlsplit(self.path)
                if url.path == '/submitted':
                    owner.submitted.append(parse_qs(url.query)['value'][0])
                    data = b'ok'
                else:
                    data = b'''<!doctype html><title>Cranibox Browser Acceptance</title><h1>Target device browser</h1>
                    <form onsubmit="event.preventDefault();fetch('/submitted?value='+encodeURIComponent(document.querySelector('input').value));document.querySelector('output').textContent='Submitted'">
                    <label>Name<input name="name"></label><button>Submit</button></form><output></output>'''
                self.send_response(200); self.send_header('Content-Type', 'text/html'); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)

        page = ThreadingHTTPServer(('127.0.0.1', 0), Page)
        self.addCleanup(page.server_close); self.addCleanup(page.shutdown)
        threading.Thread(target=page.serve_forever, daemon=True).start()
        self.cli(self.b, 'revoke', '--node', self.caller); self.token_path.unlink()
        self.cli(self.b, 'grant', '--node', self.caller, '--output-token', self.token_path, '--browser')
        self.token = self.token_path.read_text().strip()
        self.cli(self.a, 'connect', '--node', self.target, '--url', f'http://127.0.0.1:{self.port}', '--credential-file', self.token_path)
        self.cli(self.b, 'browser', '--address', f'127.0.0.1:{port}'); self.start()
        log = (self.root / 'mcp.log').open('wb'); self.logs.append(log)
        self.mcp = subprocess.Popen([str(NODE), '--config', str(self.a), 'peer', 'call', '--node', self.target, 'browser-mcp'],
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log)
        self.processes.append(self.mcp)
        self.addCleanup(self.mcp.stdin.close); self.addCleanup(self.mcp.stdout.close)
        self.rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {}, 'clientInfo': {'name': 'cranibox-browser-acceptance', 'version': '1'}})
        self.mcp.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); self.mcp.stdin.flush()
        tools = self.rpc('tools/list')['tools']
        self.assertTrue({'new_page', 'take_snapshot', 'fill', 'click', 'take_screenshot'} <= {t['name'] for t in tools})
        page_url = f'http://127.0.0.1:{page.server_port}/'
        opened = self.tool('new_page', url=page_url)
        page_list = '\n'.join(c.get('text', '') for c in opened['content'])
        selected = re.search(r'(?m)^(\d+): Cranibox Browser Acceptance \(' + re.escape(page_url) + r'\) \[selected\]$', page_list)
        self.assertIsNotNone(selected, page_list)
        page_id = int(selected[1])
        snapshot = self.tool('take_snapshot', pageId=page_id)
        text = '\n'.join(c.get('text', '') for c in snapshot['content'])
        field = re.search(r'uid=(\S+) textbox "Name"', text)
        button = re.search(r'uid=(\S+) button "Submit"', text)
        self.assertIsNotNone(field, text); self.assertIsNotNone(button, text)
        self.tool('fill', pageId=page_id, uid=field[1], value='cranibox-remote-browser')
        self.tool('click', pageId=page_id, uid=button[1]); until(lambda: self.submitted == ['cranibox-remote-browser'])
        image = next(c for c in self.tool('take_screenshot', pageId=page_id, format='png')['content'] if c['type'] == 'image')
        self.assertTrue(base64.b64decode(image['data']).startswith(b'\x89PNG\r\n\x1a\n'))
        self.cli(self.b, 'browser-owner', '--command', '{"action":"emergency_stop"}')
        denied = self.rpc('tools/call', {'name': 'take_snapshot', 'arguments': {'pageId': page_id}})
        self.assertTrue(denied.get('isError'), denied)
        self.assertEqual(self.submitted, ['cranibox-remote-browser'])
        self.mcp.stdin.close(); self.mcp.wait(timeout=10)
        self.assertEqual(self.mcp.returncode, 0)
