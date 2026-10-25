"""Isolated Rust Web host (with its embedded device node) used by tests and measurements."""
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


CORE = Path(os.environ.get("CRANIBOX_BIN", ROOT / "target/debug/cranibox"))
# The node CLI is the same program.
NODE = Path(os.environ.get("CRANIBOX_NODE_BIN", CORE))

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
        # skip the fixed model listener on these simulated devices.
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
        # There is no gateway or gateway-enrolled device any more; tests that stop
        # them to prove independence keep working against the Web host alone.
        self.server = self.node = None
        try:
            self.initialize()
        except BaseException:
            self.close()
            raise

    def initialize(self):
        self.web_port = free_port()
        config = self.root / '.config/agent-workspace'
        config.mkdir(parents=True)
        (config / 'web.json').write_text(json.dumps({'port': self.web_port}))
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
        self.login()
        until(lambda: self.post('/api/poll', {})[1].get('snapshot'))

    def start(self, binary, *args):
        log = (self.root / f'process-{len(self.logs)}.log').open('wb')
        self.logs.append(log)
        # NODE and CORE are one program; a simulated device always names its own settings.
        device = Path(binary) == NODE and args[:1] == ('--config',)
        environment = {**os.environ, 'HOME': str(self.root), **self.node_env} if device else None
        if not device and Path(binary) in (WEB, CORE) and self.web_env:
            environment = {**os.environ, **self.web_env}
        process = subprocess.Popen([str(binary), *map(str, args)], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=environment)
        self.processes.append(process)
        return process

    def start_web(self):
        if self.unified:
            self.web = self.start(CORE, 'serve', '--config-root', self.root, '--command', self.command)
        else:
            self.web = self.start(WEB, '--config-root', self.root, '--command', self.command)
        try:
            until(lambda: self.http('GET', '/healthz')[0] == 200)
        except AssertionError as error:
            path = self.root / f'process-{len(self.logs) - 1}.log'
            raise AssertionError(f'{error}\n{path.read_text(errors="replace")}') from error

    def pair_remote(self, *permissions, name='remote'):
        """Start a real peer device that grants this Web host `permissions` (CLI flags)."""
        config = self.root / name / 'settings.json'

        def peer(*args):
            result = subprocess.run([str(NODE), '--config', str(config), 'peer', *map(str, args)],
                                    capture_output=True, text=True, timeout=10,
                                    env={**os.environ, 'HOME': str(self.root), **self.node_env})
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        caller = self.post('/api/peers', {'operation': 'identity'})[1]['node_id']
        node = peer('identity')['node_id']
        token = self.root / f'{name}.token'
        peer('grant', '--allow-unverified', '--node', caller, '--output-token', token, *permissions)
        port = free_port()
        peer('listen', '--address', f'127.0.0.1:{port}')
        self.remote_config = config
        self.node = self.start(NODE, '--config', config, 'peer', 'serve')
        status, value = self.post('/api/peers', {'operation': 'connect', 'node': node, 'url': f'http://127.0.0.1:{port}',
                                                 'token': token.read_text().strip()})
        assert status == 200, value
        until(lambda: self.post('/api/peers', {'operation': 'probe', 'node': node})[0] == 200)
        return node

    def restart_remote(self):
        self.stop(self.node)
        self.node = self.start(NODE, '--config', self.remote_config, 'peer', 'serve')

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

    def stop(self, process, force=False):
        if process is not None and process.poll() is None:
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
        # So do standalone nodes' terminal hosts (terminal-host.json next to each config).
        for record in self.root.rglob('terminal-host.json'):
            try:
                os.kill(json.loads(record.read_text())['pid'], signal.SIGTERM)
            except (OSError, ValueError, KeyError):
                pass
        child = self.root / 'child.pid'
        if child.exists():
            try:
                os.killpg(int(child.read_text()), signal.SIGKILL)
            except ProcessLookupError:
                pass
        for log in self.logs:
            log.close()
        self.temp.cleanup()
