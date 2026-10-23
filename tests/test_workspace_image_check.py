"""Release gate regression tests; Docker is mocked, startup is tested separately."""
import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'workspace_image_check', Path(__file__).resolve().parents[1] / 'scripts/verify-workspace-image.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
REVISION = 'a' * 40


def healthy_probe():
    return {
        'health_9080': {'status': 200, 'value': {'ok': True, 'implementation': 'rust'}},
        'web_status': 200, 'peers_status': 200, 'api_node_id': 'node-test', 'node_id': 'node-test',
        'static_assets': {'/web.js': 200, '/web.css': 200},
        'versions': {name: {'exit_code': 0, 'version': 'test'}
                     for name in ('cranibox', 'workspacectl', 'herdr')},
        'aliases': {name: True for name in ('workspacectl', 'workspace-services')},
        'image_binary_matches': True,
        'service': {name: {'exit_code': 0} for name in ('is-enabled', 'is-active')},
        'credentials_mode': 0o600, 'peers_mode': 0o600,
        'desktop_anonymous_status': 401, 'desktop_authenticated_status': 200,
        'applications_root_status': 404,
        'enrollment': {'status': 200, 'node_ids': ['node-test']},
        'invalid_token_status': 401,
        'execution': {'status': 200, 'exit_code': 0, 'stdout': 'cranibox-image-exec-ok', 'outcome': 'completed'},
    }


class ImageReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.report = Path(self.temporary.name) / 'acceptance.json'
        self.check = CHECK.ImageCheck(SimpleNamespace(
            image='local-candidate:test', revision=REVISION, report=self.report, timeout=30, sudo=False,
            platform=None, pull=False))

    def image(self):
        return {'Id': 'sha256:test', 'Os': 'linux', 'Architecture': 'amd64', 'Config': {'Labels': {
            'io.cranibox.space-protocol': '1', 'org.opencontainers.image.revision': REVISION}}}

    def run_check(self, image, cleanup=True):
        with patch.object(self.check, 'inspect', return_value=image), \
             patch.object(self.check, 'scenario') as scenario, \
             patch.object(self.check, 'cleanup', return_value=cleanup), \
             contextlib.redirect_stdout(io.StringIO()):
            result = self.check.run()
        return result, scenario

    def test_missing_protocol_rejects_before_creating_resources(self):
        image = self.image()
        del image['Config']['Labels']['io.cranibox.space-protocol']
        result, scenario = self.run_check(image)
        self.assertEqual(result, 1)
        scenario.assert_not_called()
        self.assertEqual(json.loads(self.report.read_text())['checks'][0],
                         {'name': 'space-protocol', 'passed': False})

    def test_wrong_revision_rejects_before_creating_resources(self):
        image = self.image()
        image['Config']['Labels']['org.opencontainers.image.revision'] = 'b' * 40
        result, scenario = self.run_check(image)
        self.assertEqual(result, 1)
        scenario.assert_not_called()

    def test_success_requires_both_fresh_and_enrolled_scenarios(self):
        result, scenario = self.run_check(self.image())
        self.assertEqual(result, 0)
        self.assertEqual([call.args[0] for call in scenario.call_args_list], ['fresh', 'space'])
        self.assertTrue(json.loads(self.report.read_text())['passed'])
        self.assertEqual(self.report.stat().st_mode & 0o777, 0o600)

    def test_failed_cleanup_cannot_report_success(self):
        result, _ = self.run_check(self.image(), cleanup=False)
        self.assertEqual(result, 1)
        self.assertFalse(json.loads(self.report.read_text())['passed'])

    def test_wrong_architecture_rejects_before_creating_resources(self):
        self.check.args.platform = 'linux/arm64'
        result, scenario = self.run_check(self.image())
        self.assertEqual(result, 1)
        scenario.assert_not_called()
        self.assertEqual(json.loads(self.report.read_text())['checks'][-1],
                         {'name': 'requested-platform', 'passed': False})

    def test_arm64_image_accepts_the_requested_platform(self):
        self.check.args.platform = 'linux/arm64'
        image = self.image()
        image['Architecture'] = 'arm64'
        result, scenario = self.run_check(image)
        self.assertEqual(result, 0)
        self.assertEqual(scenario.call_count, 2)
        self.assertEqual(json.loads(self.report.read_text())['platform'], 'linux/arm64')

    def test_registry_selection_uses_exact_platform_digest(self):
        self.check.args.platform = 'linux/arm64'
        self.check.args.pull = True
        self.check.args.image = 'registry.test/workspace@sha256:' + 'a' * 64
        selected = 'registry.test/workspace@sha256:' + 'c' * 64
        index = {'manifests': [
            {'platform': {'os': 'linux', 'architecture': 'amd64'}, 'digest': 'sha256:' + 'b' * 64},
            {'platform': {'os': 'linux', 'architecture': 'arm64', 'variant': 'v8'},
             'digest': 'sha256:' + 'c' * 64},
            {'platform': {'os': 'unknown', 'architecture': 'unknown'}, 'digest': 'sha256:' + 'd' * 64},
        ]}
        with patch.object(self.check, 'docker', side_effect=[json.dumps(index), '']) as docker:
            self.assertEqual(self.check.image_reference(), selected)
        self.assertEqual(docker.call_args_list[-1].args, ('pull', '--platform', 'linux/arm64', selected))
        self.assertEqual(json.loads(self.report.read_text())['platform_image'], selected)

    def test_invalid_platform_index_never_pulls_or_creates_resources(self):
        self.check.args.platform = 'linux/arm64'
        self.check.args.pull = True
        manifest = {'platform': {'os': 'linux', 'architecture': 'arm64'}, 'digest': 'sha256:' + 'c' * 64}
        for manifests in ([], [manifest, manifest], [{**manifest, 'digest': 'not-a-digest'}]):
            with self.subTest(manifests=manifests), \
                 patch.object(self.check, 'docker', return_value=json.dumps({'manifests': manifests})) as docker, \
                 patch.object(self.check, 'scenario') as scenario, \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(self.check.run(), 1)
                scenario.assert_not_called()
                self.assertEqual(docker.call_count, 1)

    def test_non_index_registry_responses_are_rejected(self):
        for index in (None, [], {}, {'manifests': None}, {'manifests': [None, {'platform': None}]}):
            with self.subTest(index=index), self.assertRaises(ValueError):
                CHECK.ImageCheck.platform_manifest(index, 'linux/arm64')

    def test_pull_requires_an_explicit_platform_and_immutable_index(self):
        for arguments in ([], ['--platform', 'linux/arm64'],
                          ['--platform', 'linux/arm64', '--image', 'registry.test/image@sha256:short']):
            with self.subTest(arguments=arguments), \
                 patch.object(sys, 'argv', ['verify-workspace-image.py', '--pull',
                     '--image', 'registry.test/image:mutable', '--report', str(self.report), *arguments]), \
                 patch.object(CHECK, 'ImageCheck') as checker, contextlib.redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as error:
                CHECK.main()
            self.assertEqual(error.exception.code, 2)
            checker.assert_not_called()

    def test_seed_and_workspace_containers_both_use_requested_platform(self):
        self.check.platform_args = ['--platform', 'linux/arm64']
        name = 'cranibox-image-check-' + self.check.identity[:12] + '-space'
        probe = {'node_id': 'node-test', 'credentials_sha256': 'credentials', 'sentinel_sha256': 'marker'}
        container = {'Image': 'sha256:test', 'HostConfig': {'Privileged': False},
                     'Mounts': [{'Name': name+'-config', 'Destination': '/config'}]}
        with patch.object(self.check, 'docker', return_value='marker') as docker, \
             patch.object(self.check, 'inspect', return_value=container), \
             patch.object(self.check, 'wait', return_value=probe), contextlib.redirect_stdout(io.StringIO()):
            self.check.scenario('space', 'sha256:test', Path(self.temporary.name))
        runs = [call.args for call in docker.call_args_list if call.args[0] == 'run']
        self.assertEqual(len(runs), 2)
        self.assertTrue(all(command[1:3] == ('--platform', 'linux/arm64') for command in runs))

    def test_loaded_image_does_not_implicitly_contact_registry(self):
        with patch.object(self.check, 'docker') as docker:
            self.assertEqual(self.check.image_reference(), self.check.args.image)
        docker.assert_not_called()

    def test_foreign_resources_are_never_removed(self):
        self.check.resources = [('volume', 'foreign')]
        with patch.object(self.check, 'inspect', return_value={'Labels': {CHECK.LABEL: 'other'}}), \
             patch.object(self.check, 'docker') as docker:
            self.assertFalse(self.check.cleanup())
        docker.assert_not_called()

    def test_owned_resources_are_removed_in_reverse_order(self):
        self.check.resources = [('volume', 'owned-volume'), ('container', 'owned-container')]
        def inspect(kind, name):
            labels = {CHECK.LABEL: self.check.identity}
            return {'Config': {'Labels': labels}} if kind == 'container' else {'Labels': labels}
        with patch.object(self.check, 'inspect', side_effect=inspect), \
             patch.object(self.check, 'docker') as docker:
            self.assertTrue(self.check.cleanup())
        self.assertEqual([call.args for call in docker.call_args_list],
                         [('container', 'rm', '--force', 'owned-container'), ('volume', 'rm', 'owned-volume')])

    def test_healthy_probe_accepts_fresh_and_seeded_identity(self):
        probe = healthy_probe()
        for credentials in ({}, {'node_id': 'node-test', 'token': 'private-token'}):
            self.assertTrue(all(CHECK.ImageCheck.conditions(probe, credentials).values()))

    def test_missing_assets_programs_and_service_checks_fail(self):
        for key, check in [('static_assets', 'web-and-node'), ('versions', 'built-in-programs'),
                           ('aliases', 'built-in-programs'), ('service', 'default-service')]:
            with self.subTest(key=key):
                probe = healthy_probe()
                probe[key] = {}
                self.assertFalse(CHECK.ImageCheck.conditions(probe, {})[check])

    def test_wrong_identity_auth_execution_and_modes_fail(self):
        changes = [
            ('node_id', 'other', 'seeded-identity'),
            ('enrollment', {'status': 200, 'node_ids': ['other']}, 'enrollment'),
            ('invalid_token_status', 200, 'peer-auth'),
            ('execution', {'status': 200, 'exit_code': 1, 'stdout': 'cranibox-image-exec-ok'}, 'peer-execution'),
            ('credentials_mode', 0o644, 'private-credentials'),
            ('desktop_anonymous_status', 200, 'desktop-auth'),
            ('image_binary_matches', False, 'built-in-programs'),
        ]
        for key, value, check in changes:
            with self.subTest(key=key):
                probe = copy.deepcopy(healthy_probe())
                probe[key] = value
                self.assertFalse(CHECK.ImageCheck.conditions(probe, {'node_id': 'node-test'})[check])

    def test_subprocess_failure_never_leaks_private_output(self):
        result = SimpleNamespace(returncode=1, stdout='PRIVATE_STDOUT', stderr='PRIVATE_STDERR')
        with patch.object(CHECK.subprocess, 'run', return_value=result):
            with self.assertRaises(RuntimeError) as error:
                self.check.docker('exec', 'private-command', data='PRIVATE_TOKEN')
        self.assertEqual(str(error.exception), 'Docker action failed: exec')

    def test_scenario_failure_still_cleans_up_without_logging_exception_input(self):
        output = io.StringIO()
        with patch.object(self.check, 'inspect', return_value=self.image()), \
             patch.object(self.check, 'scenario', side_effect=ValueError('PRIVATE_TOKEN')), \
             patch.object(self.check, 'cleanup', return_value=True) as cleanup, \
             contextlib.redirect_stdout(output):
            result = self.check.run()
        self.assertEqual(result, 1)
        cleanup.assert_called_once()
        self.assertEqual(json.loads(self.report.read_text())['error_type'], 'ValueError')
        self.assertNotIn('PRIVATE_TOKEN', self.report.read_text() + output.getvalue())

    def test_healthy_container_must_also_pass_application_authorization(self):
        probe = healthy_probe()
        unready = copy.deepcopy(probe)
        unready['invalid_token_status'] = 200
        with patch.object(self.check, 'inspect', return_value={
                'State': {'Running': True, 'Health': {'Status': 'healthy'}}}), \
             patch.object(self.check, 'docker', side_effect=[json.dumps(unready), json.dumps(probe)]) as docker, \
             patch.object(CHECK.time, 'sleep'), contextlib.redirect_stdout(io.StringIO()):
            result = self.check.wait('owned', {'node_id': 'node-test', 'token': 'PRIVATE_TOKEN'}, 'first')
        self.assertEqual(result, probe)
        self.assertEqual(docker.call_count, 2)
        self.assertNotIn('PRIVATE_TOKEN', self.report.read_text())

    def test_existing_report_is_private_before_write(self):
        self.report.write_text('old report')
        self.report.chmod(0o644)
        self.check.save()
        self.assertEqual(self.report.stat().st_mode & 0o777, 0o600)

    def test_probe_compiles_without_running(self):
        compile(CHECK.PROBE, '<image-probe>', 'exec')


if __name__ == '__main__':
    unittest.main()
