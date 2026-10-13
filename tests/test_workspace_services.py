"""Exercise the native manager with real s6 and an isolated HOME."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SOURCE = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('WORKSPACE_SERVICES_BIN', SOURCE / 'target/debug/workspace-services'))


def until(check, seconds=6):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = check()
        if value:
            return value
        time.sleep(.03)
    raise AssertionError('Timed out waiting for service transition')


@unittest.skipUnless(sys.platform.startswith('linux') and shutil.which('s6-svscan'), 'Linux/s6 required')
class ServiceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='workspace-services-test-')
        self.home = Path(self.temp.name)
        self.units = self.home / '.config/systemd/user'
        self.units.mkdir(parents=True)
        self.env = {**os.environ, 'HOME': str(self.home)}
        self.state = self.home / '.local/run/user-systemd'

    def command(self, *args, check=True):
        return subprocess.run([str(BINARY), '--user', *args], env=self.env,
                              text=True, capture_output=True, check=check, timeout=15)

    def unit(self, command, restart='no', delay='.1', extra=''):
        (self.units / 'demo.service').write_text(
            '[Unit]\nDescription=Isolated demo\n[Service]\nExecStart=' + command +
            '\nRestart=' + restart + '\nRestartSec=' + delay + '\nTimeoutStopSec=.5\n' + extra)

    def python(self, source):
        script = self.home / 'app.py'
        script.write_text(source)
        return f'{sys.executable} "{script}"'

    def tearDown(self):
        try:
            for unit in self.units.glob('*.service'):
                self.command('stop', unit.name, check=False)
            scan = self.home / '.local/run/workspace-services/services'
            if (scan / '.s6-svscan/control').exists():
                for service in scan.iterdir():
                    if (service / 'supervise/control').exists():
                        subprocess.run(['s6-svc', '-x', str(service)], timeout=5, capture_output=True)
                subprocess.run(['s6-svscanctl', '-t', str(scan)], timeout=5, capture_output=True)
                until(lambda: subprocess.run(['s6-svscanctl', '-a', str(scan)],
                      capture_output=True).returncode != 0, 6)
        finally:
            self.temp.cleanup()

    def test_start_enable_show_stop(self):
        self.unit('/bin/sleep 60')
        self.command('enable', '--now', 'demo.service')
        self.assertEqual(self.command('is-active', 'demo').stdout.strip(), 'active')
        self.assertEqual(self.command('is-enabled', 'demo').stdout.strip(), 'enabled')
        pid = int((self.state / 'demo.pid').read_text())
        until(lambda: Path(f'/proc/{pid}/comm').read_text().strip() == 'sleep')
        supervisor = int((self.state / 'demo.supervisor.pid').read_text())
        self.assertEqual(Path(f'/proc/{supervisor}/comm').read_text().strip(), 's6-supervise')
        self.assertIn('MainPID=' + str(pid), self.command('show', 'demo', '-p', 'MainPID').stdout)
        self.command('disable', '--now', 'demo')
        self.assertEqual(self.command('is-active', 'demo', check=False).returncode, 3)

    def test_restart_policy_failure_and_clean_exit(self):
        marker = self.home / 'count'
        self.unit(self.python('from pathlib import Path\np=Path(' + repr(str(marker)) +
                              ')\nn=int(p.read_text()) if p.exists() else 0\np.write_text(str(n+1))\nraise SystemExit(1 if n<2 else 0)\n'), 'on-failure')
        self.command('start', 'demo')
        until(lambda: marker.exists() and marker.read_text() == '3')
        until(lambda: (self.state / 'demo.exit.json').exists() and
              json.loads((self.state / 'demo.exit.json').read_text())['code'] == 0)
        time.sleep(.5)
        self.assertEqual(marker.read_text(), '3')

    def test_stop_interrupts_restart_delay(self):
        marker = self.home / 'count'
        self.unit(self.python('from pathlib import Path\nPath(' + repr(str(marker)) +
                              ').write_text("started")\nraise SystemExit(1)\n'), 'always', '20')
        self.command('start', 'demo')
        until(lambda: (self.state / 'demo.exit.json').exists())
        start = time.monotonic()
        self.command('stop', 'demo')
        self.assertLess(time.monotonic() - start, 4)

    def test_snapshot_quotes_environment_file_and_env_i(self):
        self.env['TZ'] = 'America/Phoenix'
        self.env['LANG'] = 'snapshot language'
        self.command('__snapshot-env')
        self.env['LANG'] = 'caller language'
        output = self.home / 'environment.json'
        command = self.python('import json,os,time\nfrom pathlib import Path\nPath(' +
                              repr(str(output)) + ').write_text(json.dumps(dict(os.environ)))\ntime.sleep(60)\n')
        envfile = self.home / 'vars'
        envfile.write_text('VALUE="two words"\n')
        self.unit('/usr/bin/env -i HOME="' + str(self.home) + '" LANG="explicit language" ' + command,
                  extra='EnvironmentFile="' + str(envfile) + '"\nEnvironment="TZ=Europe/Berlin"\n')
        self.command('start', 'demo')
        until(output.exists)
        environment = json.loads(output.read_text())
        self.assertEqual(environment['LANG'], 'explicit language')
        self.assertEqual(environment['TZ'], 'Europe/Berlin')
        self.assertNotIn('VALUE', environment)

    def test_stop_kills_detached_descendant(self):
        childpid = self.home / 'child'
        self.unit(self.python('import subprocess,time\nfrom pathlib import Path\np=subprocess.Popen(["sleep","60"],start_new_session=True)\nPath(' +
                              repr(str(childpid)) + ').write_text(str(p.pid))\ntime.sleep(60)\n'))
        self.command('start', 'demo')
        until(childpid.exists)
        pid = int(childpid.read_text())
        self.command('stop', 'demo')
        def dead():
            path = Path(f'/proc/{pid}/stat')
            return not path.exists() or path.read_text().rsplit(') ', 1)[1].startswith('Z ')
        until(dead)

    def test_parallel_start_and_reject_invalid_units(self):
        self.unit('/bin/sleep 60')
        jobs = [subprocess.Popen([str(BINARY), '--user', 'start', 'demo'], env=self.env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(4)]
        for job in jobs:
            out, err = job.communicate(timeout=10)
            self.assertEqual(job.returncode, 0, (out, err))
        self.assertNotEqual(self.command('start', '../invalid', check=False).returncode, 0)
        self.command('stop', 'demo')
        self.unit('/bin/sleep 60', extra='ExecStartPost=/bin/true\n')
        self.assertNotEqual(self.command('start', 'demo', check=False).returncode, 0)

    def test_stale_pid_identity_is_not_signaled(self):
        self.unit('/bin/sleep 60')
        self.state.mkdir(parents=True)
        other = subprocess.Popen(['sleep', '60'])
        try:
            (self.state / 'demo.pid').write_text(str(other.pid))
            (self.state / 'demo.starttime').write_text('invalid-starttime')
            (self.state / 'demo.supervisor.pid').write_text(str(other.pid))
            self.command('stop', 'demo')
            self.assertIsNone(other.poll())
            # Old managers did not record starttime; reused PIDs must be ignored.
            (self.state / 'demo.pid').write_text(str(other.pid))
            (self.state / 'demo.supervisor.pid').write_text(str(other.pid))
            self.command('stop', 'demo')
            self.assertIsNone(other.poll())
        finally:
            other.terminate()
            other.wait()

    def test_pre_command_and_success_status(self):
        marker = self.home / 'pre.complete'
        self.unit('/bin/sh -c "exit 143"', 'on-failure',
                  extra=f'ExecStartPre=/usr/bin/touch "{marker}"\nSuccessExitStatus=0 143\n')
        self.command('start', 'demo')
        until(marker.exists)
        until(lambda: (self.state / 'demo.exit.json').exists())
        self.assertEqual(json.loads((self.state / 'demo.exit.json').read_text())['code'], 143)
        time.sleep(.4)
        self.assertEqual(self.command('is-active', 'demo', check=False).returncode, 3)

    def test_restart_migrates_legacy_supervisor(self):
        self.unit('/bin/sleep 60', 'always')
        subprocess.run(['bash', str(SOURCE / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', 'demo'],
                       env=self.env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        old = int((self.state / 'demo.supervisor.pid').read_text())
        self.command('restart', 'demo')
        new = int((self.state / 'demo.supervisor.pid').read_text())
        self.assertNotEqual(old, new)
        self.assertEqual(Path(f'/proc/{new}/comm').read_text().strip(), 's6-supervise')
        def old_dead():
            path = Path(f'/proc/{old}/stat')
            return not path.exists() or path.read_text().rsplit(') ', 1)[1].startswith('Z ')
        until(old_dead)

    def test_setsid_service_migration_tracks_real_process_and_releases_port(self):
        marker = self.home / 'listener.json'
        command = self.python('import json,os,socket,time\nfrom pathlib import Path\ns=socket.socket()\ns.bind(("127.0.0.1",0))\ns.listen()\nPath(' +
                              repr(str(marker)) + ').write_text(json.dumps({"pid":os.getpid(),"port":s.getsockname()[1]}))\ntime.sleep(60)\n')
        self.unit('/usr/bin/setsid ' + command.replace('"', ''), 'always')
        subprocess.run(['bash', str(SOURCE / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', 'demo'],
                       env=self.env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        until(marker.exists)
        old = json.loads(marker.read_text())
        report = json.loads(self.command('migrate', '--apply').stdout)
        self.assertTrue(report['completed'])
        until(lambda: json.loads(marker.read_text())['pid'] != old['pid'])
        new = json.loads(marker.read_text())
        tracked = int((self.state / 'demo.pid').read_text())
        self.assertEqual(tracked, new['pid'])
        self.assertEqual(os.getpgid(tracked), tracked)
        time.sleep(.4)
        self.assertEqual(int((self.state / 'demo.pid').read_text()), tracked)
        self.command('stop', 'demo')
        import socket
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', new['port']))
            probe.listen()
        self.assertEqual(self.command('is-active', 'demo', check=False).returncode, 3)

    def test_migration_plan_is_read_only(self):
        self.unit('/bin/sleep 60')
        plan = json.loads(self.command('migration-plan').stdout)
        self.assertTrue(plan['ready'])
        self.assertFalse(self.state.exists())
        self.assertFalse((self.home / '.local/run/workspace-services').exists())

    def test_batch_migration_preserves_enabled_flags(self):
        self.unit('/bin/sleep 60', 'always')
        (self.units / 'second.service').write_text((self.units / 'demo.service').read_text())
        for name in ['demo', 'second']:
            subprocess.run(['bash', str(SOURCE / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', name],
                           env=self.env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        self.command('enable', 'demo')
        plan = json.loads(self.command('migration-plan').stdout)
        self.assertEqual(sum(row['candidate'] for row in plan['services']), 2)
        report = json.loads(self.command('migrate', '--apply').stdout)
        self.assertTrue(report['completed'])
        self.assertTrue(all(row['success'] for row in report['results']))
        self.assertEqual(self.command('is-enabled', 'demo').stdout.strip(), 'enabled')
        self.assertEqual(self.command('is-enabled', 'second', check=False).stdout.strip(), 'disabled')
        for name in ['demo', 'second']:
            supervisor = int((self.state / (name + '.supervisor.pid')).read_text())
            self.assertEqual(Path(f'/proc/{supervisor}/comm').read_text().strip(), 's6-supervise')
        self.assertEqual(json.loads(self.command('migration-plan').stdout)['services'][0]['candidate'], False)
        self.assertEqual((self.state / 'migration.json').stat().st_mode & 0o777, 0o600)

    def test_failed_migration_rolls_back_and_leaves_remaining_services(self):
        marker = self.home / 'fail-native'
        script = self.home / 'conditional.sh'
        script.write_text('#!/bin/sh\nif [ -f "' + str(marker) + '" ]; then exit 42; fi\nexec sleep 60\n')
        script.chmod(0o700)
        self.unit(str(script), 'always')
        (self.units / 'second.service').write_text('[Service]\nExecStart=/bin/sleep 60\nRestart=always\n')
        for name in ['demo', 'second']:
            subprocess.run(['bash', str(SOURCE / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', name], env=self.env,
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        second_pid = (self.state / 'second.pid').read_text()
        marker.touch()
        rollback = self.home / 'rollback.sh'
        rollback.write_text('#!/bin/sh\nrm -f "' + str(marker) + '"\nexec bash "' + str(SOURCE / 'scripts/user-systemctl-legacy.sh') + '" "$@"\n')
        result = self.command('migrate', '--apply', '--rollback-manager=' + str(rollback), check=False)
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(result.stdout)
        self.assertFalse(report['completed'])
        self.assertEqual(len(report['results']), 1)
        self.assertFalse(report['results'][0]['success'])
        self.assertTrue(report['results'][0]['rollback_started'])
        self.assertTrue(report['results'][0]['rollback_verified'])
        self.assertEqual(self.command('is-active', 'demo').stdout.strip(), 'active')
        self.assertEqual((self.state / 'second.pid').read_text(), second_pid)
        restored_supervisor = int((self.state / 'demo.supervisor.pid').read_text())
        self.command('stop', 'demo')
        def restored_stopped():
            path = Path(f'/proc/{restored_supervisor}/stat')
            return not path.exists() or path.read_text().rsplit(') ', 1)[1].startswith('Z ')
        until(restored_stopped)

    def test_migration_preflight_leaves_legacy_service_running(self):
        self.unit('/bin/sleep 60', 'always')
        subprocess.run(['bash', str(SOURCE / 'scripts/user-systemctl-legacy.sh'), '--user', 'start', 'demo'],
                       env=self.env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        old = (self.state / 'demo.pid').read_text()
        self.unit('/bin/sleep 60', 'always', extra='ExecStartPost=/bin/true\n')
        self.assertNotEqual(self.command('migrate', '--apply', check=False).returncode, 0)
        self.assertEqual((self.state / 'demo.pid').read_text(), old)
        self.assertEqual(self.command('is-active', 'demo').stdout.strip(), 'active')
        plan = json.loads(self.command('migration-plan', '--exclude=demo').stdout)
        self.assertTrue(plan['ready'])


if __name__ == '__main__':
    unittest.main()
