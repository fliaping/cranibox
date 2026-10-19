"""Real gateway/node/MCP tests; no Python WebSocket dependency required.

Run cargo build --locked, then python3 -m unittest discover -s tests -p test_rust_runtime.py.
Windows/macOS CI runs the same binaries natively.
"""
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("WORKSPACE_RUNTIME_BIN", ROOT / "target/debug" / ("workspace-runtime.exe" if os.name == "nt" else "workspace-runtime")))


NODE_BINARY = Path(os.environ.get("CRANIBOX_NODE_BIN", ROOT / "target/debug" / ("cranibox-node.exe" if os.name == "nt" else "cranibox-node")))

def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@unittest.skipUnless(BINARY.exists(), "build the Rust runtime before running its integration suite")
class RuntimeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.temp.name)
        cls.credentials = cls.path / "runtime.json"
        cls.logs = []
        cls.processes = []
        cls.addClassCleanup(cls.cleanup)
        class Legacy(BaseHTTPRequestHandler):
            def do_GET(self):
                payload = json.dumps({"path":self.path,"host":self.headers.get("Host"),
                                      "origin":self.headers.get("Origin"),"cookie":self.headers.get("Cookie"),
                                      "authorization":self.headers.get("Authorization"),
                                      "forwarded":self.headers.get("X-Forwarded-Host")}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Set-Cookie", "aw_session=example; HttpOnly; SameSite=Strict")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                self.do_GET()

            def log_message(self, *args):
                pass

        cls.legacy = ThreadingHTTPServer(("127.0.0.1", 0), Legacy)
        threading.Thread(target=cls.legacy.serve_forever, daemon=True).start()
        cls.port = free_port()
        cls.url = f"http://127.0.0.1:{cls.port}"
        cls.ws = f"ws://127.0.0.1:{cls.port}"
        cls.cli("init", "--credentials", cls.credentials)
        cls.token = json.loads(cls.credentials.read_text())["controller_token"]
        for node in ["one", "two"]:
            cls.cli("enroll", node, "--credentials", cls.credentials, "--token-file", cls.path / f"{node}.token")
        cls.server = cls.start("serve", "--credentials", cls.credentials, "--listen", f"127.0.0.1:{cls.port}",
                               "--web-upstream", f"http://127.0.0.1:{cls.legacy.server_port}")
        cls.wait_for(lambda: cls.http("/healthz", auth=False)[0] == 200)
        cls.node_processes = {node: cls.start("node", "--server", cls.ws, "--id", node, "--token-file", cls.path / f"{node}.token") for node in ["one", "two"]}
        try:
            cls.wait_for(lambda: all(n["online"] for n in cls.http("/api/v1/nodes")[1]["nodes"]))
        except Exception:
            # Cleanup removes these files, so retain startup failures in CI output.
            for index, process in enumerate(cls.processes):
                log = cls.path / f"process-{index}.log"
                print(f"{log.name} (exit={process.poll()}):\n"
                      f"{log.read_text(encoding='utf-8', errors='replace')[-12000:]}",
                      file=sys.stderr)
            raise

    @classmethod
    def cleanup(cls):
        for process in reversed(cls.processes):
            cls.stop(process)
        for log in cls.logs:
            log.close()
        if hasattr(cls, "legacy"):
            cls.legacy.shutdown()
            cls.legacy.server_close()
        cls.temp.cleanup()

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    @classmethod
    def start(cls, *args):
        log = open(cls.path / f"process-{len(cls.logs)}.log", "wb")
        cls.logs.append(log)
        binary = NODE_BINARY if args[0] == "node" else BINARY
        args = args[1:] if args[0] == "node" else args
        if binary == NODE_BINARY:
            # Each simulated device has its own persistent identity and Shell owner.
            node_id = args[args.index("--id") + 1]
            args = ("--config", cls.path / node_id / "settings.json", *args)
        process = subprocess.Popen([str(binary), *map(str, args)], stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        cls.processes.append(process)
        return process

    @classmethod
    def cli(cls, *args, check=True, **kwargs):
        return subprocess.run([str(BINARY), *map(str, args)], capture_output=True, text=True, check=check, timeout=20, **kwargs)

    @classmethod
    def http(cls, path, data=None, auth=True, token=None, headers=None):
        headers = dict(headers or {})
        if auth:
            headers["Authorization"] = f"Bearer {token or cls.token}"
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(cls.url + path, data=json.dumps(data).encode() if data is not None else None, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            payload = error.read()
            error.close()
            try:
                return error.code, json.loads(payload)
            except ValueError:
                return error.code, payload.decode()

    @classmethod
    def wait_for(cls, callback, timeout=12):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                if callback():
                    return
            except (OSError, urllib.error.URLError):
                pass
            time.sleep(0.1)
        raise AssertionError("condition timed out")

    def start_job(self, code, node="one", timeout=5, **extra):
        status, job = self.http(f"/api/v1/nodes/{node}/exec", {"program":sys.executable,"args":["-c", code],"timeout_secs":timeout, **extra})
        self.assertEqual(status, 202, job)
        return job["id"]

    def finished(self, job_id, timeout=10):
        self.wait_for(lambda: self.http(f"/api/v1/jobs/{job_id}")[1]["outcome"] != "running", timeout)
        return self.http(f"/api/v1/jobs/{job_id}")[1]

    def test_device_mesh_groups_jobs_and_agent_mcp(self):
        token = (self.path / "one.token").read_text().strip()
        other_token = (self.path / "two.token").read_text().strip()
        mesh = "/api/v1/mesh"
        self.assertEqual(self.http(mesh + "/nodes", token=self.token)[0], 401)
        self.assertEqual(self.http(mesh + "/nodes", token=token, headers={"Origin": self.url})[0], 403)
        self.assertEqual([n["id"] for n in self.http(mesh + "/nodes", token=token)[1]["nodes"]], ["one"])
        request = {"program": sys.executable, "args": ["-c", "print('DEVICE_MESH_OK')"], "timeout_secs": 10}
        self.assertEqual(self.http(mesh + "/nodes/two/exec", request, token=token)[0], 403)
        try:
            for node in ("one", "two"):
                self.cli("group", node, "--group", "personal", "--credentials", self.credentials)
            self.assertEqual({n["id"] for n in self.http(mesh + "/nodes", token=token)[1]["nodes"]}, {"one", "two"})
            status, job = self.http(mesh + "/nodes/two/exec", request, token=token)
            self.assertEqual(status, 202, job)
            job_path = mesh + "/jobs/" + job["id"]
            self.wait_for(lambda: self.http(job_path, token=token)[1]["outcome"] != "running")
            result = self.http(job_path, token=token)[1]
            self.assertIn("DEVICE_MESH_OK", result["stdout"])
            self.assertEqual(result["requester"], "one")
            self.assertEqual(self.http(job_path, token=other_token)[0], 404)
            self.assertEqual(self.http(job_path + "/cancel", {}, token=other_token)[0], 404)
            node_cli = [str(NODE_BINARY), "--server", self.ws, "--id", "one", "--token-file", str(self.path / "one.token")]
            listed = subprocess.run([*node_cli, "nodes"], capture_output=True, text=True, check=True, timeout=15)
            self.assertEqual({n["id"] for n in json.loads(listed.stdout)["nodes"]}, {"one", "two"})
            messages = [{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_nodes","arguments":{}}}]
            mcp = subprocess.run([*node_cli, "mcp"], input="\n".join(map(json.dumps, messages)) + "\n", capture_output=True, text=True, check=True, timeout=15)
            response = json.loads(mcp.stdout)
            self.assertFalse(response["result"].get("isError"), response)
            self.assertEqual({n["id"] for n in json.loads(response["result"]["content"][0]["text"])["nodes"]}, {"one", "two"})
            self.cli("group", "two", "--group", "other", "--credentials", self.credentials)
            self.assertEqual(self.http(mesh + "/nodes/two/exec", request, token=token)[0], 403)
            self.assertEqual(self.http(job_path, token=token)[0], 404)
        finally:
            for node in ("one", "two"):
                self.cli("group", node, "--credentials", self.credentials)

    def test_authentication_separates_controller_and_node(self):
        self.assertEqual(self.http("/api/v1/nodes", auth=False)[0], 401)
        self.assertEqual(self.http("/api/v1/nodes", token="x" * 64)[0], 401)
        self.assertEqual(self.http("/api/v1/nodes", token=(self.path / "one.token").read_text().strip())[0], 401)
        self.assertEqual(self.http("/api/v1/nodes", headers={"Origin":"https://example.invalid"})[0], 403)

    def test_multiple_nodes_and_cli(self):
        jobs = [self.start_job(f"print({node!r})", node=node) for node in ["one", "two"]]
        self.assertEqual([self.finished(j)["stdout"].strip() for j in jobs], ["one", "two"])
        result = self.cli("nodes", "--credentials", self.credentials, "--url", self.url)
        self.assertEqual(len(json.loads(result.stdout)["nodes"]), 2)
        result = self.cli("exec", "--credentials", self.credentials, "--url", self.url, "--node", "one", "--", sys.executable, "-c", "print('CLI')")
        self.assertEqual(json.loads(result.stdout)["stdout"].strip(), "CLI")
        if os.name != "nt":
            environment = {**os.environ, "AGENT_WORKSPACE_RUNTIME_BIN":str(BINARY), "AGENT_WORKSPACE_CONFIG_ROOT":str(self.path)}
            result = subprocess.run(["bash", str(ROOT / "scripts/workspacectl"), "nodes", "--json", "--url", self.url, "--credentials", str(self.credentials)], env=environment, check=True, text=True, capture_output=True)
            self.assertEqual(len(json.loads(result.stdout)["nodes"]), 2)
            result = subprocess.run(["bash", str(ROOT / "scripts/workspacectl"), "node", "exec", "--url", self.url,
                                     "--credentials", str(self.credentials), "--node", "one", "--", sys.executable,
                                     "-c", "import sys; print(sys.argv[1])", "--json"], env=environment, check=True, text=True, capture_output=True)
            self.assertEqual(json.loads(result.stdout)["stdout"].strip(), "--json")

    def test_http_gateway_preserves_web_context_and_strips_credentials(self):
        status, value = self.http("/existing-ui?tab=nodes", headers={"Host":"workspace.example.com",
                                 "Origin":"https://workspace.example.com","Cookie":"aw_session=example",
                                 "X-Forwarded-Host":"attacker.invalid"})
        self.assertEqual(status, 200)
        self.assertEqual(value["host"], "workspace.example.com")
        self.assertEqual(value["origin"], "https://workspace.example.com")
        self.assertEqual(value["cookie"], "aw_session=example")
        self.assertEqual(value["path"], "/existing-ui?tab=nodes")
        self.assertIsNone(value["authorization"])
        self.assertIsNone(value["forwarded"])

    def test_bootstrap_proxy_forwards_only_invitation_credentials(self):
        invitation = "cbi_" + "d" * 64
        node_token = (self.path / "one.token").read_text().strip()
        for path, body, token, forwarded in [
            ("/api/network/bootstrap", {}, invitation, True),
            ("/api/network/bootstrap", {}, self.token, False),
            ("/api/network/bootstrap", {}, node_token, False),
            ("/api/network/bootstrap", {}, "cbi_" + "d" * 63, False),
            ("/api/network/bootstrap", {}, "cbi_" + "z" * 64, False),
            ("/api/network/bootstrap", None, invitation, False),
            ("/api/network/bootstrap?next=private", {}, invitation, False),
            ("/api/network/bootstrap/", {}, invitation, False),
            ("/api/network", {}, invitation, False),
            ("/existing-ui", {}, invitation, False),
        ]:
            with self.subTest(path=path, post=body is not None, forwarded=forwarded):
                status, value = self.http(path, body, token=token)
                self.assertEqual(status, 200)
                self.assertEqual(value["authorization"], "Bearer " + invitation if forwarded else None)

    def test_output_environment_cwd_and_nonzero_exit(self):
        code = "import os,sys; print(os.getcwd()); print(os.environ['TEST_VAR']); sys.stderr.write('错误'); sys.exit(7)"
        job = self.finished(self.start_job(code, cwd=str(self.path), env={"TEST_VAR":"value with spaces; $(echo untouched)"}))
        self.assertEqual(job["outcome"], "exited")
        self.assertEqual(job["exit_code"], 7)
        self.assertIn(str(self.path), job["stdout"])
        self.assertIn("$(echo untouched)", job["stdout"])
        self.assertEqual(job["stderr"], "错误")

    def test_binary_output_and_truncation(self):
        job = self.finished(self.start_job("import sys; sys.stdout.buffer.write(bytes(range(256))*300)"))
        data = base64.b64decode(job["stdout_base64"])
        self.assertTrue(job["truncated"])
        self.assertEqual(data, bytes(range(256))*256)
        self.assertEqual(job["exit_code"], 0)

    def test_timeout_and_cancellation(self):
        job = self.finished(self.start_job("import time; time.sleep(30)", timeout=1))
        self.assertEqual(job["outcome"], "timed_out")
        job_id = self.start_job("import time; time.sleep(30)", timeout=30)
        self.assertEqual(self.http(f"/api/v1/jobs/{job_id}/cancel", {})[0], 200)
        self.assertEqual(self.finished(job_id)["outcome"], "cancelled")

    def test_failed_launch_and_invalid_command(self):
        status, job = self.http("/api/v1/nodes/one/exec", {"program":"no-such-workspace-program"})
        self.assertEqual(status, 202)
        self.assertEqual(self.finished(job["id"])["outcome"], "failed")
        self.assertEqual(self.http("/api/v1/nodes/one/exec", {"program":"test","timeout_secs":0})[0], 400)
        self.assertEqual(self.http("/api/v1/nodes/unknown/exec", {"program":"test"})[0], 404)

    def test_concurrency_is_bounded(self):
        jobs = [self.start_job("import time; time.sleep(30)", timeout=30) for _ in range(4)]
        self.assertEqual(self.http("/api/v1/nodes/one/exec", {"program":sys.executable})[0], 429)
        for job in jobs:
            self.http(f"/api/v1/jobs/{job}/cancel", {})
        for job in jobs:
            self.assertEqual(self.finished(job)["outcome"], "cancelled")

    def test_mcp_exposes_node_tools_and_executes(self):
        requests = [
            {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":2,"method":"tools/list"},
            {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"workspace_exec","arguments":{"node":"two","program":sys.executable,"args":["-c","print('MCP')"]}}},
        ]
        result = self.cli("mcp", "--credentials", self.credentials, "--url", self.url, input="".join(json.dumps(r) + "\n" for r in requests))
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(responses), 3)
        self.assertEqual(len(responses[1]["result"]["tools"]), 5)
        job = json.loads(responses[2]["result"]["content"][0]["text"])
        self.assertEqual(self.finished(job["id"])["stdout"].strip(), "MCP")

    def test_reconnection_and_no_replay(self):
        marker = self.path / "no-replay.txt"
        job = self.start_job(f"from pathlib import Path; import time; f=Path({str(marker)!r}).open('a'); f.write('once'); f.close(); time.sleep(30)", node="two", timeout=30)
        self.wait_for(marker.exists)
        self.stop(self.node_processes["two"])
        self.assertEqual(self.finished(job)["outcome"], "disconnected")
        self.node_processes["two"] = self.start("node", "--server", self.ws, "--id", "two", "--token-file", self.path / "two.token")
        self.wait_for(lambda: all(n["online"] for n in self.http("/api/v1/nodes")[1]["nodes"]))
        self.assertEqual(marker.read_text(), "once")
        self.assertEqual(self.finished(self.start_job("print('reconnected')", node="two"))["stdout"].strip(), "reconnected")

    def test_revocation_closes_existing_connection(self):
        self.cli("enroll", "revoked", "--credentials", self.credentials, "--token-file", self.path / "revoked.token")
        node = self.start("node", "--server", self.ws, "--id", "revoked", "--token-file", self.path / "revoked.token")
        self.wait_for(lambda: any(n["id"] == "revoked" and n["online"] for n in self.http("/api/v1/nodes")[1]["nodes"]))
        job = self.start_job("import time; time.sleep(30)", node="revoked", timeout=30)
        self.cli("revoke", "revoked", "--credentials", self.credentials)
        self.assertEqual(self.finished(job, timeout=15)["outcome"], "disconnected")
        self.stop(node)

    def test_credentials_not_overwritten_or_disclosed(self):
        before = self.credentials.read_bytes()
        result = self.cli("init", "--credentials", self.credentials, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(before, self.credentials.read_bytes())
        self.assertNotIn(self.token, result.stdout + result.stderr)
        if os.name != "nt":
            self.assertEqual(self.credentials.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
