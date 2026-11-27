"""scripts/install.sh: platform detection, release URLs, SHA256SUMS verification, install dir."""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "install.sh"
SHELLS = [s for s in ("sh", "dash", "bash") if shutil.which(s)]


def call(shell, function, *args, env=None):
    """Run one function of the script (sourced with CRANIBOX_INSTALL_LIB=1)."""
    quoted = " ".join("'" + a.replace("'", "'\\''") + "'" for a in args)
    environment = {"PATH": os.environ["PATH"], "HOME": "/home/tester", **(env or {}), "CRANIBOX_INSTALL_LIB": "1"}
    return subprocess.run([shell, "-c", f'. "$0"; {function} {quoted}', str(SCRIPT)],
                          capture_output=True, text=True, env=environment)


class InstallScriptFunctions(unittest.TestCase):
    def test_asset_names_cover_every_released_platform(self):
        cases = {
            ("Linux", "x86_64"): "cranibox-linux-x86_64",
            ("Linux", "amd64"): "cranibox-linux-x86_64",
            ("Linux", "aarch64"): "cranibox-linux-arm64",
            ("Darwin", "arm64"): "cranibox-macos-arm64",
            ("Darwin", "x86_64"): "cranibox-macos-x86_64",
        }
        for shell in SHELLS:
            for (os_name, arch), asset in cases.items():
                result = call(shell, "asset_name", os_name, arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), asset)
            for os_name, arch in (("Windows_NT", "x86_64"), ("FreeBSD", "amd64"), ("Linux", "armv7l"), ("Linux", "riscv64")):
                result = call(shell, "asset_name", os_name, arch)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("unsupported", result.stderr)

    def test_asset_names_match_the_release_platform_table(self):
        import sys
        sys.path.insert(0, str(SCRIPT.parents[0] / "release"))
        from release_common import PLATFORMS
        unix = {name for name, system, _ in PLATFORMS.values() if system in ("linux", "darwin")}
        produced = {call("sh", "asset_name", o, a).stdout.strip()
                    for o, a in (("Linux", "x86_64"), ("Linux", "aarch64"), ("Darwin", "arm64"), ("Darwin", "x86_64"))}
        self.assertEqual(produced, unix)

    def test_download_base_follows_version_and_mirror(self):
        self.assertEqual(call("sh", "download_base").stdout.strip(),
                         "https://github.com/fliaping/cranibox/releases/latest/download")
        for version in ("1.2.1", "v1.2.1"):
            self.assertEqual(call("sh", "download_base", env={"CRANIBOX_VERSION": version}).stdout.strip(),
                             "https://github.com/fliaping/cranibox/releases/download/v1.2.1")
        self.assertEqual(call("sh", "download_base", env={"CRANIBOX_DOWNLOAD_BASE": "https://mirror.example/c/"}).stdout.strip(),
                         "https://mirror.example/c")

    def test_checksum_lookup_accepts_both_sha256sum_forms(self):
        with tempfile.TemporaryDirectory() as tmp:
            sums = Path(tmp) / "SHA256SUMS"
            sums.write_text("A" * 64 + "  cranibox-linux-x86_64\n" + "b" * 64 + " *cranibox-macos-arm64\n"
                            + "c" * 64 + "  cranibox-linux-x86_64.sig\n")
            for shell in SHELLS:
                self.assertEqual(call(shell, "expected_sum", str(sums), "cranibox-linux-x86_64").stdout.strip(), "a" * 64)
                self.assertEqual(call(shell, "expected_sum", str(sums), "cranibox-macos-arm64").stdout.strip(), "b" * 64)
                self.assertNotEqual(call(shell, "expected_sum", str(sums), "cranibox-linux-arm64").returncode, 0)

    def test_install_dir_defaults(self):
        self.assertEqual(call("sh", "default_install_dir", env={"CRANIBOX_INSTALL_DIR": "/opt/x"}).stdout.strip(), "/opt/x")
        expected = "/usr/local/bin" if os.getuid() == 0 else "/home/tester/.local/bin"
        self.assertEqual(call("sh", "default_install_dir").stdout.strip(), expected)


class InstallScriptEndToEnd(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.release = self.tmp / "release"
        self.release.mkdir()
        program = "#!/bin/sh\necho 'cranibox 9.9.9-test'\n"
        for name in ("cranibox-linux-x86_64", "cranibox-linux-arm64"):
            (self.release / name).write_text(program.replace("test", name))
        self.write_sums()
        self.bin = self.tmp / "bin"

    def write_sums(self):
        lines = [f"{hashlib.sha256(f.read_bytes()).hexdigest()}  {f.name}\n"
                 for f in sorted(self.release.iterdir()) if f.name != "SHA256SUMS"]
        (self.release / "SHA256SUMS").write_text("".join(lines))

    def install(self, shell="sh", arch="x86_64", **extra):
        env = {"PATH": os.environ["PATH"], "HOME": str(self.tmp / "home"),
               "CRANIBOX_DOWNLOAD_BASE": self.release.as_uri(), "CRANIBOX_INSTALL_DIR": str(self.bin),
               "CRANIBOX_INSTALL_OS": "Linux", "CRANIBOX_INSTALL_ARCH": arch, **extra}
        return subprocess.run([shell, str(SCRIPT)], capture_output=True, text=True, env=env)

    @unittest.skipUnless(shutil.which("curl"), "needs curl")
    def test_installs_the_verified_binary_and_hints_at_path(self):
        for shell in SHELLS:
            for arch, name in (("x86_64", "cranibox-linux-x86_64"), ("aarch64", "cranibox-linux-arm64")):
                result = self.install(shell, arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                installed = self.bin / "cranibox"
                self.assertTrue(os.access(installed, os.X_OK))
                self.assertEqual(installed.read_bytes(), (self.release / name).read_bytes())
                self.assertIn(f"Installed cranibox 9.9.9-{name}", result.stderr)
                self.assertIn("is not on your PATH", result.stderr)
                self.assertIn("cranibox serve --daemon", result.stderr)
                self.assertFalse((self.bin / ".cranibox.new").exists())
        on_path = self.install(PATH=f"{self.bin}:{os.environ['PATH']}")
        self.assertEqual(on_path.returncode, 0, on_path.stderr)
        self.assertNotIn("is not on your PATH", on_path.stderr)

    @unittest.skipUnless(shutil.which("curl"), "needs curl")
    def test_refuses_a_tampered_or_unlisted_binary(self):
        (self.release / "cranibox-linux-x86_64").write_text("#!/bin/sh\necho evil\n")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)
        self.assertFalse((self.bin / "cranibox").exists())
        self.write_sums()
        (self.release / "SHA256SUMS").write_text(
            (self.release / "SHA256SUMS").read_text().replace("cranibox-linux-x86_64\n", "other\n"))
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not listed in SHA256SUMS", result.stderr)
        self.assertFalse((self.bin / "cranibox").exists())

    @unittest.skipUnless(shutil.which("curl"), "needs curl")
    def test_missing_platform_build_is_explained(self):
        result = self.install(CRANIBOX_INSTALL_OS="Darwin", CRANIBOX_INSTALL_ARCH="x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cranibox-macos-x86_64", result.stderr)
        self.assertFalse((self.bin / "cranibox").exists())


if __name__ == "__main__":
    unittest.main()
