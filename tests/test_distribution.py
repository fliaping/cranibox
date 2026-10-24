"""Offline package installation, process forwarding and resumable publication."""
import base64
import hashlib
import importlib.util
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
from release_common import PLATFORMS, check_binary, sha256, verified_assets, version_for_tag


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"scripts/release/{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


builder = load("build-npm")
collector = load("collect-assets")
npm_publisher = load("publish-npm")


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
            packed = json.loads(result.stdout)[0]
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
