"""Desktop on/off switch: s6 gate, status, and the Selkies page / nginx patch."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/cranibox-desktop"
INSTALL = ROOT / "addons/desktop-power/install.sh"


def load_script():
    import importlib.machinery
    import importlib.util
    loader = importlib.machinery.SourceFileLoader("cranibox_desktop", str(SCRIPT))
    spec = importlib.util.spec_from_loader("cranibox_desktop", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module
NGINX = """server {
  listen 3000 default_server;
  location SUBFOLDER {
    alias /usr/share/selkies/web/;
  }
  error_page 500 502 503 504 /50x.html;
}

server {
  listen 3001 ssl;
  location SUBFOLDER {
    alias /usr/share/selkies/web/;
  }
  error_page 500 502 503 504 /50x.html;
}
"""
INDEX = ('<!doctype html><html lang="en"><head><title>Selkies</title>'
         '<script type="module" crossorigin src="./assets/index-BpOBp7NB.js"></script>'
         '<link rel="stylesheet" crossorigin href="./assets/index.css"></head><body><div id="app"></div></body></html>')


class DesktopPowerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {**os.environ, "CRANIBOX_DESKTOP_STATE_DIR": str(self.root / "state"),
                    "CRANIBOX_DESKTOP_SERVICE_ROOT": str(self.root / "service")}

    def run_script(self, *args, env=None):
        return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True,
                              env=env or self.env, timeout=20)

    def test_gate_runs_service_when_on(self):
        marker = self.root / "ran"
        result = self.run_script("gate", "svc-de", "sh", "-c", f"touch {marker}")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(marker.exists())
        self.assertEqual((self.root / "state/state").read_text(), "on")

    def test_boot_state_off_idles_service(self):
        env = {**self.env, "CRANIBOX_DESKTOP": "off"}
        process = subprocess.Popen([sys.executable, str(SCRIPT), "gate", "svc-de", "sh", "-c", "exit 7"],
                                   env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            with self.assertRaises(subprocess.TimeoutExpired):
                process.wait(timeout=1.5)  # replaced by `sleep infinity`
            self.assertEqual(Path(f"/proc/{process.pid}/comm").read_text().strip(), "sleep")
        finally:
            process.kill()
            process.wait()
        self.assertEqual((self.root / "state/state").read_text(), "off")

    def test_runtime_state_wins_over_boot_variable(self):
        (self.root / "state").mkdir()
        (self.root / "state/state").write_text("on")
        env = {**self.env, "CRANIBOX_DESKTOP": "off"}
        result = self.run_script("status", "--json", env=env)
        value = json.loads(result.stdout)
        self.assertEqual(value["state"], "on")
        self.assertEqual(value["boot_state"], "off")

    def test_status_json_shape(self):
        (self.root / "state").mkdir()
        (self.root / "state/state").write_text("off")
        value = json.loads(self.run_script("status", "--json").stdout)
        self.assertEqual(value["phase"], "stopped")
        for key in ("desktop_memory_mb", "memory_hint_mb", "container_memory_mb", "process_count"):
            self.assertIn(key, value)

    def test_desktop_programs_of_every_variant(self):
        module = load_script()
        desktop = ["xfdesktop", "xfce4-panel", "labwc", "lxqt-panel", "lxqt-session", "pcmanfm-qt", "openbox",
                   "plasmashell", "plasma_waitforn", "kwin_wayland", "kded6", "kactivitymanage", "ksecretd",
                   "baloo_file", "pipewire", "wireplumber", "xembedsniproxy", "Xwayland", "pulseaudio"]
        for comm in desktop:
            self.assertTrue(module.is_desktop_program(comm, lambda: ""), comm)
        for comm in ["cranibox", "node", "bash", "codex", "python3", "code-server"]:
            self.assertFalse(module.is_desktop_program(comm, lambda: ""), comm)
        self.assertTrue(module.is_desktop_program("chromium", lambda: "chromium --user-data-dir=/x"))
        self.assertFalse(module.is_desktop_program("chromium", lambda: "chromium --headless=new"))
        self.assertTrue(module.is_desktop_program("dbus-daemon", lambda: "dbus-daemon --session --nofork"))
        self.assertFalse(module.is_desktop_program("dbus-daemon", lambda: "dbus-daemon --system"))
        self.assertIn("geoclue", module.SYSTEM_HELPERS)

    def test_page_and_nginx_patch(self):
        dashboard = self.root / "selkies/selkies-dashboard"
        dashboard.mkdir(parents=True)
        (dashboard / "index.html").write_text(INDEX)
        conf = self.root / "default.conf"
        conf.write_text(NGINX)
        env = {**os.environ, "SELKIES_ROOT": str(self.root / "selkies"), "NGINX_DEFAULT_CONF": str(conf)}
        for _ in range(2):  # idempotent
            result = subprocess.run(["bash", str(INSTALL)], capture_output=True, text=True, env=env, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
        html = (dashboard / "index.html").read_text()
        self.assertIn('<script src="cranibox-desktop.js" data-entry="./assets/index-BpOBp7NB.js"></script>', html)
        self.assertNotIn('type="module"', html)  # the Selkies client only starts while the desktop is on
        self.assertTrue((dashboard / "cranibox-desktop.js").exists())
        text = conf.read_text()
        self.assertEqual(text.count("location ^~ SUBFOLDERcranibox-desktop/"), 2)
        self.assertEqual(text.count("proxy_pass              http://127.0.0.1:3079/;"), 2)


if __name__ == "__main__":
    unittest.main()
