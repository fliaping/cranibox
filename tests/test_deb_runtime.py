"""Regression tests for the filesystem boundary, transactions and desktop argv."""
import importlib.machinery
import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import socket
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / 'scripts/agent-workspace-deb'
loader = importlib.machinery.SourceFileLoader('deb_runtime', str(SOURCE))
spec = importlib.util.spec_from_loader(loader.name, loader)
module = importlib.util.module_from_spec(spec)
loader.exec_module(module)


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='deb-runtime-test-')
        self.addCleanup(self.temporary.cleanup)
        # Exercise desktop quoting using a real config path with shell metacharacters.
        self.config = Path(self.temporary.name) / 'home with $dollar'
        self.environment = patch.dict(os.environ, {'AGENT_WORKSPACE_CONFIG_ROOT': str(self.config)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.runtime = module.Runtime()
        self.runtime.root.mkdir(parents=True)
        self.runtime.proot.parent.mkdir(parents=True)
        self.runtime.proot.write_text('#!/bin/sh\nexit 0\n')
        self.runtime.proot.chmod(0o755)
        self.runtime.command.parent.mkdir(parents=True)

    def ready(self):
        module.atomic_json(self.runtime.state / 'ready.json', {'release': '24.04'})
        apt = self.runtime.root / 'usr/bin/apt-get'
        apt.parent.mkdir(parents=True, exist_ok=True)
        apt.touch()

    def desktop(self):
        desktop = self.runtime.root / 'usr/share/applications/test.desktop'
        desktop.parent.mkdir(parents=True)
        desktop.write_text(
            '[Desktop Entry]\nType=Application\nName=Test\nName[zh_CN]=测试\n'
            'Exec="/opt/Test App/bin/test" --literal "$unchanged" %F\n'
            'TryExec=/opt/Test App/bin/test\nPath=/opt/Test App\n'
            'DBusActivatable=true\nIcon=test\nActions=New;\n'
            '[Desktop Action New]\nName=New\nExec="/opt/Test App/bin/test" --new\n')
        return '/usr/share/applications/test.desktop'

    def purge_fixture(self):
        self.ready()
        module.atomic_json(self.runtime.manifest, {'schema': 1, 'packages': {'test-app': {}}})
        self.runtime.cache.mkdir(parents=True)
        archive = self.runtime.cache / ('test-app-' + 'a' * 64 + '.deb')
        archive.write_text('old installer')
        unrelated = self.runtime.cache / ('test-app-extra-' + 'b' * 64 + '.deb')
        unrelated.write_text('keep')
        config = self.config / '.config/test-app'
        config.mkdir(parents=True)
        (config / 'settings').write_text('keep by default')
        return archive, unrelated, config

    def test_purge_cleans_cache_and_history_but_preserves_config(self):
        archive, unrelated, config = self.purge_fixture()
        with patch.object(self.runtime, 'apt') as apt, patch.object(self.runtime, 'sync'), \
                patch.object(self.runtime, 'installed', return_value=(False, '')):
            self.runtime.purge('test-app', False, autoremove=True)
        apt.assert_called_once_with(['-y', '--autoremove', 'purge', 'test-app'])
        self.assertFalse(archive.exists())
        self.assertTrue(unrelated.exists())
        self.assertTrue(config.exists())
        self.assertNotIn('test-app', self.runtime.load()['packages'])

    def test_purge_dry_run_and_apt_failure_preserve_files_and_history(self):
        archive, _, config = self.purge_fixture()
        with patch.object(self.runtime, 'apt'), patch.object(self.runtime, 'sync') as sync:
            self.runtime.purge('test-app', True, user_config_paths=['.config/test-app'])
            sync.assert_not_called()
        with patch.object(self.runtime, 'apt', side_effect=RuntimeError('apt failed')):
            with self.assertRaisesRegex(RuntimeError, 'apt failed'):
                self.runtime.purge('test-app', False)
        self.assertTrue(archive.exists())
        self.assertTrue(config.exists())
        self.assertIn('test-app', self.runtime.load()['packages'])

    def test_purge_explicit_config_deletion(self):
        _, _, config = self.purge_fixture()
        with patch.object(self.runtime, 'apt'), patch.object(self.runtime, 'sync'), \
                patch.object(self.runtime, 'installed', return_value=(False, '')):
            self.runtime.purge('test-app', False, user_config_paths=['.config/test-app'])
        self.assertFalse(config.exists())

    def test_purge_rejects_broad_traversing_and_symlinked_config_paths(self):
        self.purge_fixture()
        (self.config / '.config/link').symlink_to(self.config / '.config/test-app')
        for path in ('.config', '.local/share', '/config', '.config/../other',
                     '.config/link', '.config/code-server', '.local/share/agent-workspace'):
            with self.subTest(path=path), patch.object(self.runtime, 'apt') as apt:
                with self.assertRaises(RuntimeError):
                    self.runtime.purge('test-app', False, user_config_paths=[path])
                apt.assert_not_called()

    def test_install_never_binds_host_package_database_or_home(self):
        argv = self.runtime.invocation(['apt-get', 'install', 'test'], install=True)
        self.assertIn('-0', argv)
        self.assertNotIn('-R', argv)
        bindings = [argv[i + 1] for i, value in enumerate(argv) if value == '-b']
        for forbidden in ('/config', '/etc/passwd', '/etc/group', '/var/lib/dpkg', '/run:'):
            self.assertFalse(any(forbidden in value for value in bindings), bindings)
        self.assertEqual(self.runtime.environment(True)['HOME'], '/root')
        self.assertEqual(self.runtime.environment(True)['DPKG_DEB_THREADS_MAX'], '1')

    def test_runtime_does_not_inherit_editor_or_library_injection(self):
        with patch.dict(os.environ, {'LD_PRELOAD': '/host/library.so', 'VSCODE_IPC_HOOK_CLI': '/old/ipc'}):
            env = self.runtime.environment(False)
        self.assertNotIn('LD_PRELOAD', env)
        self.assertNotIn('VSCODE_IPC_HOOK_CLI', env)
        self.assertEqual(env['HOME'], str(self.config))

    def test_package_writers_are_serialized_while_apps_can_run(self):
        with self.runtime.lock():
            with self.assertRaisesRegex(RuntimeError, 'Another persistent'):
                with self.runtime.lock():
                    pass
            with self.runtime.app_lock(shared=True):
                pass
        with self.runtime.app_lock(shared=True):
            with self.runtime.lock():
                pass

    def test_rootfs_operations_and_running_apps_exclude_each_other(self):
        with self.runtime.lock(exclusive_apps=True):
            with self.assertRaisesRegex(RuntimeError, 'backup or restore is running'):
                with self.runtime.app_lock(shared=True):
                    pass
        with self.runtime.app_lock(shared=True):
            with self.assertRaisesRegex(RuntimeError, 'Close persistent'):
                with self.runtime.lock(exclusive_apps=True):
                    pass

    def test_cli_allows_install_but_rejects_restore_while_app_runs(self):
        with patch.object(module, 'Runtime', return_value=self.runtime), \
                self.runtime.app_lock(shared=True):
            with patch.object(self.runtime, 'install') as install, \
                    patch.object(module.sys, 'argv', ['deb', 'install', '--yes', '/tmp/example.deb']):
                self.assertEqual(module.main(), 0)
                install.assert_called_once_with(Path('/tmp/example.deb'), False)
            for action, arguments in [('backup', []), ('restore', ['--yes', '/tmp/backup.tar.gz'])]:
                with self.subTest(action=action), \
                        patch.object(self.runtime, action) as operation, \
                        patch.object(module.sys, 'argv', ['deb', action, *arguments]):
                    with self.assertRaisesRegex(RuntimeError, 'Close persistent'):
                        module.main()
                    operation.assert_not_called()

    def test_failed_desktop_launch_reports_error_on_kde_without_zenity(self):
        self.ready()
        runner = self.config / 'fail-application'
        runner.write_text('#!/bin/sh\necho application-failed >&2\nexit 7\n')
        runner.chmod(0o755)
        dialog = self.config / 'kdialog'
        reported = self.config / 'dialog-arguments.json'
        dialog.write_text(
            '#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\n'
            f'Path({str(reported)!r}).write_text(json.dumps(sys.argv[1:]))\n')
        dialog.chmod(0o755)
        with patch.object(self.runtime, 'invocation', return_value=[str(runner)]), \
                patch.object(self.runtime, 'open_bridge', return_value=contextlib.nullcontext()), \
                patch.dict(os.environ, {'PATH': str(self.config)}):
            code = self.runtime.run(['example-app'], package='example-app', desktop=True, cwd=None)
        self.assertEqual(code, 7)
        deadline = time.monotonic() + 5
        while not reported.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(reported.exists(), 'KDE launch error dialog was not invoked')
        self.assertIn('exited with status 7', reported.read_text())
        log = (self.runtime.log.parent / 'deb-apps/example-app.log').read_text()
        self.assertIn('launch', log)
        self.assertIn('application-failed', log)
        self.assertIn('exit=7', log)

    def test_desktop_preserves_actions_field_codes_and_names(self):
        name = self.runtime.export_desktop('test-app', self.desktop(), [])
        entry = module.desktop_config()
        entry.read(self.runtime.launchers / name)
        main = entry['Desktop Entry']
        self.assertEqual(main['Name[zh_CN]'], '测试 (Persistent)')
        self.assertEqual(main['DBusActivatable'], 'false')
        self.assertNotIn('TryExec', main)
        self.assertNotIn('Path', main)
        self.assertTrue(main['Exec'].endswith('--literal "$unchanged" %F'))
        self.assertIn('run --desktop --package "test-app"', entry['Desktop Action New']['Exec'])

    def test_zero_exit_sandbox_failure_is_reported_and_not_reused(self):
        self.ready()
        with patch.object(self.runtime, 'invocation', return_value=[
                '/bin/sh', '-c', 'echo "ERROR:zygote_linux.cc Broken pipe (32)"; exit 0']), \
                patch.object(self.runtime, 'open_bridge', return_value=contextlib.nullcontext()), \
                patch.object(module.shutil, 'which', return_value=None):
            self.assertEqual(self.runtime.run(['app'], None, 'test-app', True), 1)
        with patch.object(self.runtime, 'invocation', return_value=['/bin/true']), \
                patch.object(self.runtime, 'open_bridge', return_value=contextlib.nullcontext()):
            self.assertEqual(self.runtime.run(['app'], None, 'test-app', True), 0)

    def test_overrides_precede_file_separator_and_preserve_values(self):
        self.ready()
        module.atomic_json(self.runtime.overrides, {'test-app': {'extra_args': ['--label', 'same']}})
        original = ['app', '--', 'same']
        with patch.object(self.runtime, 'invocation', return_value=['/bin/true']) as invocation, \
                patch.object(self.runtime, 'open_bridge', return_value=contextlib.nullcontext()):
            self.runtime.run(original, None, 'test-app')
        self.assertEqual(invocation.call_args.args[0], ['app', '--label', 'same', '--', 'same'])
        self.assertEqual(original, ['app', '--', 'same'])

    def test_primary_launcher_skips_hidden_url_handler(self):
        name = self.runtime.export_desktop('test-app', self.desktop(), [])
        hidden = self.runtime.launchers / 'agent-workspace-deb-hidden.desktop'
        hidden.write_text('[Desktop Entry]\nType=Application\nExec=app --url %U\nNoDisplay=true\n')
        module.atomic_json(self.runtime.manifest, {'packages': {'test-app': {'launchers': [hidden.name, name]}}})
        self.assertEqual(self.runtime.primary_launcher('test-app'), self.runtime.launchers / name)

    def test_real_gio_expands_files_without_shell_execution(self):
        try:
            from gi.repository import Gio
        except ImportError:
            self.skipTest('PyGObject not installed')
        output = self.config / 'argv.json'
        self.runtime.command.write_text(
            '#!/usr/bin/python3\nimport json,sys\n'
            + 'open(' + repr(str(output)) + ', "w").write(json.dumps(sys.argv[1:]))\n')
        self.runtime.command.chmod(0o755)
        name = self.runtime.export_desktop('test-app', self.desktop(), [])
        app = Gio.DesktopAppInfo.new_from_filename(str(self.runtime.launchers / name))
        self.assertIsNotNone(app)
        selected = self.config / 'file with spaces;$(touch SHOULD_NOT_EXIST).txt'
        selected.touch()
        self.assertTrue(app.launch([Gio.File.new_for_path(str(selected))], None))
        deadline = time.monotonic() + 3
        while not output.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        argv = json.loads(output.read_text())
        self.assertEqual(argv, ['run', '--desktop', '--package', 'test-app', '--cwd', '/opt/Test App', '--',
                                '/opt/Test App/bin/test', '--literal', '$unchanged', str(selected)])
        self.assertFalse((self.config / 'SHOULD_NOT_EXIST').exists())

    def test_guest_symlink_cannot_export_host_files(self):
        link = self.runtime.root / 'host-file'
        link.symlink_to('/etc/passwd')
        self.assertIsNone(self.runtime.guest_file('/host-file'))
        self.assertIsNone(self.runtime.guest_file('/../../etc/passwd'))

    def test_absolute_symlink_resolves_inside_guest(self):
        target = self.runtime.root / 'usr/share/icon.png'
        target.parent.mkdir(parents=True)
        target.write_bytes(b'guest icon')
        (self.runtime.root / 'icon').symlink_to('/usr/share/icon.png')
        self.assertEqual(self.runtime.guest_file('/icon'), target)

    def test_backup_restore_preserves_guest_absolute_symlinks(self):
        self.ready()
        module.atomic_json(self.runtime.state / 'ready.json', {'release': '24.04', 'arch': self.runtime.arch})
        marker = self.runtime.root / 'marker'
        marker.write_text('before')
        (self.runtime.root / 'link').symlink_to('/marker')
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.runtime.backup()
        archive = Path(output.getvalue().strip())
        marker.write_text('after')
        with contextlib.redirect_stdout(io.StringIO()):
            self.runtime.restore(archive)
        self.assertEqual(marker.read_text(), 'before')
        self.assertEqual(self.runtime.guest_file('/link'), marker)
        saved = list(archive.parent.glob('previous-*/rootfs/marker'))
        self.assertEqual(len(saved), 1)
        self.assertEqual(saved[0].read_text(), 'after')

    def test_restore_rejects_archive_traversal_before_touching_runtime(self):
        marker = self.runtime.root / 'marker'
        marker.write_text('keep')
        archive = self.config / 'malformed.tar'
        with tarfile.open(archive, 'w') as stream:
            item = tarfile.TarInfo('rootfs/../../escape')
            item.size = 4
            stream.addfile(item, io.BytesIO(b'evil'))
        with self.assertRaises((tarfile.FilterError, RuntimeError)):
            self.runtime.restore(archive)
        self.assertEqual(marker.read_text(), 'keep')
        self.assertFalse((self.runtime.state / 'escape').exists())

    def test_open_bridge_accepts_target_without_shell_parsing(self):
        ncat = self.config / '.local/bin/ncat'
        ncat.touch()
        ncat.chmod(0o755)
        output = self.config / 'opened.json'
        opener = self.config / 'fake-open'
        opener.write_text('#!/usr/bin/python3\nimport json,sys\n' +
                          'open(' + repr(str(output)) + ', "w").write(json.dumps(sys.argv[1:]))\n')
        opener.chmod(0o755)
        env = {}
        target = 'https://example.invalid/a;$(touch SHOULD_NOT_EXIST)'
        with patch.object(module.shutil, 'which', return_value=str(opener)):
            with self.runtime.open_bridge(env):
                with socket.socket(socket.AF_UNIX) as client:
                    client.connect(env['AGENT_WORKSPACE_DEB_OPEN_SOCKET'])
                    client.sendall(target.encode())
                    client.shutdown(socket.SHUT_WR)
                    self.assertEqual(client.recv(10), b'OK')
        deadline = time.monotonic() + 3
        while not output.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertEqual(json.loads(output.read_text()), [target])
        self.assertFalse(Path(env['AGENT_WORKSPACE_DEB_OPEN_SOCKET']).exists())

    def test_dry_run_never_initializes_or_registers_package(self):
        package = self.config / 'test.deb'
        package.touch()
        self.ready()
        with patch.object(self.runtime, 'metadata', return_value={'name': 'test-app'}), \
             patch.object(self.runtime, 'init') as init, patch.object(self.runtime, 'apt') as apt:
            self.runtime.install(package, simulate=True)
        init.assert_not_called()
        self.assertIn('--simulate', apt.call_args.args[0])
        self.assertFalse(self.runtime.manifest.exists())
        self.assertFalse(self.runtime.cache.exists())

    def test_failed_update_keeps_previous_launcher_record(self):
        package = self.config / 'test.deb'
        package.write_bytes(b'test')
        self.runtime.cache.mkdir(parents=True)
        module.atomic_json(self.runtime.manifest, {'packages': {
            'test-app': {'version': '1', 'launchers': ['previous.desktop']}}})
        with patch.object(self.runtime, 'metadata', return_value={'name': 'test-app', 'version': '2'}), \
             patch.object(self.runtime, 'init'), \
             patch.object(self.runtime, 'apt', side_effect=subprocess.CalledProcessError(1, ['apt-get'])):
            with self.assertRaises(subprocess.CalledProcessError):
                self.runtime.install(package, simulate=False)
        record = self.runtime.load()['packages']['test-app']
        self.assertEqual(record['status'], 'failed')
        self.assertEqual(record['launchers'], ['previous.desktop'])

    def test_remove_cleans_only_its_managed_entries(self):
        self.ready()
        self.runtime.launchers.mkdir(parents=True)
        own = 'agent-workspace-deb-test-app-123.desktop'
        (self.runtime.launchers / own).touch()
        unrelated = self.runtime.launchers / 'user.desktop'
        unrelated.write_text('keep')
        module.atomic_json(self.runtime.manifest, {'packages': {
            'test-app': {'launchers': [own, 'user.desktop', '../outside']}}})
        with patch.object(self.runtime, 'installed', return_value=(False, '')):
            self.runtime.sync()
        self.assertFalse((self.runtime.launchers / own).exists())
        self.assertEqual(unrelated.read_text(), 'keep')


if __name__ == '__main__':
    unittest.main()
