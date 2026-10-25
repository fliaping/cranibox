"""Offline package installation, process forwarding and resumable publication."""
import base64
from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/release"))
from release_common import PLATFORMS, check_binary, dev_version, sha256, verified_assets, version_for_tag


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"scripts/release/{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


builder = load("build-npm")
collector = load("collect-assets")
npm_publisher = load("publish-npm")
stage_approver = load("approve-staged")


FAKE_NPM = r'''
import base64
import hashlib
import json
from pathlib import Path
import sys

root = Path(__file__).parent
state_path = root / 'state.json'
state = json.loads(state_path.read_text())
args = sys.argv[1:]
logged = args.copy()
if '--otp' in logged:
    logged[logged.index('--otp') + 1] = '<redacted>'
with (root / 'calls.jsonl').open('a') as stream:
    stream.write(json.dumps(logged) + '\n')
payload = b'deterministic packed data'
integrity = 'sha512-' + base64.b64encode(hashlib.sha512(payload).digest()).decode()
if args[0] == 'pack':
    filename = Path.cwd().name + '.tgz'
    (Path(args[args.index('--pack-destination') + 1]) / filename).write_bytes(payload)
    print(json.dumps([{'filename': filename}]))
elif args[:2] == ['stage', 'list']:
    if state.get('list_error'):
        print(json.dumps({'error': {'code': state['list_error']}}))
        sys.exit(1)
    records = [item for item in state.get('stages', []) if item['name'] == args[2]]
    print(json.dumps(state.get('lists', {}).get(args[2], records)))
elif args[:2] == ['stage', 'view']:
    print(json.dumps(state['views'][args[2]]))
elif args[:2] == ['stage', 'publish']:
    package = Path(args[2])
    assert package.is_dir(), 'stage must receive an unpacked directory'
    if '--dry-run' not in args:
        manifest = json.loads((package / 'package.json').read_text())
        state.setdefault('stages', []).append(dict(
            id='stage-' + package.name, name=manifest['name'], version=manifest['version'],
            tag=args[args.index('--tag') + 1], integrity=integrity))
        state_path.write_text(json.dumps(state))
elif args[0] == 'stage' and args[1] in ('approve', 'reject'):
    assert args[args.index('--otp') + 1] == '123456'
    if state.get('fail_action'):
        # Deliberately hostile child output must never reach the operator.
        print(args[args.index('--otp') + 1])
        print(args[args.index('--otp') + 1], file=sys.stderr)
        sys.exit(1)
    assert any(item['id'] == args[2] for item in state['stages'])
    state['stages'] = [item for item in state['stages'] if item['id'] != args[2]]
    state_path.write_text(json.dumps(state))
else:
    sys.exit('Unexpected fake npm command')
'''


def fixtures(directory):
    directory.mkdir()
    for target, (filename, _, _) in PLATFORMS.items():
        header = bytearray(128)
        if target.startswith("linux"):
            header[:6] = b"\x7fELF\x02\x01"
            header[18:20] = (62).to_bytes(2, "little")
        elif target.startswith("darwin"):
            header[:8] = b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01"
        else:
            header[:2] = b"MZ"
            header[60:64] = (64).to_bytes(4, "little")
            header[64:70] = b"PE\0\0\x64\x86"
        (directory / filename).write_bytes(header)
    (directory / "Cranibox installer.dmg").write_bytes(b"fixture desktop installer")
    checksums(directory)
    return directory


def checksums(directory):
    (directory / "SHA256SUMS").write_text("".join(
        f"{sha256(file)}  {file.name}\n" for file in sorted(directory.iterdir()) if file.name != "SHA256SUMS"))


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cranibox packages ", dir=ROOT / "scripts/release")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.assets = fixtures(self.root / "assets")
        self.version = json.loads((ROOT / "npm/cli/package.json").read_text())["version"]
        self.tag = "v" + self.version

    def npm_environment(self):
        # Keep credentials, user configuration, caches and network out of fixtures.
        environment = {key: os.environ[key] for key in
                       ("PATH", "SystemRoot", "COMSPEC", "PATHEXT", "TMPDIR", "TEMP", "TMP") if key in os.environ}
        for name in ("user.npmrc", "global.npmrc"):
            (self.root / name).touch()
        environment.update(npm_config_update_notifier="false", npm_config_offline="true",
                           npm_config_cache=str(self.root / "cache"),
                           npm_config_userconfig=str(self.root / "user.npmrc"),
                           npm_config_globalconfig=str(self.root / "global.npmrc"))
        return environment

    def fake_npm(self, state=None):
        directory = self.root / "fake-npm"
        directory.mkdir(exist_ok=True)
        executable = directory / "npm"
        executable.write_text(f"#!{sys.executable}\n" + FAKE_NPM)
        executable.chmod(0o755)
        if os.name == "nt":
            (directory / "npm.cmd").write_text(f'@"{sys.executable}" "{executable}" %*\n')
        (directory / "state.json").write_text(json.dumps(state or {}))
        (directory / "calls.jsonl").write_text("")
        environment = self.npm_environment()
        environment["PATH"] = str(directory) + os.pathsep + environment.get("PATH", "")
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        return environment

    def fake_calls(self):
        return [json.loads(line) for line in (self.root / "fake-npm/calls.jsonl").read_text().splitlines()]

    def stage_fixtures(self, package_name="@fliaping/cranibox"):
        integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"deterministic packed data").digest()).decode()
        return [dict(id=f"stage-{index}", name=name, version="99.0.0-dev.42", tag="dev", integrity=integrity)
                for index, name in enumerate(stage_approver.package_names(package_name))]

    def publish_with_fake(self, output, environment, published=None, **kwargs):
        run = subprocess.run
        stdout = io.StringIO()
        with patch.object(npm_publisher.subprocess, "run", side_effect=lambda command, **options: run(command, env=environment, **options)), \
                patch.object(npm_publisher, "existing_integrity", return_value=published), redirect_stdout(stdout):
            npm_publisher.publish(output, mode="stage", **kwargs)
        return stdout.getvalue()

    def approve_with_fake(self, environment, *args):
        return subprocess.run([sys.executable, str(ROOT / "scripts/release/approve-staged.py"), *args],
                              env=environment, capture_output=True, text=True, timeout=10)

    def test_checksum_manifest_rejects_changed_extra_and_unsafe_files(self):
        verified_assets(self.assets)
        file = self.assets / "Cranibox installer.dmg"
        file.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            verified_assets(self.assets)
        checksums(self.assets)
        (self.assets / "unlisted").touch()
        with self.assertRaisesRegex(ValueError, "do not match"):
            verified_assets(self.assets)
        (self.assets / "SHA256SUMS").write_text("0" * 64 + "  ../outside\n")
        with self.assertRaisesRegex(ValueError, "Unsafe"):
            verified_assets(self.assets)

    def test_version_and_actual_architecture_are_checked(self):
        for invalid in ["main", "v01.2.3", "v1.2.3-beta.01", "v1.2.3+build", "v1.2.3;echo"]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                version_for_tag(invalid)
        self.assertEqual(version_for_tag("v1.2.3-beta.1"), "1.2.3-beta.1")
        with self.assertRaisesRegex(ValueError, "does not match"):
            check_binary(self.assets / "cranibox-macos-arm64", "linux-x64-gnu")
        with self.assertRaisesRegex(ValueError, "does not match"):
            builder.build(self.assets, self.root / "bad-version", "v99.0.0")

    def test_generated_packages_pin_versions_and_support_custom_scope(self):
        output = self.root / "npm"
        plan = builder.build(self.assets, output, self.tag, "@example/cranibox")
        self.assertEqual(plan[-1], "cli")
        manifest = json.loads((output / "cli/package.json").read_text())
        self.assertEqual(set(manifest["optionalDependencies"].values()), {self.version})
        self.assertNotIn("scripts", manifest)
        for target, package in manifest["craniboxPlatforms"].items():
            self.assertEqual(package, f"@example/cranibox-{target}")
            native = json.loads((output / f"cli-{target}/package.json").read_text())
            self.assertEqual(native["name"], package)
            self.assertEqual(native["version"], self.version)
            self.assertTrue((output / f"cli-{target}/LICENSE").is_file())
            self.assertNotIn("bin", native)
        with self.assertRaisesRegex(ValueError, "not empty"):
            builder.build(self.assets, output, self.tag)

    def test_dev_versions_sort_above_the_release_they_follow(self):
        self.assertEqual(dev_version("1.1.0", 7), "1.1.1-dev.7")
        self.assertEqual(dev_version("1.2.0-beta.1", 9), "1.2.0-beta.1.dev.9")
        for bad in ("1.1", "1.1.0+build"):
            with self.assertRaises(ValueError):
                dev_version(bad, 1)
        for bad in (0, "07", "x", -1):
            with self.assertRaises(ValueError):
                dev_version("1.1.0", bad)

    def test_version_override_pins_every_package_and_skips_source_version_check(self):
        output = self.root / "npm"
        version = "99.2.3-dev.42"
        plan = builder.build(self.assets, output, "v0.0.0", version=version)
        self.assertEqual(len(plan), 4)
        self.assertEqual(plan[-1], "cli")
        for entry in plan:
            manifest = json.loads((output / entry / "package.json").read_text())
            self.assertEqual(manifest["version"], version)
        self.assertEqual(manifest["optionalDependencies"],
                         {f"@fliaping/cranibox-{target}": version for target in PLATFORMS})

    def test_version_override_rejects_bad_semver_and_build_metadata(self):
        for version in ("", "v1.2.3", "1.2", "01.2.3", "1.2.3-dev.01", "1.2.3+build",
                        "1.2.3-dev.1+build", "1.2.3-", "1.2.3-dev..1", "1.2.3;echo", "1.２.3"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                builder.build(self.assets, self.root / "invalid", version=version)
        self.assertFalse((self.root / "invalid").exists())
        with self.assertRaisesRegex(ValueError, "Provide --tag or --version"):
            builder.build(self.assets, self.root / "invalid")

    def test_build_command_accepts_version_without_tag(self):
        output = self.root / "npm"
        result = subprocess.run([sys.executable, str(ROOT / "scripts/release/build-npm.py"),
                                 "--assets", str(self.assets), "--output", str(output),
                                 "--version", "99.0.0-dev.42"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((output / "cli/package.json").read_text())["version"], "99.0.0-dev.42")

    def test_collect_cli_artifacts_with_installers_and_licenses(self):
        artifacts = self.root / "artifacts"
        # These names are the upload-artifact contract, including Windows .exe.
        names = {"macos": "cranibox-macos-arm64", "linux": "cranibox-linux-x86_64",
                 "windows": "cranibox-windows-x86_64.exe"}
        for platform, filename in names.items():
            directory = artifacts / f"cranibox-cli-{platform}"
            directory.mkdir(parents=True)
            shutil.copyfile(self.assets / filename, directory / filename)
            (directory / filename).chmod(0o644)
        desktop = artifacts / "cranibox-desktop-macos-latest/bundle/dmg"
        desktop.mkdir(parents=True)
        shutil.copyfile(self.assets / "Cranibox installer.dmg", desktop / "Cranibox installer.dmg")
        for installers in (False, True):
            with self.subTest(installers=installers):
                output = self.root / f"collected-{installers}"
                collector.collect(artifacts, output, installers=installers)
                expected = set(names.values()) | {"LICENSE", "SHA256SUMS"}
                expected.update(p.name for p in (ROOT / "docs/licenses").glob("*.txt"))
                if installers:
                    expected.add("Cranibox installer.dmg")
                self.assertEqual({p.name for p in verified_assets(output)}, expected)
                for platform, filename in names.items():
                    self.assertEqual((output / filename).read_bytes(), (self.assets / filename).read_bytes())
                    if os.name != "nt" and platform != "windows":
                        self.assertEqual((output / filename).stat().st_mode & 0o111, 0o111)
                builder.build(output, self.root / f"npm-{installers}", version="99.0.0-dev.42")
        (artifacts / "cranibox-cli-windows" / names["windows"]).unlink()
        with self.assertRaisesRegex(ValueError, "Missing release asset.*cranibox-windows-x86_64.exe"):
            collector.collect(artifacts, self.root / "missing")
        self.assertFalse((self.root / "missing").exists())

    def test_npm_dist_tags_and_publication_order(self):
        for version, override, expected in (("99.0.0", None, "latest"),
                                            ("99.0.0-beta.1", None, "next"),
                                            ("99.0.0-dev.42", "dev", "dev")):
            with self.subTest(version=version):
                output = self.root / version
                plan = builder.build(self.assets, output, version=version)
                commands = []

                def run(command, **kwargs):
                    commands.append(command)
                    if command[1] == "pack":
                        name = Path(kwargs["cwd"]).name + ".tgz"
                        (output / name).write_bytes(b"packed data")
                        return subprocess.CompletedProcess(command, 0, json.dumps([{"filename": name}]))
                    return subprocess.CompletedProcess(command, 0)

                with patch.object(npm_publisher.subprocess, "run", side_effect=run), \
                        patch.object(npm_publisher, "existing_integrity", return_value=None):
                    npm_publisher.publish(output, dist_tag=override)
                publications = [command for command in commands if command[1] == "publish"]
                self.assertEqual([Path(command[2]).stem for command in publications], plan)
                self.assertEqual(len(publications), 4)
                for command in publications:
                    self.assertEqual(command[command.index("--tag") + 1], expected)
                    self.assertIn("--provenance", command)

    def test_npm_retry_verifies_identical_tarball_instead_of_overwriting(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            name = Path(kwargs["cwd"]).name + ".tgz"
            (output / name).write_bytes(b"deterministic packed data")
            return subprocess.CompletedProcess(command, 0, json.dumps([{"filename": name}]))

        integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"deterministic packed data").digest()).decode()
        with patch.object(npm_publisher.subprocess, "run", side_effect=run), patch.object(npm_publisher, "existing_integrity", return_value=integrity):
            npm_publisher.publish(output, dist_tag="dev")
        self.assertTrue(all(command[1] == "pack" for command in commands))
        with patch.object(npm_publisher.subprocess, "run", side_effect=run), patch.object(npm_publisher, "existing_integrity", return_value="sha512-different"):
            with self.assertRaisesRegex(ValueError, "different contents"):
                npm_publisher.publish(output, dist_tag="dev")

    def test_stage_publishes_directories_in_order_and_retry_skips(self):
        output = self.root / "npm"
        plan = builder.build(self.assets, output, version="99.0.0-dev.42")
        environment = self.fake_npm()
        summary = self.publish_with_fake(output, environment, dist_tag="dev")
        publications = [call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]
        self.assertEqual([Path(call[2]).name for call in publications], plan)
        for call in publications:
            self.assertTrue(Path(call[2]).is_dir())
            self.assertEqual(call[call.index("--tag") + 1], "dev")
            self.assertEqual(call[call.index("--access") + 1], "public")
            self.assertIn("--ignore-scripts", call)
            self.assertIn("--provenance", call)
        self.assertIn("approve-staged.py --package-name @fliaping/cranibox --approve", summary)
        self.assertIn("https://www.npmjs.com/package/@fliaping/cranibox", summary)
        summary = self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertEqual(len([call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]), 4)
        self.assertIn("Already staged and verified", summary)
        self.assertIn("npm stage approve stage-cli --otp <OTP>", summary)

    def test_stage_skip_published_requires_identical_integrity(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        environment = self.fake_npm()
        summary = self.publish_with_fake(output, environment, published=self.stage_fixtures()[0]["integrity"])
        self.assertTrue(all(call[0] == "pack" for call in self.fake_calls()))
        self.assertEqual(summary.count("Already published and verified"), 4)
        with self.assertRaisesRegex(ValueError, "different contents"):
            self.publish_with_fake(output, environment, published="sha512-different")

    def test_stage_rejects_conflicting_tag_or_contents(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        for field, value, message in (("tag", "latest", "already staged with tag"),
                                      ("integrity", "sha512-different", "different contents")):
            with self.subTest(field=field):
                stages = self.stage_fixtures()
                stages[0][field] = value
                environment = self.fake_npm({"stages": stages})
                with self.assertRaisesRegex(ValueError, message):
                    self.publish_with_fake(output, environment, dist_tag="dev")
                self.assertFalse(any(call[:2] == ["stage", "publish"] for call in self.fake_calls()))

    def test_stage_reads_view_for_missing_integrity_and_handles_wrapped_records(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        stages = self.stage_fixtures()
        lists = {stage["name"]: {"stages": [{"stageId": stage["id"],
                 "package": {"name": stage["name"], "version": stage["version"]}, "distTag": "dev"}]}
                 for stage in stages}
        views = {stage["id"]: {**stage, "integrity": None, "dist": {"integrity": stage["integrity"]}} for stage in stages}
        environment = self.fake_npm({"lists": lists, "views": views})
        self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertEqual(len([call for call in self.fake_calls() if call[:2] == ["stage", "view"]]), 4)
        self.assertFalse(any(call[:2] == ["stage", "publish"] for call in self.fake_calls()))

    def test_stage_unknown_metadata_and_missing_integrity_fail_closed(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        stage = self.stage_fixtures()[0]
        for listing in ({"unexpected": []}, None, ["bad-record"], [{**stage, "integrity": None}]):
            with self.subTest(listing=listing):
                environment = self.fake_npm({"lists": {stage["name"]: listing},
                                             "views": {stage["id"]: {**stage, "integrity": None}}})
                with self.assertRaises(ValueError):
                    self.publish_with_fake(output, environment, dist_tag="dev")
                self.assertFalse(any(call[:2] == ["stage", "publish"] for call in self.fake_calls()))

    def test_stage_bootstraps_not_found_but_fails_on_auth_error(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        environment = self.fake_npm({"list_error": "E404"})
        self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertEqual(len([call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]), 4)
        environment = self.fake_npm({"list_error": "E401"})
        with self.assertRaisesRegex(RuntimeError, "npm stage list failed"):
            self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertFalse(any(call[:2] == ["stage", "publish"] for call in self.fake_calls()))

    def test_stage_dry_run_is_offline_and_uses_default_tags(self):
        for version, tag in (("99.0.0", "latest"), ("99.0.0-beta.1", "next")):
            with self.subTest(version=version):
                output = self.root / version
                builder.build(self.assets, output, version=version)
                environment = self.fake_npm()
                # Invoke the CLI as well to cover --mode argument parsing.
                result = subprocess.run([sys.executable, str(ROOT / "scripts/release/publish-npm.py"),
                                         "--directory", str(output), "--mode", "stage", "--dry-run"],
                                        env=environment, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("nothing staged", result.stdout)
                self.assertNotIn("--approve", result.stdout)
                self.assertFalse(any(call[:2] in (["stage", "list"], ["stage", "view"]) for call in self.fake_calls()))
                publications = [call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]
                self.assertEqual(len(publications), 4)
                for call in publications:
                    self.assertIn("--dry-run", call)
                    self.assertIn("--offline", call)
                    self.assertNotIn("--provenance", call)
                    self.assertEqual(call[call.index("--tag") + 1], tag)

    def test_stage_partial_failure_writes_summary_with_exact_custom_package_name(self):
        name = "@example/tools-win32-x64"
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42", package_name=name)
        stages = self.stage_fixtures(name)[1:]
        stages[0]["tag"] = "next"
        environment = self.fake_npm({"stages": stages})
        summary_path = self.root / "summary.md"
        with patch.object(npm_publisher.os, "environ", {"GITHUB_STEP_SUMMARY": str(summary_path)}):
            with self.assertRaisesRegex(ValueError, "already staged with tag"):
                self.publish_with_fake(output, environment, dist_tag="dev")
        summary = summary_path.read_text()
        self.assertIn("stopped before completion", summary)
        self.assertIn(f"--package-name {name} --approve", summary)
        self.assertIn(f"{name}-darwin-arm64@99.0.0-dev.42: staged", summary)
        self.assertNotIn("No new stages", summary)

    def test_stage_different_version_does_not_skip_current_version(self):
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        stages = self.stage_fixtures()
        for stage in stages:
            stage["version"] = "99.0.0-dev.41"
        environment = self.fake_npm({"stages": stages})
        self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertEqual(len([call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]), 4)

    def test_stage_of_another_version_without_integrity_does_not_block(self):
        # An approved or superseded stage may no longer report integrity in npm stage view.
        output = self.root / "npm"
        builder.build(self.assets, output, version="99.0.0-dev.42")
        old = {**self.stage_fixtures()[0], "version": "99.0.0-dev.41", "integrity": None}
        unversioned = {**old, "id": "older-unversioned", "version": None}
        environment = self.fake_npm({"lists": {old["name"]: [old, unversioned]},
                                     "views": {old["id"]: old, unversioned["id"]: {**old, "id": unversioned["id"]}}})
        self.publish_with_fake(output, environment, dist_tag="dev")
        self.assertEqual(len([call for call in self.fake_calls() if call[:2] == ["stage", "publish"]]), 4)

    def test_approve_lists_only_expected_packages_without_action(self):
        stages = self.stage_fixtures()
        foreign = {**stages[0], "id": "unrelated", "name": "@other/package"}
        environment = self.fake_npm({"stages": stages + [foreign],
                                     "lists": {stages[0]["name"]: [stages[0], foreign]}})
        result = self.approve_with_fake(environment)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("@other/package", result.stdout)
        self.assertEqual(len(self.fake_calls()), 4)
        self.assertTrue(all(call[:2] == ["stage", "list"] for call in self.fake_calls()))
        for stage in stages:
            self.assertIn(stage["id"], result.stdout)

    def test_approve_and_reject_all_four_with_custom_scope_and_redacted_otp(self):
        for action in ("approve", "reject"):
            with self.subTest(action=action):
                stages = self.stage_fixtures("@example/tools")
                environment = self.fake_npm({"stages": stages})
                environment["CRANIBOX_NPM_PACKAGE"] = "@example/tools"
                result = self.approve_with_fake(environment, f"--{action}", "--otp", "123456")
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = self.fake_calls()
                self.assertEqual([call[2] for call in calls[:4]], [stage["name"] for stage in stages])
                self.assertEqual([call[2] for call in calls[4:]], [stage["id"] for stage in stages])
                self.assertTrue(all(call[:2] == ["stage", action] for call in calls[4:]))
                self.assertNotIn("123456", result.stdout + result.stderr + json.dumps(calls))

    def test_approve_prompts_once_after_listing_all_packages(self):
        stages = self.stage_fixtures()
        environment = self.fake_npm({"stages": stages})
        run = subprocess.run

        def prompt(message):
            self.assertEqual(len(self.fake_calls()), 4)
            return "123456"

        with patch.object(stage_approver.subprocess, "run", side_effect=lambda command, **options: run(command, env=environment, **options)), \
                patch.object(stage_approver.getpass, "getpass", side_effect=prompt) as getpass, redirect_stdout(io.StringIO()):
            stage_approver.manage("@fliaping/cranibox", "approve")
        getpass.assert_called_once()
        self.assertEqual(len(self.fake_calls()), 8)

    def test_approve_failure_does_not_leak_otp_from_child_output(self):
        environment = self.fake_npm({"stages": self.stage_fixtures(), "fail_action": True})
        result = self.approve_with_fake(environment, "--approve", "--otp", "123456")
        self.assertEqual(result.returncode, 1)
        self.assertIn("fresh OTP", result.stderr)
        self.assertNotIn("123456", result.stdout + result.stderr)
        self.assertEqual(len(self.fake_calls()), 5)

    def test_approve_rejects_unverifiable_identity_before_any_action(self):
        stage = self.stage_fixtures()[0]
        for detail in ({**stage, "name": "@other/package"}, {**stage, "id": "another-stage"}):
            with self.subTest(detail=detail):
                environment = self.fake_npm({"lists": {stage["name"]: [{"id": stage["id"]}]},
                                             "views": {stage["id"]: detail}})
                result = self.approve_with_fake(environment, "--approve", "--otp", "123456")
                self.assertEqual(result.returncode, 1)
                self.assertFalse(any(call[:2] == ["stage", "approve"] for call in self.fake_calls()))
                self.assertNotIn("123456", result.stdout + result.stderr)

    def test_approve_refuses_echoing_getpass_fallback(self):
        with patch.object(stage_approver, "list_stages", return_value=self.stage_fixtures()[:1]), \
                patch.object(stage_approver.getpass, "getpass", side_effect=stage_approver.getpass.GetPassWarning), \
                patch.object(stage_approver.subprocess, "run") as run, redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "terminal is required"):
                stage_approver.manage("@fliaping/cranibox", "approve")
        run.assert_not_called()

    def test_approve_empty_list_needs_no_otp_and_actions_are_exclusive(self):
        environment = self.fake_npm()
        result = self.approve_with_fake(environment, "--approve")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No staged", result.stdout)
        result = self.approve_with_fake(environment, "--approve", "--reject")
        self.assertEqual(result.returncode, 2)

    @unittest.skipUnless(shutil.which("npm"), "requires npm")
    def test_real_npm_dry_run_is_offline_and_reproducible_after_artifact_download(self):
        output = self.root / "npm"
        plan = builder.build(self.assets, output, version="99.0.0-dev.42")
        env = self.npm_environment()
        command = [sys.executable, str(ROOT / "scripts/release/publish-npm.py"), "--directory", str(output), "--dist-tag", "dev", "--dry-run"]
        for attempt in range(2):
            if attempt:
                for entry in plan:
                    for file in (output / entry / "bin").iterdir():
                        file.chmod(0o644)
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            digests = {file.name: sha256(file) for file in output.glob("*.tgz")}
            self.assertEqual(len(digests), 4)
            if attempt == 0:
                original = digests
            else:
                self.assertEqual(original, digests)
        for archive in output.glob("*.tgz"):
            with tarfile.open(archive) as package:
                self.assertIn("package/LICENSE", package.getnames())
                self.assertTrue(any(name.startswith("package/licenses/") for name in package.getnames()))
                for member in package.getmembers():
                    if member.name.startswith("package/bin/") and member.isfile():
                        self.assertEqual(member.mode & 0o111, 0o111)

    @unittest.skipUnless(sys.platform == "linux" and shutil.which("npm") and shutil.which("node"), "requires Linux, Node and npm")
    def test_npm_install_without_scripts_or_external_downloads_and_stdio_signals(self):
        output = self.root / "npm"
        plan = builder.build(self.assets, output, self.tag)
        # Replace only the test executable with a small process fixture. Native
        # architecture validation was exercised by build() above.
        binary = output / "cli-linux-x64-gnu/bin/cranibox"
        binary.write_text('''#!/usr/bin/env node
const fs = require('node:fs');
if (process.argv[2] === '--wait') {
  process.on('SIGTERM', () => process.exit(42));
  console.log('READY'); setInterval(() => {}, 1000);
} else if (process.argv[2] === '--exit') {
  process.exit(7);
} else {
  console.log(JSON.stringify({args: process.argv.slice(2), input: fs.readFileSync(0, 'utf8')}));
}
''')
        environment = self.npm_environment()
        tarballs = {}
        for entry in plan:
            package = output / entry
            # Simulate Actions losing permissions and verify the publisher restores them.
            for file in (package / "bin").iterdir():
                file.chmod(0o755)
            result = subprocess.run(["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", str(output)],
                                    cwd=package, env=environment, text=True, capture_output=True, check=True)
            packed = json.loads(result.stdout)
            packed = (list(packed.values()) if isinstance(packed, dict) else packed)[0]  # npm 12 keys by name
            manifest = json.loads((package / "package.json").read_text())
            tarballs[manifest["name"]] = output / packed["filename"]

        # Install the local native and entry archives together. --offline also
        # prevents registry lookups for the other platforms' optional packages.
        prefix = self.root / "installed"
        result = subprocess.run(["npm", "install", "--prefix", str(prefix), "--ignore-scripts", "--include=optional", "--no-audit", "--no-fund", "--offline",
                                 str(tarballs["@fliaping/cranibox-linux-x64-gnu"]), str(tarballs["@fliaping/cranibox"])],
                                env=environment, text=True, capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        command = prefix / "node_modules/.bin/cranibox"
        result = subprocess.run([str(command), "argument with spaces", "中文"], input="MCP input\n", capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"args": ["argument with spaces", "中文"], "input": "MCP input\n"})
        self.assertEqual(subprocess.run([str(command), "--exit"], timeout=10).returncode, 7)
        with subprocess.Popen([str(command), "--wait"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            try:
                self.assertEqual(process.stdout.readline().strip(), "READY")
                process.send_signal(signal.SIGTERM)
                self.assertEqual(process.wait(timeout=10), 42)
            finally:
                if process.poll() is None:
                    process.kill()
        native = prefix / "node_modules/@fliaping/cranibox-linux-x64-gnu/package.json"
        data = json.loads(native.read_text())
        data["version"] = "0.0.0"
        native.write_text(json.dumps(data))
        result = subprocess.run([str(command), "--help"], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertIn("Version mismatch", result.stderr)
        shutil.rmtree(native.parent)
        result = subprocess.run([str(command)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertIn("--include=optional", result.stderr)


if __name__ == "__main__":
    unittest.main()
