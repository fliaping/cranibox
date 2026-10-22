"""Native CTL compatibility tests. All writes and HTTP endpoints are isolated.

Build with cargo build --locked, then run unittest discovery for this file.
Linux-specific systemctl/s6 integrations are mocked, never restarted on the host.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("WORKSPACE_CTL_BIN", ROOT / "target/debug/workspacectl"))


@unittest.skipUnless(sys.platform.startswith("linux") and BINARY.exists(), "build the Rust CTL on Linux first")
class ControlTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "mock-bin"
        self.bin.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("AGENT_WORKSPACE_", "AGENT_DESKTOP_", "PROXY_", "PROXYCTL_", "CODE_SERVER_", "CADDY_"))}
        self.env.update(HOME=str(self.root), PATH=str(self.bin),
                        AGENT_WORKSPACE_CONFIG_ROOT=str(self.root),
                        AGENT_WORKSPACE_SOURCE_DIR=str(self.root / "source"),
                        AGENT_DESKTOP_BRIDGE_URL="http://127.0.0.1:9")
        for name, content in [("sudo", "exit 1"), ("lsof", "exit 0"), ("git", "exit 1"),
                              ("s6-svstat", "echo down"), ("id", "echo testuser")]:
            self.shell(name, content)

    def write(self, path, content):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def shell(self, name, content):
        path = self.write(Path("mock-bin") / name, "#!/bin/sh\n" + content + "\n")
        path.chmod(0o755)
        return path

    def python(self, name, content):
        path = self.write(Path("mock-bin") / name, f"#!{sys.executable}\n" + content + "\n")
        path.chmod(0o755)
        return path

    def cli(self, *args, ok=True, timeout=20):
        result = subprocess.run([str(BINARY), *map(str, args)], env=self.env,
                                capture_output=True, text=True, timeout=timeout)
        if ok:
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def payload(self, *args):
        return json.loads(self.cli(*args, "--json").stdout)

    def http(self, get=None, post=None):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.respond(*(get(self.path) if get else (200, {})))

            def do_POST(self):
                data = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
                self.respond(*(post(self.path, data, dict(self.headers)) if post else (200, {})))

            def respond(self, status, data):
                data = json.dumps(data).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"http://127.0.0.1:{server.server_port}"

    def proxy(self, post=None):
        url = self.http(get=lambda path: (200, {"original": True}), post=post)
        self.write("proxyctl/env", f"PROXY_ROOT_DOMAIN=example.test\nCODE_SERVER_PROXY_DOMAIN=example.test\nCADDY_ADMIN={url}\nPROXY_LISTEN=:8080\n")
        self.write("proxyctl/routes.json", "[]\n")
        self.shell("caddy", "exit 0")
        (self.root / "proxyctl/bin").mkdir()
        (self.root / "proxyctl/bin/caddy").touch()
        return url

    def test_help_and_inspection_need_no_python(self):
        self.env["PATH"] = ""
        self.assertIn("Rust", self.cli("--help").stdout)
        self.assertIn("workspacectl 0.1.0", self.cli("--version").stdout)
        value = self.payload("status")
        self.assertEqual(value["schema_version"], 1)
        self.assertEqual([row["id"] for row in value["agents"]],
                         ["deepseek-harness", "codex", "claude-code", "pi", "hermes", "openclaw"])
        self.assertEqual([row["id"] for row in value["agents"] if row["selected"]],
                         ["deepseek-harness"])

    def test_snapshot_matches_existing_json_contract(self):
        try:
            import yaml  # Only the legacy reference needs PyYAML.
        except ImportError:
            self.skipTest("PyYAML is required to compare the legacy JSON contract")
        self.write(".agents/skills/demo/SKILL.md", "---\nname: demo\ndescription: Demo skill\n---\n")
        self.write(".codex/config.toml", '[mcp_servers.demo]\nurl="https://user:secret@example.test/mcp?token=hidden"\n')
        self.write(".config/systemd/user/demo.service", "[Unit]\nDescription=Demo\n")
        self.write(".config/code-server/config.yaml", "locale: en\n")
        self.python("chromium", "print('Chromium 140.0.7339.80')")
        native = self.payload("status")
        reference_program = "import runpy,sys,urllib.request,urllib.error\ndef unavailable(*args,**kwargs):\n    raise urllib.error.URLError('isolated reference')\nurllib.request.urlopen=unavailable\nmodule=runpy.run_path(sys.argv[1])\nsys.exit(module['main'](['status']))"
        reference = subprocess.run([sys.executable, "-c", reference_program, str(ROOT / "scripts/workspacectl-json.py")],
                                   env=self.env, capture_output=True, text=True, timeout=20)
        self.assertEqual(reference.returncode, 0, reference.stderr)
        legacy = json.loads(reference.stdout)

        def contract(expected, actual, path=""):
            if isinstance(expected, dict):
                self.assertIsInstance(actual, dict, path)
                self.assertEqual(set(expected), set(actual), path)
                for key in expected:
                    contract(expected[key], actual[key], path + "/" + key)
            elif isinstance(expected, list):
                self.assertIsInstance(actual, list, path)
                self.assertEqual(len(expected), len(actual), path)
                for index, (left, right) in enumerate(zip(expected, actual)):
                    contract(left, right, path + f"/{index}")
            elif expected is not None:
                self.assertIsInstance(actual, type(expected), path)
        contract(legacy, native)
        for section in ["skills", "mcp", "network", "locale", "bootstrap"]:
            self.assertEqual(legacy[section], native[section], section)
        self.assertTrue(native["browser"]["supported"])
        self.assertNotIn("secret", json.dumps(native["mcp"]))
        self.assertNotIn("hidden", json.dumps(native["mcp"]))

    def test_bad_agent_binary_is_isolated(self):
        broken = self.write("mock-bin/claude", "not a valid executable")
        broken.chmod(0o755)
        self.python("codex", "print('codex 1.2.3')")
        agents = {row["id"]: row for row in self.payload("agents")}
        self.assertEqual(agents["codex"]["version"], "codex 1.2.3")
        self.assertEqual(agents["claude-code"]["installed"], True)
        self.assertTrue(agents["claude-code"]["version"])

    def test_versions_merge_cache_and_invalidate_changed_binary(self):
        counter = self.root / "count"
        script = f"from pathlib import Path\np=Path({str(counter)!r})\np.write_text(p.read_text()+'x' if p.exists() else 'x')\nprint('VERSION')"
        self.python("codex", script.replace("VERSION", "codex 1.2.3"))
        browser = self.python("chromium", script.replace("VERSION", "Chromium 140.0.7339.80"))
        self.payload("status")
        first = counter.read_text()
        self.payload("status")
        self.assertEqual(counter.read_text(), first)
        self.assertEqual(len(json.loads((self.root / ".local/state/agent-workspace/command-versions-v1.json").read_text())["entries"]), 2)
        browser.write_text(browser.read_text().replace("140.0.7339.80", "141.0.7339.80"))
        value = self.payload("browser")
        self.assertEqual(value["major_version"], 141)
        self.assertEqual(len(counter.read_text()), len(first) + 1)

    def test_hanging_probe_times_out_and_kills_descendants(self):
        pidfile = self.root / "child.pid"
        self.python("codex", f"import subprocess,time\nfrom pathlib import Path\np=subprocess.Popen([{sys.executable!r},'-c','import time;time.sleep(60)'])\nPath({str(pidfile)!r}).write_text(str(p.pid))\ntime.sleep(60)")
        started = time.monotonic()
        agents = self.payload("agents")
        self.assertLess(time.monotonic() - started, 8)
        self.assertEqual(next(row for row in agents if row["id"] == "codex")["version"], "command timed out")
        pid = int(pidfile.read_text())
        stat = Path(f"/proc/{pid}/stat")
        self.assertTrue(not stat.exists() or stat.read_text().split()[2] == "Z", "probe descendant survived timeout")

    def test_device_arguments_and_exit_code_pass_through(self):
        # Device commands run in the node, over its paired peers; the gateway is gone.
        node = self.python("cranibox-node", "import json,sys\nprint(json.dumps(sys.argv[1:]))\nsys.exit(7)")
        self.env["CRANIBOX_NODE_BIN"] = str(node)
        self.assertNotEqual(self.cli("runtime", "status", ok=False).returncode, 0)
        result = self.cli("node", "exec", "one", "--", "program", "--json", ok=False)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(json.loads(result.stdout), ["exec", "one", "--", "program", "--json"])
        result = self.cli("nodes", "--json", ok=False)
        self.assertEqual(json.loads(result.stdout), ["nodes"])

    def test_skill_source_is_one_argument(self):
        args = self.root / "args.json"
        self.python("npx", f"import json,sys\nfrom pathlib import Path\nPath({str(args)!r}).write_text(json.dumps(sys.argv[1:]))")
        source = "https://example.test/repo; touch /tmp/never-created-by-test"
        self.cli("skill", "add", source)
        self.assertEqual(json.loads(args.read_text()), ["--yes", "skills", "add", source, "--global", "--agent", "*", "--yes"])

    def test_deepseek_mcp_preserves_other_servers_and_arguments(self):
        self.python("deepseek-harness", "print('ready')")
        self.shell("systemctl", "echo ActiveState=inactive")
        self.write(".local/share/agent-workspace/deepseek-mcp.json", '{"servers":{"existing":{"url":"https://example.test/mcp","transport":"streamable-http"}}}')
        native = "# Native approval and app MCP remain independent\n- insert:\n  - id: cranibox-native-approval\n    name: dsh-user-approval\n  - id: cranibox-mcp-devices\n    name: mcp-client\n"
        overlay = self.write(".dsh/agent-workspace-mcp.cordis.yml", native)
        self.cli("mcp", "add", "demo", "--transport", "stdio", "--command-line", 'tool "argument with spaces" --json', "--agents", "deepseek-harness")
        registry = json.loads((self.root / ".local/share/agent-workspace/deepseek-mcp.json").read_text())
        self.assertEqual(registry["servers"]["demo"]["args"], ["argument with spaces", "--json"])
        self.assertIn("existing", registry["servers"])
        self.assertIn(hashlib.sha256(b"demo").hexdigest()[:12], (self.root / ".dsh/agent-workspace-mcp.cordis.yml").read_text())
        self.assertTrue(overlay.read_text().startswith(native))
        self.cli("mcp", "remove", "demo", "--agents", "deepseek-harness")
        self.assertNotIn("demo", json.loads((self.root / ".local/share/agent-workspace/deepseek-mcp.json").read_text())["servers"])
        self.assertTrue(overlay.read_text().startswith(native))

    def test_pi_mcp_removal_preserves_other_server_and_user_settings(self):
        self.python("pi", "print('ready')")
        settings = self.write(".pi/agent/settings.json", '{"theme":"user-theme"}\n')
        for name in ["first", "second"]:
            self.cli("mcp", "add", name, "--transport", "stdio", "--command-line", 'tool "argument with spaces"', "--agents", "pi")
        path = self.root / ".pi/agent/mcp.json"
        self.assertEqual(json.loads(path.read_text())["mcpServers"]["first"]["args"], ["argument with spaces"])
        self.cli("mcp", "remove", "first", "--agents", "pi")
        self.assertEqual(list(json.loads(path.read_text())["mcpServers"]), ["second"])
        self.assertEqual(json.loads(settings.read_text()), {"theme": "user-theme"})
        self.cli("mcp", "remove", "second", "--agents", "pi")
        self.assertFalse(path.exists())

    def test_locale_is_idempotent_and_preserves_config(self):
        config = self.write(".config/code-server/config.yaml", "password: '$secret'\nlocale: zh-cn\n")
        self.payload("locale", "en")
        first = config.read_text()
        self.payload("locale", "en")
        self.assertEqual(config.read_text(), first)
        self.assertIn("password: '$secret'", first)
        self.assertEqual(first.count("locale:"), 1)
        self.cli("locale", "invalid", ok=False)
        self.assertEqual(config.read_text(), first)

    def test_language_pack_registration_keeps_existing_metadata(self):
        extension = self.root / "pack"
        self.write("pack/package.json", json.dumps({"version": "1.2.3", "contributes": {"localizations": [{"languageId": "zh-cn", "localizedLanguageName": "简体中文", "translations": [{"id": "vscode", "path": "main.json"}]}]}}))
        self.write("pack/main.json", "{}")
        self.write(".local/share/code-server/extensions/extensions.json", json.dumps([{"identifier": {"id": "ms-ceintl.vscode-language-pack-zh-hans", "uuid": "pack-uuid"}, "location": {"fsPath": str(extension)}}]))
        cache = self.write(".local/share/code-server/languagepacks.json", '{"other":{"preserved":true}}')
        value = self.payload("locale", "zh-cn")
        self.assertEqual(value["current"], "zh-cn")
        loaded = json.loads(cache.read_text())
        self.assertEqual(loaded["other"], {"preserved": True})
        self.assertEqual(loaded["zh-cn"]["hash"], hashlib.md5(b"pack-uuid1.2.3").hexdigest())
        self.assertEqual(loaded["zh-cn"]["translations"]["vscode"], str(extension / "main.json"))

    def test_timezone_persistence_and_rollback_without_system_changes(self):
        self.write(".config/agent-workspace/timezone", "Etc/UTC\n")
        self.write(".bashrc", "# user config\n")
        self.cli("timezone", "set", "America/Phoenix", "--config-only")
        hooks = (self.root / ".bashrc").read_text()
        self.cli("timezone", "set", "America/Phoenix", "--config-only")
        self.assertEqual((self.root / ".bashrc").read_text(), hooks)
        self.assertEqual(hooks.count("# BEGIN Workspace timezone"), 1)
        value = self.payload("timezone")
        self.assertEqual(value["configured"], "America/Phoenix")
        self.assertEqual(value["offset"], "-0700")
        self.assertEqual(value["previous"], "Etc/UTC")
        self.cli("timezone", "set", "../../etc/passwd", "--config-only", ok=False)
        self.cli("timezone", "rollback", "--config-only")
        self.assertEqual(self.payload("timezone")["configured"], "Etc/UTC")
        self.assertIn(str(BINARY.resolve()), (self.root / "custom-services.d/workspace-timezone/run").read_text())

    def test_web_configuration_validates_before_writing(self):
        self.cli("web", "configure", "--public-url", "https://workspace.example.test", "--port", "9081")
        path = self.root / ".config/agent-workspace/web.json"
        previous = path.read_bytes()
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.cli("web", "url").stdout.strip(), "https://workspace.example.test")
        for value in ["https://user:secret@example.test", "https://example.test/path", "https://example.test/?secret=1"]:
            self.cli("web", "configure", "--public-url", value, ok=False)
            self.assertEqual(path.read_bytes(), previous)

    def test_service_arguments_are_validated_and_exit_status_preserved(self):
        self.python("systemctl", "import json,sys\nprint(json.dumps(sys.argv[1:]))\nsys.exit(4)")
        result = self.cli("service", "restart", "demo", ok=False)
        self.assertEqual(result.returncode, 4)
        self.assertEqual(json.loads(result.stdout), ["--user", "restart", "demo"])
        self.cli("service", "restart", "../../etc/passwd", ok=False)
        self.cli("desktop", "scaling", "status", "extra", ok=False)

    def test_desktop_diagnostics_survive_backend_http_failure(self):
        self.write("bin/agent-desktop-mcp", "installed")
        self.env["AGENT_DESKTOP_BRIDGE_URL"] = self.http(
            get=lambda path: (503, {"state": "unavailable", "detail": "backend disconnected", "installed": False}))
        state = self.payload("desktop")["computer_use"]
        self.assertEqual(state["state"], "unavailable")
        self.assertEqual(state["detail"], "backend disconnected")
        self.assertTrue(state["installed"])
        self.assertTrue(state["running"])

    def test_desktop_actions_keep_session_and_write_screenshot_privately(self):
        calls = []
        image = b"test screenshot bytes"

        def post(path, data, headers):
            calls.append((path, data, headers))
            return 200, ({"data": base64.b64encode(image).decode()}
                         if data["action"] == "screenshot" else {"paused": data["action"] == "emergency_stop"})

        self.env["AGENT_DESKTOP_BRIDGE_URL"] = self.http(post=post)
        self.assertTrue(json.loads(self.cli("desktop", "emergency-stop").stdout)["paused"])
        self.assertFalse(json.loads(self.cli("desktop", "resume").stdout)["paused"])
        output = self.root / "capture/screen.png"
        value = json.loads(self.cli("desktop", "screenshot", "--output", str(output)).stdout)
        self.assertEqual(value["output"], str(output))
        self.assertEqual(output.read_bytes(), image)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        self.assertTrue(all(path == "/v1/action" and {key.lower(): value for key, value in headers.items()}["x-agent-session"] == "workspacectl"
                            for path, data, headers in calls))

    def test_desktop_http_error_does_not_overwrite_screenshot(self):
        output = self.write("capture/screen.png", "previous image")
        self.env["AGENT_DESKTOP_BRIDGE_URL"] = self.http(post=lambda path, data, headers: (503, {"error": "paused"}))
        self.cli("desktop", "screenshot", "--output", str(output), ok=False)
        self.assertEqual(output.read_text(), "previous image")

    def test_proxy_failure_does_not_persist_routes_and_dry_run_has_no_http_write(self):
        posts = []
        self.proxy(post=lambda path, data, headers: (posts.append(data) or 503, {"error": "rejected"}))
        path = self.root / "proxyctl/routes.json"
        before = path.read_bytes()
        self.cli("proxy", "add", "app", "127.0.0.1:9000", "--dry-run")
        self.assertEqual(posts, [])
        self.cli("proxy", "add", "app", "127.0.0.1:9000", ok=False)
        self.assertEqual(len(posts), 1)
        self.assertEqual(path.read_bytes(), before)

    def test_proxy_restores_caddy_if_local_persistence_fails(self):
        posts = []
        self.proxy(post=lambda path, data, headers: (posts.append(data) or 200, {}))
        (self.root / "proxyctl/audit.log").mkdir()
        self.cli("proxy", "add", "app", "127.0.0.1:9000", ok=False)
        self.assertEqual(json.loads((self.root / "proxyctl/routes.json").read_text()), [])
        self.assertEqual(len(posts), 2)
        self.assertIn("proxyctl:app.example.test", json.dumps(posts[0]))
        self.assertNotIn("proxyctl:app.example.test", json.dumps(posts[1]))

    def test_proxy_domain_migrates_and_https_transport_is_preserved(self):
        self.proxy()
        self.cli("proxy", "add", "app", "https://upstream.example.test", "--upstream-origin", "https://client.example.test", "--rewrite-client-ip")
        rendered = json.loads(self.cli("proxy", "render").stdout)
        block = rendered["apps"]["http"]["servers"]["proxyctl"]["routes"][0]
        handler = block["handle"][0]
        self.assertEqual(handler["transport"]["tls"]["server_name"], "upstream.example.test")
        self.assertEqual(handler["headers"]["request"]["set"]["Origin"], ["https://client.example.test"])
        self.cli("network", "domain", "new.example.test")
        self.assertEqual(json.loads((self.root / "proxyctl/routes.json").read_text())[0]["host"], "app.new.example.test")
        self.assertIn("PROXY_ROOT_DOMAIN=new.example.test", (self.root / "proxyctl/env").read_text())
        self.cli("proxy", "add", "code", "127.0.0.1:9000", ok=False)
        self.cli("proxy", "add", "1234", "127.0.0.1:9000", ok=False)

    def test_proxy_env_is_parsed_without_executing_shell(self):
        self.proxy()
        marker = self.root / "executed"
        with (self.root / "proxyctl/env").open("a") as env:
            env.write(f"IGNORED=$(touch {marker})\n")
        self.cli("proxy", "render")
        self.assertFalse(marker.exists())

    def test_prebuilt_installer_replaces_launcher_with_native_binary(self):
        path = self.root / "bin/workspacectl"
        path.parent.mkdir()
        path.symlink_to(ROOT / "scripts/workspacectl")
        # A Workspace image ships a built-in cranibox; keep this test about the plain installer.
        env = {**self.env, "PATH": "/usr/bin:/bin", "AGENT_WORKSPACE_CTL_BIN": str(BINARY), "AGENT_WORKSPACE_BUILTIN_DIR": str(self.root / "no-builtin")}
        result = subprocess.run(["/bin/bash", str(ROOT / "scripts/install-workspacectl.sh")], env=env, capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(path.is_symlink())
        self.assertEqual(path.read_bytes()[:4], b"\x7fELF")
        self.assertEqual((self.root / ".local/bin/workspacectl").resolve(), path)
        result = subprocess.run([str(path), "--version"], env={**self.env, "PATH": ""}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
