"""Isolated OpenClaw service handoff tests; never touches a live Gateway."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/workspacectl-openclaw.py"
SPEC = importlib.util.spec_from_file_location("workspace_openclaw_update", SCRIPT)
updater = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(updater)


class OpenClawUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.unit = root / ".config/systemd/user/openclaw-gateway.service"
        self.unit.parent.mkdir(parents=True)
        self.old_entry = root / "node/bin/openclaw"
        self.old_entry.parent.mkdir(parents=True)
        self.old_entry.touch()
        self.original = ("[Service]\nExecStart=" + str(self.old_entry) +
                         " gateway run --port 18789\nEnvironment=API_KEY=secret\n")
        self.unit.write_text(self.original)
        self.release = root / "opt/openclaw/releases/2026.9.5/node_modules/openclaw"
        self.release.mkdir(parents=True)
        (self.release / "openclaw.mjs").touch()
        (self.release / "package.json").write_text('{"version":"2026.9.5"}')
        for key, value in {"ROOT": root, "RELEASES": root / "opt/openclaw/releases",
                           "STATE": root / ".local/state/agent-workspace", "UNIT": self.unit,
                           "ALIAS": root / ".local/bin/openclaw"}.items():
            original = getattr(updater, key)
            setattr(updater, key, value)
            self.addCleanup(setattr, updater, key, original)

    def test_success_preserves_environment_and_gateway_args(self):
        with patch.object(updater, "run", return_value='"2026.9.5"'), \
             patch.object(updater.shutil, "which", side_effect=lambda name: "/usr/bin/" + name), \
             patch.object(updater, "restart") as restart, patch.object(updater, "verify") as verify:
            updater.update()
        self.assertEqual(restart.call_count, 1)
        verify.assert_called_once_with(self.release / "openclaw.mjs")
        changed = self.unit.read_text()
        self.assertIn("Environment=API_KEY=secret", changed)
        self.assertIn("Environment=OPENCLAW_SUPERVISOR_MODE=external", changed)
        self.assertIn("gateway run --port 18789", changed)
        self.assertTrue(updater.ALIAS.is_symlink())

    def test_failed_verification_restores_exact_original_unit(self):
        with patch.object(updater, "run", return_value='"2026.9.5"'), \
             patch.object(updater.shutil, "which", side_effect=lambda name: "/usr/bin/" + name), \
             patch.object(updater, "restart") as restart, \
             patch.object(updater, "verify", side_effect=[RuntimeError("not ready"), None]):
            with self.assertRaisesRegex(RuntimeError, "not ready"):
                updater.update()
        self.assertEqual(self.unit.read_text(), self.original)
        self.assertEqual(restart.call_count, 2)
        self.assertFalse(updater.ALIAS.exists())

    def test_rejects_complex_command_instead_of_rewriting_it(self):
        with self.assertRaisesRegex(RuntimeError, "Unsupported Gateway ExecStart"):
            updater.managed_unit("[Service]\nExecStart=/bin/sh -c openclaw gateway run\n", Path("/new/openclaw.mjs"), "/usr/bin/node")

    def test_existing_node_launcher_is_replaced_without_duplicate_node(self):
        unit = "[Service]\nExecStart=/config/node/bin/node /config/old/openclaw.mjs gateway --port 18789\n"
        changed = updater.managed_unit(unit, Path("/config/new/openclaw.mjs"), "/config/node/bin/node")
        self.assertIn("ExecStart=/config/node/bin/node /config/new/openclaw.mjs gateway --port 18789", changed)
        self.assertNotIn("node /config/node/bin/node", changed)


if __name__ == "__main__":
    unittest.main()
