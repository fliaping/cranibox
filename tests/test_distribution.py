"""Offline package installation, process forwarding and resumable publication."""
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
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
import threading
import unittest
from unittest.mock import patch
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/release"))
from release_common import PLATFORMS, check_binary, sha256, verified_assets, version_for_tag


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"scripts/release/{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


builder = load("build-npm")
publisher = load("publish-gitcode")
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


class FakeGitCode:
    def __init__(self):
        self.value = None
        self.files = {}
        self.statuses = []
        self.uploaded = []
        self.fail_on = None

    def release(self, tag):
        if self.value is None:
            return None
        return {**self.value, "assets": [{"name": name} for name in self.files]}

    def request(self, method, suffix, data):
        self.statuses.append(data["release_status"])
        self.value = {**(self.value or {}), **data}

    def upload(self, tag, file):
        if file.name == self.fail_on:
            raise RuntimeError("interrupted upload")
        self.files[file.name] = file.read_bytes()
        self.uploaded.append(file.name)

    def download_digest(self, tag, filename):
        return hashlib.sha256(self.files[filename]).hexdigest()


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cranibox packages ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.assets = fixtures(self.root / "assets")
        self.version = json.loads((ROOT / "npm/cli/package.json").read_text())["version"]
        self.tag = "v" + self.version

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

    def test_gitcode_interrupted_upload_resumes_and_promotes_only_when_complete(self):
        client = FakeGitCode()
        client.fail_on = "cranibox-linux-x86_64"
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
            publisher.publish(client, self.tag, self.assets, "Release notes")
        self.assertEqual(client.statuses, ["pre"])
        uploaded_before_retry = list(client.uploaded)
        client.fail_on = None
        publisher.publish(client, self.tag, self.assets, "Release notes")
        self.assertEqual(client.statuses, ["pre", "latest"])
        self.assertEqual(len(client.uploaded), len(verified_assets(self.assets)))
        self.assertEqual(client.uploaded[:len(uploaded_before_retry)], uploaded_before_retry)

    def test_gitcode_does_not_overwrite_conflicting_public_attachment(self):
        client = FakeGitCode()
        client.value = {"tag_name": self.tag}
        client.files["Cranibox installer.dmg"] = b"different existing release"
        with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
            publisher.publish(client, self.tag, self.assets, "Notes")
        self.assertEqual(client.uploaded, [])
        self.assertEqual(client.statuses, [])

    def test_gitcode_download_failure_never_promotes_release(self):
        client = FakeGitCode()
        with patch.object(client, "download_digest", return_value="0" * 64):
            with self.assertRaisesRegex(RuntimeError, "checksum"):
                publisher.publish(client, self.tag, self.assets, "Notes")
        self.assertEqual(client.statuses, ["pre"])

    def test_npm_retry_verifies_identical_tarball_instead_of_overwriting(self):
        output = self.root / "npm"
        builder.build(self.assets, output, self.tag)
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            name = Path(kwargs["cwd"]).name + ".tgz"
            (output / name).write_bytes(b"deterministic packed data")
            return subprocess.CompletedProcess(command, 0, json.dumps([{"filename": name}]))

        integrity = "sha512-" + base64.b64encode(hashlib.sha512(b"deterministic packed data").digest()).decode()
        with patch.object(npm_publisher.subprocess, "run", side_effect=run), patch.object(npm_publisher, "existing_integrity", return_value=integrity):
            npm_publisher.publish(output)
        self.assertTrue(all(command[1] == "pack" for command in commands))
        with patch.object(npm_publisher.subprocess, "run", side_effect=run), patch.object(npm_publisher, "existing_integrity", return_value="sha512-different"):
            with self.assertRaisesRegex(ValueError, "different contents"):
                npm_publisher.publish(output)

    @unittest.skipUnless(shutil.which("npm"), "requires npm")
    def test_real_npm_dry_run_is_offline_and_reproducible_after_artifact_download(self):
        output = self.root / "npm"
        plan = builder.build(self.assets, output, self.tag)
        env = {**os.environ, "npm_config_update_notifier": "false", "npm_config_cache": str(self.root / "cache"),
               "npm_config_userconfig": str(self.root / "empty.npmrc")}
        (self.root / "empty.npmrc").touch()
        command = [sys.executable, str(ROOT / "scripts/release/publish-npm.py"), "--directory", str(output), "--dry-run"]
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
        environment = {**os.environ, "npm_config_update_notifier": "false", "npm_config_cache": str(self.root / "cache"),
                       "npm_config_userconfig": str(self.root / "empty.npmrc")}
        (self.root / "empty.npmrc").touch()
        metadata, tarballs = {}, {}
        for entry in plan:
            package = output / entry
            # Simulate Actions losing permissions and verify the publisher restores them.
            for file in (package / "bin").iterdir():
                file.chmod(0o755)
            result = subprocess.run(["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", str(output)],
                                    cwd=package, env=environment, text=True, capture_output=True, check=True)
            packed = json.loads(result.stdout)[0]
            manifest = json.loads((package / "package.json").read_text())
            filename = packed["filename"]
            tarballs[filename] = (output / filename).read_bytes()
            metadata[manifest["name"]] = (manifest, filename, packed["integrity"])

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                key = unquote(urlsplit(self.path).path.lstrip("/"))
                if key in tarballs:
                    content = tarballs[key]
                    content_type = "application/octet-stream"
                elif key in metadata:
                    manifest, filename, integrity = metadata[key]
                    version = manifest["version"]
                    version_info = {**manifest, "dist": {"tarball": f"http://127.0.0.1:{self.server.server_port}/{filename}", "integrity": integrity}}
                    content = json.dumps({"name": key, "dist-tags": {"latest": version}, "versions": {version: version_info}}).encode()
                    content_type = "application/json"
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(content)))
                self.end_headers()
                self.wfile.write(content)

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        prefix = self.root / "installed"
        entry_tarball = output / metadata["@fliaping/cranibox"][1]
        result = subprocess.run(["npm", "install", "--prefix", str(prefix), "--ignore-scripts", "--include=optional", "--no-audit", "--no-fund",
                                 "--registry", f"http://127.0.0.1:{server.server_port}", str(entry_tarball)],
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
