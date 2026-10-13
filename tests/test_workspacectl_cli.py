import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKSPACECTL = ROOT / "scripts/workspacectl"


class TailscaleServeTargetTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        stub = bin_dir / "tailscale"
        stub.write_text('#!/bin/sh\necho "$@"\n')
        stub.chmod(0o755)
        sudo = bin_dir / "sudo"
        sudo.write_text('#!/bin/sh\n[ "$1" != "-n" ] || shift\nexec "$@"\n')
        sudo.chmod(0o755)
        self.config = self.root / "config/.config/code-server/config.yaml"
        self.config.parent.mkdir(parents=True)
        self.env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}",
                    "AGENT_WORKSPACE_CONFIG_ROOT": str(self.root / "config")}

    def serve(self) -> str:
        result = subprocess.run(["bash", str(WORKSPACECTL), "tailscale", "serve"], env=self.env,
                                capture_output=True, text=True, check=True)
        return result.stdout.strip()

    def test_https_code_server_uses_insecure_https_upstream(self):
        self.config.write_text("bind-addr: 0.0.0.0:8443\nauth: password\ncert: true\n")
        self.assertEqual(self.serve(), "serve --bg https+insecure://127.0.0.1:8443")

    def test_plain_http_code_server_keeps_http_upstream(self):
        self.config.write_text("bind-addr: 127.0.0.1:9443\ncert: false\n")
        self.assertEqual(self.serve(), "serve --bg http://127.0.0.1:9443")

    def test_missing_config_defaults_to_code_server_default(self):
        self.assertEqual(self.serve(), "serve --bg http://127.0.0.1:8443")


if __name__ == "__main__":
    unittest.main()
