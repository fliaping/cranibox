"""Boot the image's built-in control plane in an isolated persistent volume."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.request

SOURCE = Path(__file__).resolve().parents[1]
BINARIES = Path(os.environ.get('WORKSPACE_BUILTIN_BIN_DIR', SOURCE / 'target/debug'))
NAMES = ('cranibox', 'workspacectl')


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


@unittest.skipUnless(all((BINARIES / name).exists() for name in NAMES) and
                     shutil.which('s6-svscan'), 'build native binaries and install s6 first')
class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='cranibox-boot-')
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.home = self.directory / 'volume'
        self.scripts = self.directory / 'image/bin'
        self.scripts.mkdir(parents=True)
        for name in ('bootstrap-workspace-core.sh', 'install-workspace-runtime.sh',
                     'install-workspace-web.sh', 'install-workspacectl.sh', 'install-cranibox.sh', 'install-cranibox.py'):
            shutil.copy2(SOURCE / 'scripts' / name, self.scripts / name)
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(('AGENT_WORKSPACE_', 'CARGO_', 'RUSTUP_'))}
        self.env.update(HOME=str(self.home), AGENT_WORKSPACE_CONFIG_ROOT=str(self.home),
                        AGENT_WORKSPACE_BUILTIN_DIR=str(BINARIES), PATH='/usr/bin:/bin',
                        AGENT_WORKSPACE_RUNTIME_URL='http://127.0.0.1:9')
        self.units = self.home / '.config/systemd/user'
        self.credentials = self.home / '.config/agent-workspace/runtime.json'
        self.addCleanup(self.stop_services)

    def bootstrap(self):
        result = subprocess.run(['/bin/bash', str(self.scripts / 'bootstrap-workspace-core.sh')],
                                env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def manager(self, *args, check=True):
        return subprocess.run([str(self.home / 'bin/workspace-services'), '--user', *args],
                              env=self.env, check=check, capture_output=True, text=True, timeout=20)

    def stop_services(self):
        if not (self.home / 'bin/workspace-services').exists():
            return
        for unit in self.units.glob('*.service'):
            self.manager('stop', unit.name, check=False)
        scan = self.home / '.local/run/workspace-services/services'
        if (scan / '.s6-svscan/control').exists():
            for service in scan.iterdir():
                if (service / 'supervise/control').exists():
                    subprocess.run(['s6-svc', '-x', str(service)], capture_output=True, timeout=5)
            subprocess.run(['s6-svscanctl', '-t', str(scan)], capture_output=True, timeout=5)
            deadline = time.monotonic() + 6
            while time.monotonic() < deadline:
                if subprocess.run(['s6-svscanctl', '-a', str(scan)], capture_output=True).returncode:
                    break
                time.sleep(.05)

    def test_empty_volume_gets_enabled_services_without_source_or_cargo(self):
        self.bootstrap()
        self.assertEqual(self.credentials.stat().st_mode & 0o777, 0o600)
        self.assertTrue(json.loads(self.credentials.read_text())['controller_token'])
        self.assertEqual(self.manager('is-enabled', 'agent-workspace-web').stdout.strip(), 'enabled')
        self.assertFalse((self.units / 'agent-workspace-runtime.service').exists())
        for name in ('workspacectl', 'workspace-services', 'workspace-runtime'):
            self.assertTrue((self.home / 'bin' / name).is_symlink())
            self.assertEqual((self.home / 'bin' / name).resolve(), self.home / 'bin/cranibox')
        self.assertEqual((self.home / 'bin/workspacectl').read_bytes()[:4], b'\x7fELF')
        # Exercise the same enabled units that the startup oneshot starts.
        ports = {name: free_port() for name in ('runtime', 'web')}
        unit = self.units / 'agent-workspace-web.service'
        text = unit.read_text().replace(' --config-root', f' --port {ports["web"]} --gateway-port {ports["runtime"]} --config-root')
        unit.write_text(text)
        self.manager('start', unit.name)
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        for port in ports.values():
            deadline = time.monotonic() + 10
            while True:
                try:
                    with opener.open(f'http://127.0.0.1:{port}/healthz', timeout=1) as response:
                        self.assertEqual(response.status, 200)
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        self.fail('Built-in service did not become healthy')
                    time.sleep(.05)

    def test_repeated_boot_preserves_credentials_configuration_and_disabled_state(self):
        self.bootstrap()
        subprocess.run([str(self.home / 'bin/workspace-runtime'), 'enroll', 'existing-node',
                        '--credentials', str(self.credentials), '--token-file', str(self.directory / 'node.token')],
                       check=True, capture_output=True, env=self.env, timeout=10)
        self.manager('disable', 'agent-workspace-web')
        self.credentials.chmod(0o644)
        for unit in self.units.glob('*.service'):
            with unit.open('a') as output:
                output.write('\n# user-maintained configuration\n')
        before = {p: p.read_bytes() for p in [self.credentials, *self.units.glob('*.service')]}
        binary = self.home / 'bin/cranibox'
        original_mtime = binary.stat().st_mtime_ns
        self.bootstrap()
        self.assertEqual(self.credentials.stat().st_mode & 0o777, 0o600)
        self.assertEqual(binary.stat().st_mtime_ns, original_mtime)
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertNotEqual(self.manager('is-enabled', 'agent-workspace-web', check=False).returncode, 0)
        # Restore missing program files while keeping custom service definitions.
        (self.home / 'bin/workspace-runtime').unlink()
        (self.home / 'opt/agent-workspace-web/workspace-web').unlink()
        self.bootstrap()
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)

    def test_prebuilt_ctl_installer_needs_no_checkout(self):
        env = {**self.env, 'AGENT_WORKSPACE_CTL_BIN': str(BINARIES / 'workspacectl'),
               'AGENT_WORKSPACE_BUILTIN_DIR': str(self.directory / 'missing')}
        result = subprocess.run(['/bin/bash', str(self.scripts / 'install-workspacectl.sh')],
                                env=env, capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.home / 'bin/workspacectl').read_bytes()[:4], b'\x7fELF')

    def seed_legacy(self):
        self.units.mkdir(parents=True)
        (self.home / 'bin').mkdir()
        (self.home / 'opt/agent-workspace-web').mkdir(parents=True)
        for name in ('workspacectl', 'workspace-services', 'workspace-runtime'):
            shutil.copy2(BINARIES / name, self.home / 'bin' / name)
        shutil.copy2(BINARIES / 'workspace-web', self.home / 'opt/agent-workspace-web/workspace-web')
        subprocess.run([str(self.home / 'bin/workspace-runtime'), 'init', '--credentials', str(self.credentials)], env=self.env, check=True, capture_output=True)
        subprocess.run([str(self.home / 'bin/workspace-runtime'), 'enroll', 'retained-node', '--credentials', str(self.credentials), '--token-file', str(self.directory / 'existing.token')], env=self.env, check=True, capture_output=True)
        port, gateway = free_port(), free_port()
        for name, command in {
            'runtime': f'"{self.home}/bin/workspace-runtime" serve --credentials "{self.credentials}" --listen 127.0.0.1:{gateway}',
            'web': f'"{self.home}/opt/agent-workspace-web/workspace-web" --config-root "{self.home}" --port {port} --gateway-port {gateway}',
        }.items():
            unit = self.units / f'agent-workspace-{name}.service'
            unit.write_text(f'[Service]\nType=simple\nEnvironment="HOME={self.home}"\nExecStart={command}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n# user-owned arguments\n')
            self.manager('enable', unit.name)
            self.manager('start', unit.name)
        custom = self.units / 'user-custom.service'
        custom.write_text('[Service]\nType=oneshot\nExecStart=/bin/true\n')
        return port

    def test_migration_retires_gateway_and_preserves_credentials_units_and_custom_service(self):
        port = self.seed_legacy()
        before = {p: p.read_bytes() for p in [self.credentials, *self.units.glob('*.service')]}
        self.bootstrap()
        for path, data in before.items():
            self.assertEqual(path.read_bytes(), data)
        self.assertNotEqual(self.manager('is-active', 'agent-workspace-runtime', check=False).returncode, 0)
        self.assertNotEqual(self.manager('is-enabled', 'agent-workspace-runtime', check=False).returncode, 0)
        self.assertEqual(self.manager('is-enabled', 'agent-workspace-web').stdout.strip(), 'enabled')
        self.manager('restart', 'agent-workspace-web')
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        deadline = time.monotonic() + 8
        while True:
            try:
                with opener.open(f'http://127.0.0.1:{port}/healthz', timeout=1) as response:
                    self.assertEqual(json.load(response)['protocol_version'], 1)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(.05)

    def test_failed_install_restores_binaries_credentials_and_service_state(self):
        self.seed_legacy()
        before = {p: p.read_bytes() for p in [self.credentials, *self.units.glob('*.service'), *self.home.joinpath('bin').glob('*')] if p.is_file()}
        fake = self.directory / 'failed-core'
        fake.write_text('#!/bin/sh\nif [ "$1" = --version ]; then exit 0; fi\nexit 7\n')
        fake.chmod(0o755)
        result = subprocess.run(['/bin/bash', str(self.scripts / 'install-cranibox.sh')], env={**self.env, 'CRANIBOX_BIN': str(fake)}, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        for path, data in before.items():
            self.assertEqual(path.read_bytes(), data)
        self.assertFalse((self.home / 'bin/cranibox').exists())
        for name in ('runtime', 'web'):
            self.assertEqual(self.manager('is-enabled', 'agent-workspace-' + name).stdout.strip(), 'enabled')
            self.assertEqual(self.manager('is-active', 'agent-workspace-' + name).stdout.strip(), 'active')


if __name__ == '__main__':
    unittest.main()
