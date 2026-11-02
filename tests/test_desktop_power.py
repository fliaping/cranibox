"""Desktop on/off switch: s6 gate, status, and the Selkies page / nginx patch."""
import json
import os
from pathlib import Path
import subprocess
import sys
import socket
import tempfile
import time
import unittest
from unittest import mock
import urllib.error
import urllib.request

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
        env = {**os.environ, "SELKIES_ROOT": str(self.root / "selkies"), "NGINX_DEFAULT_CONF": str(conf),
               "XFCONF_SYSTEM_DIR": str(self.root / "no-xfce")}
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

    def test_xfce_display_defaults_keep_the_stream_output_on(self):
        dashboard = self.root / "selkies/selkies-dashboard"
        dashboard.mkdir(parents=True)
        (dashboard / "index.html").write_text(INDEX)
        conf = self.root / "default.conf"
        conf.write_text(NGINX)
        xfconf = self.root / "xfconf"
        xfconf.mkdir()
        env = {**os.environ, "SELKIES_ROOT": str(self.root / "selkies"), "NGINX_DEFAULT_CONF": str(conf),
               "XFCONF_SYSTEM_DIR": str(xfconf)}
        result = subprocess.run(["bash", str(INSTALL)], capture_output=True, text=True, env=env, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        displays = (xfconf / "displays.xml").read_text()
        self.assertIn('<property name="Notify" type="int" value="0"/>', displays)
        self.assertIn('<property name="AutoEnableProfiles" type="bool" value="false"/>', displays)

    def test_stale_shared_memory(self):
        module = load_script()
        table = (" key shmid perms size cpid lpid nattch uid gid cuid cgid atime dtime ctime rss swap\n"
                 "0 11 1600 4096 100 100 0 1000 1000 1000 1000 0 0 0 0 0\n"   # creator gone: stale
                 "0 12 1600 4096 200 200 0 1000 1000 1000 1000 0 0 0 0 0\n"   # creator alive
                 "0 13 1600 4096 100 100 1 1000 1000 1000 1000 0 0 0 0 0\n"   # still attached
                 "0 14 1600 4096 100 100 0 0 0 0 0 0 0 0 0 0\n")               # another user
        self.assertEqual(module.stale_shared_memory(table, 1000, lambda pid: pid == 200), [11])

    def test_stale_shared_memory_is_removed_as_its_owner(self):
        # Root in a container lacks CAP_IPC_OWNER, so ipcrm has to run as abc.
        module = load_script()
        table = (" key shmid perms size cpid lpid nattch uid gid cuid cgid atime dtime ctime rss swap\n"
                 "0 11 777 4096 999999999 1 0 1000 1000 1000 1000 0 0 0 0 0\n"
                 "0 12 777 4096 999999999 1 0 1000 1000 1000 1000 0 0 0 0 0\n")
        calls = []
        module.abc_user_id = lambda: 1000
        with mock.patch.object(module.pathlib.Path, "read_text", lambda self: table), \
                mock.patch.object(module.subprocess, "run",
                                  lambda command, **kwargs: calls.append((command, kwargs.get("user")))):
            module.free_shared_memory()
        self.assertEqual(calls, [(["ipcrm", "-m", "11", "-m", "12"], 1000)])

    def test_selkies_port_answers_while_off(self):
        api_port, selkies_port = free_port(), free_port()
        state = self.root / "state"
        state.mkdir()
        (state / "state").write_text("off")
        js = self.root / "cranibox-desktop.js"
        js.write_text("/* page */")
        env = {**self.env, "CRANIBOX_DESKTOP_PORT": str(api_port), "CUSTOM_WS_PORT": str(selkies_port),
               "CRANIBOX_DESKTOP_PAGE_SCRIPT": str(js)}
        server = subprocess.Popen([sys.executable, str(SCRIPT), "serve"], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(server.wait, 10)
        self.addCleanup(server.terminate)
        base = f"http://127.0.0.1:{selkies_port}"
        page = wait_for(lambda: urllib.request.urlopen(base + "/", timeout=1).read().decode())
        self.assertIn("桌面已关闭", page)
        self.assertIn('src="cranibox-desktop.js"', page)
        self.assertEqual(urllib.request.urlopen(base + "/cranibox-desktop.js", timeout=2).read(), b"/* page */")
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(base + "/api/websockets", timeout=2)
        self.assertEqual(caught.exception.code, 503)
        self.assertEqual(caught.exception.headers["X-Cranibox-Desktop"], "off")
        self.assertEqual(json.loads(caught.exception.read())["error"], "desktop_off")
        # The page's API works on this port too, and the placeholder never counts as a running stream.
        value = json.loads(urllib.request.urlopen(base + "/cranibox-desktop/status", timeout=2).read())
        self.assertEqual((value["state"], value["phase"], value["stream_ready"]), ("off", "stopped", False))
        value = json.loads(urllib.request.urlopen(f"http://127.0.0.1:{api_port}/status", timeout=2).read())
        self.assertFalse(value["stream_ready"])
        request = urllib.request.Request(base + "/cranibox-desktop/on", method="POST")
        with self.assertRaises(urllib.error.HTTPError) as caught:
            urllib.request.urlopen(request, timeout=2)
        self.assertEqual(caught.exception.code, 403)  # the switch header is still required


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def wait_for(read, timeout=10.0):
    deadline = time.time() + timeout
    while True:
        try:
            return read()
        except OSError:
            if time.time() > deadline:
                raise
            time.sleep(0.1)


if __name__ == "__main__":
    unittest.main()
