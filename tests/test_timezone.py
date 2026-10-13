import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("workspace_timezone", Path(__file__).parents[1] / "scripts/workspacectl-timezone.py")
tz = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tz)

class TimezoneTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        state = self.root / ".config/agent-workspace/timezone"
        for key, value in (("ROOT", self.root), ("STATE", state), ("PREVIOUS", state.with_name("timezone.previous"))):
            p = patch.object(tz, key, value)
            p.start()
            self.addCleanup(p.stop)

    def test_invalid_zone_does_not_mutate(self):
        for zone in ("../etc/passwd", "Nowhere/Mesa", "America/Phoenix;echo bad"):
            with self.assertRaises(ValueError):
                tz.set_zone(zone, True)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_idempotence_and_rollback(self):
        original = "export OTHER=keep\nexport TZ=Asia/Taipei\nreturn\n"
        (self.root / ".bashrc").write_text(original)
        tz.set_zone("Asia/Taipei", True)
        tz.set_zone("America/Phoenix", True)
        first = (self.root / ".bashrc").read_text()
        tz.set_zone("America/Phoenix", True)
        self.assertEqual(first, (self.root / ".bashrc").read_text())
        self.assertEqual(first.count("# BEGIN Workspace timezone"), 1)
        self.assertLess(first.index("# BEGIN Workspace timezone"), first.index("return"))
        self.assertIn("export OTHER=keep", first)
        self.assertEqual(tz.read(tz.PREVIOUS), "Asia/Taipei")
        self.assertEqual((self.root / ".bashrc.before-workspace-timezone").read_text(), original)
        self.assertEqual(tz.main(["rollback", "--config-only"]), 0)
        self.assertEqual(tz.read(tz.STATE), "Asia/Taipei")

    def test_system_failure_keeps_state(self):
        tz.set_zone("Asia/Taipei", True)
        with patch.object(tz, "apply_system", side_effect=PermissionError("denied")):
            with self.assertRaises(PermissionError):
                tz.set_zone("America/Phoenix")
        self.assertEqual(tz.read(tz.STATE), "Asia/Taipei")

    def test_set_and_boot_hook(self):
        self.assertEqual(tz.main(["set", "America/Phoenix", "--config-only"]), 0)
        self.assertEqual(tz.status()["offset"], "-0700")
        self.assertEqual(tz.STATE.with_name("timezone.env").read_text(), "TZ=America/Phoenix\n")
        self.assertIn('TZ: "America/Phoenix"', tz.STATE.with_name("timezone.compose.yaml").read_text())
        hook = self.root / "custom-services.d/workspace-timezone/run"
        self.assertTrue(hook.stat().st_mode & 0o111)
        self.assertIn(" apply || exit $?", hook.read_text())

if __name__ == "__main__":
    unittest.main()
