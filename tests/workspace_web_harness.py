"""Isolated Rust Web, gateway and Linux node used by tests and measurements."""
import http.client
import json
import os
from pathlib import Path
import queue
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
WEB = Path(os.environ.get('WORKSPACE_WEB_BIN', ROOT / 'target/debug/workspace-web'))
RUNTIME = Path(os.environ.get('WORKSPACE_RUNTIME_BIN', ROOT / 'target/debug/workspace-runtime'))


NODE = Path(os.environ.get("CRANIBOX_NODE_BIN", ROOT / "target/debug/cranibox-node"))
CORE = Path(os.environ.get("CRANIBOX_BIN", ROOT / "target/debug/cranibox"))

def sockets_allowed():
    try:
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
        return True
    except PermissionError:
        return False


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def until(check, seconds=12):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            result = check()
            if result:
                return result
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(.04)
    raise AssertionError('Timed out waiting for transition')


class EventStream:
    def __init__(self, harness, port=None):
        self.connection = http.client.HTTPConnection('127.0.0.1', port or harness.web_port, timeout=20)
        self.connection.request('POST', '/api/events', '{}', harness.headers())
        self.response = self.connection.getresponse()
        self.status = self.response.status
        self.frames = queue.Queue()
        self.bytes = 0
        self.values = []
        self.stopping = False
        if self.status == 200:
            self.thread = threading.Thread(target=self.read, daemon=True)
            self.thread.start()
        harness.streams.append(self)

    def read(self):
        data = []
        try:
            while not self.stopping:
                line = self.response.readline()
                if not line:
                    return
                self.bytes += len(line)
                if line.startswith(b'data:'):
                    data.append(line[5:].strip())
                elif line.strip() == b'' and data:
                    value = json.loads(b'\n'.join(data))
                    self.values.append(value)
                    self.frames.put(value)
                    data = []
        except (OSError, http.client.HTTPException, ValueError):
            if not self.stopping:
                self.frames.put({'stream_error': True})

    def next(self, seconds=8):
        return self.frames.get(timeout=seconds)

    def close(self):
        self.stopping = True
        if self.connection.sock:
            try:
                self.connection.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        if hasattr(self, 'thread'):
            self.thread.join(timeout=2)
        self.connection.close()
        self.response.close()


class Harness:
    def __init__(self, unified=False, node_env=None, web_env=None):
        self.unified = unified
        # Simulated remote devices share this host network with the Web node.
        # Model routing is exercised by the Rust three-node integration test;
        # skip the fixed model listener on this simulated gateway client.
        self.node_env = {"CRANIBOX_APP_MOCK": "1", **(node_env or {})}
        # Multiple Web hosts also share this network. Their model listeners must
        # be disabled just like the simulated remote nodes above.
        self.web_env = {"CRANIBOX_APP_MOCK": "1", **(web_env or {})}
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-operations-')
        self.root = Path(self.temp.name)
        self.streams = []
        self.processes = []
        self.logs = []
        self.cookie = ''
        self.csrf = ''
        try:
            self.initialize()
        except BaseException:
            self.close()
            raise

    def initialize(self):
        # Keep both reservations open while allocating: releasing the first
        # socket before the second bind can return the same ephemeral port.
        with socket.socket() as web_socket, socket.socket() as gateway_socket:
            web_socket.bind(('127.0.0.1', 0))
            gateway_socket.bind(('127.0.0.1', 0))
            self.web_port = web_socket.getsockname()[1]
            self.gateway_port = gateway_socket.getsockname()[1]
        self.credentials = self.root / '.config/agent-workspace/runtime.json'
        self.credentials.parent.mkdir(parents=True)
        self.cli('init', '--credentials', self.credentials)
        self.cli('enroll', 'test-node', '--credentials', self.credentials, '--token-file', self.root / 'node.token')
        (self.credentials.parent / 'web.json').write_text(json.dumps({
            'port': self.web_port, 'runtimeUrl': f'http://127.0.0.1:{self.gateway_port}',
            'publicUrl': f'http://127.0.0.1:{self.gateway_port}'}))
        self.command = self.root / 'workspacectl'
        self.command.write_text('#!' + sys.executable + '''
import json,os,sys,time
from pathlib import Path
root=Path(__file__).parent
args=sys.argv[1:]
with (root / "calls").open("a") as calls: calls.write(json.dumps(args)+"\\n")
if args[0]=="status":
 print(json.dumps({"schema_version":1,"summary":{"healthy":True},"locale":{"current":"en"},"services":[]}))
elif args[0]=="logs":
 (root / "child.pid").write_text(str(os.getpid()))
 print("long job started",flush=True)
 os.write(1,b"Z" * (140*1024))
 for byte in "中文输出\\n".encode():
  os.write(1,bytes([byte]));time.sleep(.01)
 for n in range(600):
  time.sleep(.2); print("tick " + str(n),flush=True)
else:
 print("completed " + " ".join(args),flush=True)
''', encoding='utf8')
        self.command.chmod(0o700)
        self.start_web()
        if not self.unified:
            self.start_gateway()
        self.node = self.start(NODE, '--id', 'test-node', '--server', f'ws://127.0.0.1:{self.gateway_port}', '--token-file', self.root / 'node.token')
        self.login()
        until(lambda: self.gateway('/api/v1/nodes')['nodes'][0]['online'])
        until(lambda: self.post('/api/poll', {})[1].get('snapshot'))

    def cli(self, *args):
        command = [str(CORE), 'runtime'] if self.unified else [str(RUNTIME)]
        return subprocess.run([*command, *map(str, args)], capture_output=True, check=True, timeout=10)

    def start(self, binary, *args):
        log = (self.root / f'process-{len(self.logs)}.log').open('wb')
        self.logs.append(log)
        if Path(binary) == NODE and '--config' not in args:
            # The simulated remote device has its own identity and catalog.
            args = ('--config', self.root / 'test-node/settings.json', *args)
        environment = {**os.environ, 'HOME': str(self.root), **self.node_env} if Path(binary) == NODE else None
        if Path(binary) in (WEB, CORE) and self.web_env:
            environment = {**os.environ, **self.web_env}
        process = subprocess.Popen([str(binary), *map(str, args)], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=environment)
        self.processes.append(process)
        return process

    def start_web(self):
        if self.unified:
            self.web = self.start(CORE, 'serve', '--config-root', self.root, '--command', self.command, '--gateway-port', self.gateway_port)
            self.server = self.web
        else:
            self.web = self.start(WEB, '--config-root', self.root, '--command', self.command)
        try:
            until(lambda: self.http('GET', '/healthz')[0] == 200)
        except AssertionError as error:
            path = self.root / f'process-{len(self.logs) - 1}.log'
            raise AssertionError(f'{error}\n{path.read_text(errors="replace")}') from error

    def start_gateway(self):
        self.server = self.start(RUNTIME, 'serve', '--credentials', self.credentials,
                                 '--listen', f'127.0.0.1:{self.gateway_port}', '--web-upstream', f'http://127.0.0.1:{self.web_port}')
        until(lambda: self.http('GET', '/healthz', port=self.gateway_port)[0] == 200)

    def headers(self):
        return {'Cookie': self.cookie, 'X-Workspace-CSRF': self.csrf, 'Content-Type': 'application/json'}

    def http(self, method, path, data=None, port=None, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', port or self.web_port, timeout=8)
        try:
            connection.request(method, path, json.dumps(data) if data is not None else None, headers or self.headers())
            response = connection.getresponse()
            body = response.read()
            self.last_size = len(body)
            self.last_headers = dict(response.getheaders())
            return response.status, json.loads(body) if 'application/json' in response.getheader('Content-Type', '') else body.decode()
        finally:
            connection.close()

    def login(self):
        self.cookie = ''
        status, html = self.http('GET', '/')
        assert status == 200
        self.cookie = self.last_headers['set-cookie'].split(';')[0]
        self.csrf = json.loads(re.search(r'window.workspaceWeb=(.*?);</script>', html).group(1))['csrf']

    def post(self, path, body):
        return self.http('POST', path, body)

    def gateway(self, path, data=None):
        token = json.loads(self.credentials.read_text())['controller_token']
        status, body = self.http('POST' if data is not None else 'GET', path, data, self.gateway_port,
                                {'Authorization':'Bearer ' + token, 'Content-Type':'application/json'})
        assert status < 300, (status, body)
        return body

    def remote_job(self, code, timeout=10):
        status, value = self.post('/api/nodes/exec', {'id':'test-node','request':{
            'program':sys.executable,'args':['-c',code], 'env':{},'timeout_secs':timeout}})
        assert status == 200, value
        return value['id']

    def stop(self, process, force=False):
        if process.poll() is None:
            process.kill() if force else process.terminate()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    def close(self):
        for stream in self.streams:
            stream.close()
        for process in reversed(self.processes):
            self.stop(process)
        # The local external engine deliberately outlives the Web process.
        herdr = os.environ.get('CRANIBOX_HERDR_BIN')
        if herdr:
            for state in (self.root / '.local/state/cranibox/terminal',):
                if state.exists():
                    environment = {**os.environ, 'HERDR_CONFIG_PATH': str(state / 'config.toml'), 'HERDR_SOCKET_PATH': str(state / 'h.sock')}
                    subprocess.run([herdr, 'server', 'stop'], env=environment, capture_output=True, timeout=8)
        child = self.root / 'child.pid'
        if child.exists():
            try:
                os.killpg(int(child.read_text()), signal.SIGKILL)
            except ProcessLookupError:
                pass
        for log in self.logs:
            log.close()
        self.temp.cleanup()
