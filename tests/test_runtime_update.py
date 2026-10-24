"""Real-binary check of self-update: adopt, launch, update from an npm registry, roll back.

Runs the actual `cranibox` program with a fake local npm registry. Set CRANIBOX_BIN to the
program to test (a release build is much faster to copy than a debug build).
"""
import hashlib, base64, http.server, io, json, os, platform, shutil, signal, subprocess, sys
import tarfile, tempfile, threading, time, unittest
from pathlib import Path

BIN = os.environ.get("CRANIBOX_BIN") or str(
    Path(__file__).resolve().parents[1] / "target" / "debug" / ("cranibox.exe" if os.name == "nt" else "cranibox")
)
WIN = os.name == "nt"


def platform_suffix():
    machine = platform.machine().lower()
    if sys.platform == "darwin" and machine in ("arm64", "aarch64"):
        return "darwin-arm64"
    if sys.platform.startswith("linux") and machine in ("x86_64", "amd64"):
        return "linux-x64-gnu"
    if WIN and machine in ("amd64", "x86_64"):
        return "win32-x64"
    return None


def tarball(exe_bytes, version):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as t:
        for name, data, mode in (
            ("package/package.json", json.dumps({"name": "x", "version": version}).encode(), 0o644),
            ("package/bin/cranibox" + (".exe" if WIN else ""), exe_bytes, 0o755),
        ):
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(data), mode
            t.addfile(info, io.BytesIO(data))
    return out.getvalue()


class Registry(http.server.ThreadingHTTPServer):
    def __init__(self, package):
        self.package, self.versions, self.tags = package, {}, {}
        super().__init__(("127.0.0.1", 0), self.Handler)
        threading.Thread(target=self.serve_forever, daemon=True).start()

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_address[1]}"

    def publish(self, version, exe_bytes, tag=None):
        self.versions[version] = tarball(exe_bytes, version)
        if tag:
            self.tags[tag] = version

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            reg = self.server
            if self.path.startswith("/tarballs/"):
                data = reg.versions.get(self.path.rsplit("/", 1)[1].removesuffix(".tgz"))
                if data is None:
                    return self.send_error(404)
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                return self.wfile.write(data)
            if self.path.strip("/").replace("%2F", "/").replace("%2f", "/") == reg.package:
                versions = {
                    v: {"version": v, "dist": {
                        "tarball": f"{reg.url}/tarballs/{v}.tgz",
                        "integrity": "sha512-" + base64.b64encode(hashlib.sha512(d).digest()).decode(),
                    }} for v, d in reg.versions.items()
                }
                body = json.dumps({"name": reg.package, "dist-tags": reg.tags, "versions": versions}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                return self.wfile.write(body)
            self.send_error(404)


@unittest.skipUnless(platform_suffix(), "no npm platform package for this machine")
@unittest.skipUnless(Path(BIN).exists(), "build the cranibox program or set CRANIBOX_BIN")
class RuntimeUpdate(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="cranibox-update-test-"))
        self.cfg = self.tmp / "cfg" / "settings.json"
        self.cfg.parent.mkdir()
        # This test only uses the loopback npm registry and local runtime supervision.
        (self.cfg.parent / "networks.json").write_text(json.dumps({"iroh": {"enabled": False}}))
        self.exe = self.tmp / ("cranibox.exe" if WIN else "cranibox")
        shutil.copy(BIN, self.exe)
        if not WIN and self.exe.stat().st_size > 100_000_000 and shutil.which("strip"):
            subprocess.run(["strip", str(self.exe)], check=False)  # debug builds are huge
            if sys.platform == "darwin":  # stripping invalidates the signature arm64 requires
                subprocess.run(["codesign", "-s", "-", "-f", str(self.exe)], check=True)
        self.exe_bytes = self.exe.read_bytes()
        self.base = subprocess.run([str(self.exe), "--version"], capture_output=True, text=True).stdout.split()[1]
        self.registry = Registry(f"@fliaping/cranibox-{platform_suffix()}")
        self.env = {**os.environ, "CRANIBOX_NPM_REGISTRY": self.registry.url}
        self.launcher = None
        self.log = self.tmp / "launcher.log"

    def tearDown(self):
        if self.launcher and self.launcher.poll() is None:
            self.launcher.send_signal(signal.SIGTERM if not WIN else signal.CTRL_BREAK_EVENT)
            try:
                self.launcher.wait(15)
            except subprocess.TimeoutExpired:
                self.launcher.kill()
        self.registry.shutdown()
        self.registry.server_close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def cranibox(self, *args, check=True):
        r = subprocess.run([str(self.exe), "--config", str(self.cfg), *args], capture_output=True,
                           text=True, env=self.env, timeout=120)
        if check:
            self.assertEqual(r.returncode, 0, r.stderr)
        return r


    def app_api(self, path, method="GET", body=None):
        """Call the node's local app API the way the web UI does: sign-in link, cookie, CSRF."""
        import http.cookiejar, re, urllib.request
        link = self.cranibox("ui-url").stdout.strip()
        jar = http.cookiejar.CookieJar()
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
        html = opener.open(link, timeout=20).read().decode()
        csrf = json.loads(html.split("window.workspaceWeb=")[1].split(";</script>")[0])["csrf"]
        base = link.split("/?")[0].split("/#")[0].rstrip("/")
        request = urllib.request.Request(base + path, method=method, headers={
            "X-Workspace-CSRF": csrf, "Content-Type": "application/json"},
            data=None if body is None else json.dumps(body).encode())
        with opener.open(request, timeout=30) as response:
            return json.loads(response.read().decode())

    def status(self):
        return json.loads(self.cranibox("update", "status").stdout)["devices"][0]

    def wait_for(self, predicate, seconds, what):
        end = time.time() + seconds
        while time.time() < end:
            try:
                s = self.status()
                if predicate(s):
                    return s
            except Exception:
                pass
            time.sleep(1)
        pointer = self.tmp / "cfg" / "runtime" / "current.json"
        self.fail(f"timed out waiting for {what}; last status: {self.status()}\npointer: "
                  f"{pointer.read_text() if pointer.exists() else None}\nlauncher log:\n"
                  f"{self.log.read_text(errors='replace')[-3000:] if self.log.exists() else ''}")

    def test_update_and_rollback(self):
        self.cranibox("update", "adopt")
        s = self.status()
        self.assertEqual((s["runtime"]["version"], s["runtime"]["managed"]), (self.base, True))

        self.registry.publish("1.1.1-dev.1", self.exe_bytes, tag="dev")
        with self.log.open("wb") as log:
            self.launcher = subprocess.Popen(
                [str(self.exe), "--config", str(self.cfg), "update", "launch"],
                env=self.env, stdout=log, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if WIN else 0)
        # adopt already marks the version confirmed, so wait for a node that is actually serving.
        self.wait_for(lambda s: "device interface" in self.log.read_text(errors="replace"), 90, "the node to start")

        # The web UI's update panel talks to the running node's app API: check the real shapes.
        info = self.app_api("/api/v1/app/update?node=self&check=1")
        self.assertEqual((info["self"], info["version"], info["runtime"]["managed"]), (True, self.base, True))
        self.assertTrue(info["runtime"]["can_update"], info["runtime"]["reasons"])
        self.assertEqual(info["available"]["dev"], "1.1.1-dev.1")
        self.assertIsNone(info.get("available_error"))

        # Update through the dist-tag; the running node restarts into the new version.
        self.cranibox("update", "apply", "dev")
        s = self.wait_for(lambda s: s["status"] and s["status"]["phase"] == "done", 120, "the update to finish")
        self.assertEqual((s["runtime"]["version"], s["runtime"]["state"]), ("1.1.1-dev.1", "confirmed"))

        # A version that cannot start is rolled back by the launcher without any network.
        bad = b"#!/bin/sh\nexit 1\n" if not WIN else b"MZ not a program"
        self.registry.publish("1.1.2-dev.1", bad, tag="dev")
        self.cranibox("update", "apply", "1.1.2-dev.1")
        s = self.wait_for(lambda s: s["status"] and s["status"]["phase"] == "rolled_back", 180, "the rollback")
        self.assertEqual((s["runtime"]["version"], s["runtime"]["state"]), ("1.1.1-dev.1", "confirmed"))

        # The failed version is not retried unless forced.
        again = self.cranibox("update", "apply", "1.1.2-dev.1", check=False)
        self.assertNotEqual(again.returncode, 0)
        self.assertIn("failed", again.stderr)

        available = json.loads(self.cranibox("update", "check").stdout)["devices"][0]
        self.assertEqual(available["available"]["dev"], "1.1.2-dev.1")
        self.assertIsNone(available["available"]["latest"])
        self.assertIsNone(available["available"]["next"])
        self.assertIsNone(available["available_error"])


if __name__ == "__main__":
    unittest.main()
