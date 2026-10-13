#!/usr/bin/env python3
"""Update OpenClaw through Agent Workspace's lightweight service manager."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config"))
RELEASES = ROOT / "opt/openclaw/releases"
STATE = ROOT / ".local/state/agent-workspace"
UNIT = ROOT / ".config/systemd/user/openclaw-gateway.service"
ALIAS = ROOT / ".local/bin/openclaw"
VERSION_RE = re.compile(r"^\d{4}\.\d{1,2}\.\d+(?:[-+][A-Za-z0-9.-]+)?$")


def run(args: list[str], timeout: int = 1800) -> str:
    result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
    if result.returncode:
        # npm diagnostics may echo private registry credentials or proxy headers.
        raise RuntimeError(f"{Path(args[0]).name} failed (exit {result.returncode})")
    return result.stdout.strip()


def package_version(entry: Path) -> str:
    metadata = entry.parent / "package.json"
    if not metadata.is_file():
        raise RuntimeError(f"OpenClaw package metadata is missing: {metadata}")
    version = json.loads(metadata.read_text())["version"]
    if not isinstance(version, str) or not VERSION_RE.fullmatch(version):
        raise RuntimeError("OpenClaw package has an invalid version")
    return version


def service_command(text: str) -> tuple[Path, list[str]]:
    commands = [line.removeprefix("ExecStart=") for line in text.splitlines()
                if line.startswith("ExecStart=")]
    if len(commands) != 1:
        raise RuntimeError("Expected exactly one Gateway ExecStart")
    args = shlex.split(commands[0])
    if args and Path(args[0]).name.startswith("node") and len(args) > 1:
        args = args[1:]
    if len(args) < 2 or Path(args[0]).name not in ("openclaw", "openclaw.mjs") or args[1] != "gateway":
        raise RuntimeError("Unsupported Gateway ExecStart; expected openclaw gateway ...")
    if not Path(args[0]).is_absolute():
        raise RuntimeError("Gateway ExecStart must use an absolute OpenClaw path")
    return Path(args[0]), args[1:]


def managed_unit(original: str, entry: Path, node: str) -> str:
    _, arguments = service_command(original)
    lines = original.splitlines(keepends=True)
    for index, line in enumerate(lines):
        if not line.startswith("ExecStart="):
            continue
        lines[index] = "ExecStart=" + shlex.join([node, str(entry), *arguments]) + "\n"
        break
    else:
        raise RuntimeError("Gateway service has no ExecStart")
    mode = "Environment=OPENCLAW_SUPERVISOR_MODE=external\n"
    for index, line in enumerate(lines):
        if line.startswith("Environment=OPENCLAW_SUPERVISOR_MODE="):
            lines[index] = mode
            break
    else:
        index = next((i for i, line in enumerate(lines) if line.strip() == "[Service]"), None)
        if index is None:
            raise RuntimeError("Gateway unit has no [Service] section")
        lines.insert(index + 1, mode)
    return "".join(lines)


def atomic_write(path: Path, content: bytes, mode: int) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_bytes(content)
        temporary.chmod(mode)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def restart() -> None:
    # The caller must not be part of the Gateway process tree.
    log = ROOT / ".local/log/user-systemd/openclaw-workspace-update.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("ab") as output:
        child = subprocess.Popen(
            ["systemctl", "--user", "restart", "openclaw-gateway.service"],
            stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        if child.wait(timeout=40):
            raise RuntimeError(f"Gateway restart failed; see {log}")


def verify(entry: Path, timeout: int = 45) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            show = run(["systemctl", "--user", "show", "openclaw-gateway.service",
                        "--property=ActiveState,MainPID"], timeout=5)
            values = dict(line.split("=", 1) for line in show.splitlines() if "=" in line)
            pid = int(values.get("MainPID", "0"))
            if values.get("ActiveState") == "active" and pid > 0:
                cmdline = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
                if os.fsencode(str(entry)) in cmdline:
                    try:
                        urllib.request.urlopen("http://127.0.0.1:18789/", timeout=2).close()
                        return
                    except urllib.error.HTTPError as error:
                        if error.code < 500:
                            return
                    except (OSError, urllib.error.URLError):
                        pass
        except (OSError, ValueError, RuntimeError):
            pass
        time.sleep(1)
    raise RuntimeError("Gateway did not start the requested release and answer on port 18789")


def update() -> None:
    if not UNIT.is_file():
        raise RuntimeError("OpenClaw gateway service is not installed")
    pid_file = ROOT / ".local/run/user-systemd/openclaw-gateway.pid"
    if pid_file.is_file():
        try:
            gateway_pid = int(pid_file.read_text().strip())
            ancestor = os.getpid()
            while ancestor > 1:
                if ancestor == gateway_pid:
                    raise RuntimeError("Run this update from Control Center or a terminal outside the OpenClaw Gateway")
                status = Path(f"/proc/{ancestor}/status").read_text()
                ancestor = int(next(line.split()[1] for line in status.splitlines() if line.startswith("PPid:")))
        except (OSError, ValueError, StopIteration):
            pass
    STATE.mkdir(parents=True, exist_ok=True)
    with (STATE / "openclaw-update.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("An OpenClaw update is already running") from error
        original = UNIT.read_bytes()
        text = original.decode()
        old_entry, _ = service_command(text)
        node, npm = shutil.which("node"), shutil.which("npm")
        if not node or not npm:
            raise RuntimeError("Node.js and npm are required")
        version = json.loads(run([npm, "view", "openclaw@latest", "version", "--json"], timeout=30))
        if not isinstance(version, str) or not VERSION_RE.fullmatch(version):
            raise RuntimeError("npm returned an invalid OpenClaw version")
        RELEASES.mkdir(parents=True, exist_ok=True)
        release = RELEASES / version
        entry = release / "node_modules/openclaw/openclaw.mjs"
        if not entry.is_file() or package_version(entry) != version:
            if release.exists():
                raise RuntimeError(f"Incomplete release already exists: {release}")
            staging = Path(tempfile.mkdtemp(prefix=".staging-", dir=RELEASES))
            try:
                print(f"Installing OpenClaw {version} into {release}", flush=True)
                run([npm, "install", "--prefix", str(staging), "--no-save",
                     "--no-audit", "--no-fund", f"openclaw@{version}"], timeout=1800)
                staged = staging / "node_modules/openclaw/openclaw.mjs"
                if package_version(staged) != version:
                    raise RuntimeError("Installed package version differs from npm latest")
                run([node, str(staged), "--version"], timeout=30)
                staging.replace(release)
            finally:
                if staging.exists():
                    shutil.rmtree(staging)
        next_unit = managed_unit(text, entry, node)
        if next_unit == text:
            verify(entry)
            print(f"OpenClaw {version} is already active")
            return
        backup = STATE / f"openclaw-gateway.service.{int(time.time())}.{os.getpid()}.bak"
        backup.write_bytes(original)
        backup.chmod(0o600)
        print(f"Switching Gateway; previous service definition saved at {backup}", flush=True)
        mode = UNIT.stat().st_mode & 0o777
        atomic_write(UNIT, next_unit.encode(), mode)
        try:
            restart()
            verify(entry)
        except Exception:
            atomic_write(UNIT, original, mode)
            try:
                restart()
                verify(old_entry, timeout=30)
                print("Previous Gateway restored", file=sys.stderr)
            except Exception as restore_error:
                print(f"Automatic service recovery failed: {restore_error}", file=sys.stderr)
            raise
        ALIAS.parent.mkdir(parents=True, exist_ok=True)
        if not ALIAS.exists() or (ALIAS.is_symlink() and str(ALIAS.resolve()).startswith(str(RELEASES) + "/")):
            alias_tmp = ALIAS.with_name(f".{ALIAS.name}.{os.getpid()}.tmp")
            alias_tmp.symlink_to(entry)
            alias_tmp.replace(ALIAS)
        else:
            print(f"Gateway updated, but existing CLI alias was preserved: {ALIAS}", file=sys.stderr)
        print(f"OpenClaw {version} is running; Gateway and HTTP checks passed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["update"])
    parser.parse_args()
    try:
        update()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"OpenClaw update failed: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
