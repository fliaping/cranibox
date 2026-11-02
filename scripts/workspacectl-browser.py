#!/usr/bin/env python3
"""Connect trusted Agents to the Chromium window used in the Selkies desktop."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from collections.abc import Sequence


CONFIG_ROOT = pathlib.Path(os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config"))
PROFILE_ROOT = pathlib.Path(
    os.environ.get("AGENT_WORKSPACE_BROWSER_PROFILE", str(CONFIG_ROOT / ".config/agent-browser"))
)
LEGACY_PROFILE = pathlib.Path(
    os.environ.get("AGENT_WORKSPACE_BROWSER_LEGACY_PROFILE", str(CONFIG_ROOT / ".config/chromium"))
)
PROFILE_NAME = os.environ.get(
    "AGENT_WORKSPACE_BROWSER_PROFILE_NAME", "Cranibox (Managed)"
).strip() or "Cranibox (Managed)"
try:
    DEBUG_PORT = int(os.environ.get("AGENT_WORKSPACE_BROWSER_PORT", "9222"))
except ValueError:
    DEBUG_PORT = 9222
if not 1024 <= DEBUG_PORT <= 65535:
    DEBUG_PORT = 9222
MIN_MANAGED_VERSION = 136
AGENT_BINARIES = {
    "codex": "codex",
    "claude-code": "claude",
    "hermes": "hermes",
    "deepseek-harness": "deepseek-harness",
}


def run(args: Sequence[str], timeout: float = 30) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.setdefault("HOME", str(CONFIG_ROOT))
    return subprocess.run(
        list(args),
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
    )


def browser_binary() -> str:
    configured = os.environ.get("AGENT_WORKSPACE_BROWSER_BIN", "").strip()
    if configured:
        return configured
    return (
        shutil.which("chromium")
        or shutil.which("google-chrome")
        or ""
    )


def browser_launcher() -> str:
    return (
        shutil.which("agent-workspace-browser")
        or str(CONFIG_ROOT / "bin/agent-workspace-browser")
    )


def desktop_display(env: dict[str, str]) -> str:
    configured = os.environ.get("AGENT_WORKSPACE_BROWSER_DISPLAY", "").strip()
    if configured:
        return configured

    current = env.get("DISPLAY", "").strip()
    match = re.fullmatch(r"(?:(?:localhost|unix))?:(\d+)(?:\.\d+)?", current)
    if match and pathlib.Path(f"/tmp/.X11-unix/X{match.group(1)}").exists():
        return current

    sockets = sorted(
        pathlib.Path("/tmp/.X11-unix").glob("X[0-9]*"),
        key=lambda item: int(item.name[1:]) if item.name[1:].isdigit() else 65536,
    )
    if sockets and sockets[0].name[1:].isdigit():
        return f":{sockets[0].name[1:]}"
    return current or ":0"


def browser_version(binary: str) -> tuple[str, int]:
    if not binary:
        return "", 0
    try:
        result = run([binary, "--version"], timeout=5)
    except (OSError, subprocess.TimeoutExpired):
        return "", 0
    version = (result.stdout or result.stderr).strip()
    match = re.search(r"\b(\d+)(?:\.\d+){2,3}\b", version)
    return version, int(match.group(1)) if match else 0


def chromium_processes() -> tuple[list[int], list[int]]:
    managed = []
    other = []
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / "cmdline").read_bytes().split(b"\0")
        except OSError:
            continue
        if not command:
            continue
        decoded = [item.decode(errors="ignore") for item in command if item]
        if not decoded:
            continue
        command_line = " ".join(decoded)
        executable = pathlib.Path(decoded[0].split(maxsplit=1)[0]).name
        if (
            executable in {"chromium", "chrome", "google-chrome", "wrapped-chromium"}
            and "--type=" not in command_line
        ):
            target = managed if f"--user-data-dir={PROFILE_ROOT}" in command_line else other
            target.append(int(entry.name))
    return sorted(managed), sorted(other)


def devtools_endpoint() -> tuple[bool, int]:
    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{DEBUG_PORT}/json/version", timeout=0.5
        ) as response:
            payload = json.loads(response.read().decode("utf-8"))
        ready = isinstance(payload, dict) and bool(payload.get("webSocketDebuggerUrl"))
        return ready, DEBUG_PORT
    except (OSError, urllib.error.URLError, json.JSONDecodeError):
        return False, DEBUG_PORT


def configured_agents() -> list[str]:
    markers = {
        "codex": CONFIG_ROOT / ".codex/config.toml",
        "claude-code": CONFIG_ROOT / ".claude.json",
        "hermes": CONFIG_ROOT / ".hermes/config.yaml",
        "deepseek-harness": CONFIG_ROOT / ".local/share/agent-workspace/deepseek-mcp.json",
    }
    configured = []
    for agent, path in markers.items():
        try:
            content = path.read_text(encoding="utf-8")
        except OSError:
            continue
        if "chrome-devtools" in content:
            configured.append(agent)
    return configured


def managed_browser_is_default() -> bool:
    entries = [
        CONFIG_ROOT / ".local/share/applications/chromium.desktop",
        pathlib.Path("/usr/share/applications/chromium.desktop"),
    ]
    for entry in entries:
        try:
            if "agent-workspace-browser" in entry.read_text(encoding="utf-8"):
                return True
        except OSError:
            continue
    return False


def visible_profile_name() -> str:
    profile_directory = "Default"
    try:
        local_state = json.loads((PROFILE_ROOT / "Local State").read_text(encoding="utf-8"))
        candidate = local_state.get("profile", {}).get("last_used", "Default")
        if isinstance(candidate, str) and re.fullmatch(r"Default|Profile [0-9]+", candidate):
            profile_directory = candidate
    except (OSError, json.JSONDecodeError, AttributeError):
        pass
    try:
        preferences = json.loads(
            (PROFILE_ROOT / profile_directory / "Preferences").read_text(encoding="utf-8")
        )
        current = preferences.get("profile", {}).get("name", "")
        if isinstance(current, str) and current.strip():
            return current.strip()
    except (OSError, json.JSONDecodeError, AttributeError):
        pass
    return ""


def state() -> dict[str, object]:
    binary = browser_binary()
    version, major = browser_version(binary)
    processes, other_processes = chromium_processes()
    debugging, port = devtools_endpoint()
    agents = configured_agents()
    supported = major >= MIN_MANAGED_VERSION
    migration_pending = not PROFILE_ROOT.exists() and (LEGACY_PROFILE / "Local State").is_file()
    current_profile_name = visible_profile_name()
    if not binary or not supported:
        status = "unsupported"
    elif not agents:
        status = "needs-setup"
    elif migration_pending and other_processes:
        status = "needs-restart"
    elif other_processes and not processes:
        status = "needs-restart"
    elif not processes:
        status = "not-running"
    elif not debugging:
        status = "needs-restart"
    else:
        status = "ready"
    return {
        "installed": bool(binary),
        "supported": supported,
        "state": status,
        "binary": binary,
        "version": version,
        "major_version": major,
        "profile": str(PROFILE_ROOT),
        "profile_name": current_profile_name or PROFILE_NAME,
        "desired_profile_name": PROFILE_NAME,
        "profile_name_applied": current_profile_name == PROFILE_NAME,
        "running": bool(processes),
        "pids": processes,
        "other_browser_pids": other_processes,
        "remote_debugging": debugging,
        "debug_port": port,
        "mcp_agents": agents,
        "mcp_configured": bool(agents),
        "connection_mode": "managed-loopback-cdp",
        "legacy_profile": str(LEGACY_PROFILE),
        "profile_exists": PROFILE_ROOT.exists(),
        "migration_pending": migration_pending,
        "legacy_browser_running": bool(other_processes),
        "managed_default": managed_browser_is_default(),
        "exposed": False,
    }


def print_status(as_json: bool) -> int:
    current = state()
    if as_json:
        print(json.dumps(current, ensure_ascii=False, separators=(",", ":")))
        return 0
    print("Desktop browser control")
    print(f"  browser:          {current['version'] or 'not installed'}")
    print(f"  running:          {'yes' if current['running'] else 'no'}")
    print(f"  user profile:     {current['profile']}")
    print(f"  profile name:     {current['profile_name']}")
    if not current["profile_name_applied"]:
        print(f"  name on restart:  {current['desired_profile_name']}")
    print(f"  Agent MCP:        {', '.join(current['mcp_agents']) if current['mcp_agents'] else 'not configured'}")
    print(f"  control endpoint: {'ready' if current['remote_debugging'] else 'not running'}")
    print("  default access:   trusted container Agents (no per-session prompt)")
    print("  network exposure: none (127.0.0.1 only)")
    if current["state"] == "ready":
        print("Ready: configured Agents can operate the user's managed Chromium window.")
    elif current["state"] == "needs-restart":
        print("Next: close any legacy Chromium window, then run: workspacectl browser open")
    elif current["state"] == "needs-setup":
        print("Next: workspacectl browser setup")
    elif current["state"] == "not-running":
        print("Next: workspacectl browser open")
    else:
        print(f"Chromium {MIN_MANAGED_VERSION}+ is required for managed browser control.")
    return 0


def open_browser() -> int:
    binary = browser_launcher()
    if not pathlib.Path(binary).is_file() or not os.access(binary, os.X_OK):
        print(f"Managed browser launcher is unavailable: {binary}", file=sys.stderr)
        return 1
    log_path = CONFIG_ROOT / ".local/log/agent-workspace-browser.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.setdefault("HOME", str(CONFIG_ROOT))
    env["DISPLAY"] = desktop_display(env)
    try:
        with log_path.open("ab") as output:
            process = subprocess.Popen(
                [binary, "about:blank"],
                stdin=subprocess.DEVNULL,
                stdout=output,
                stderr=subprocess.STDOUT,
                env=env,
                start_new_session=True,
                close_fds=True,
            )
    except OSError as exc:
        print(f"Could not open Chromium: {exc}", file=sys.stderr)
        return 1
    try:
        return_code = process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        return_code = None
    if return_code not in {None, 0}:
        print(
            "Managed Chromium could not start. Close the existing Chromium once, then try again. "
            f"See {log_path}",
            file=sys.stderr,
        )
        return return_code
    print(f"Opened the managed desktop browser with profile {PROFILE_ROOT}")
    return 0


def setup() -> int:
    binary = browser_binary()
    version, major = browser_version(binary)
    if not binary or major < MIN_MANAGED_VERSION:
        print(
            f"Chromium {MIN_MANAGED_VERSION}+ is required; found {version or 'no browser'}",
            file=sys.stderr,
        )
        return 1
    agents = [agent for agent, command in AGENT_BINARIES.items() if shutil.which(command)]
    if not agents:
        print("Install and sign in to at least one Agent before enabling browser control", file=sys.stderr)
        return 1
    helper = pathlib.Path(__file__).resolve().with_name("workspacectl-resources.py")
    if not helper.is_file():
        print(f"MCP resource helper is unavailable: {helper}", file=sys.stderr)
        return 1
    command_line = (
        "npx --yes chrome-devtools-mcp@latest "
        f"--browser-url=http://127.0.0.1:{DEBUG_PORT} --no-usage-statistics"
    )
    result = subprocess.run(
        [
            sys.executable,
            str(helper),
            "mcp",
            "add",
            "chrome-devtools",
            "--transport",
            "stdio",
            "--command-line",
            command_line,
            "--agents",
            ",".join(agents),
        ],
        check=False,
    )
    if result.returncode != 0:
        return result.returncode
    opened = open_browser()
    print("Managed browser control is configured for all installed Agents.")
    print("Future Agent sessions connect automatically without an Allow prompt.")
    print("Start a new Agent session after changing MCP configuration.")
    print("Security: container Agents can read and operate all tabs and logged-in sessions in this profile.")
    return opened


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(prog="workspacectl browser")
    commands = root.add_subparsers(dest="command", required=True)
    status = commands.add_parser("status", help="show desktop browser control readiness")
    status.add_argument("--json", action="store_true")
    commands.add_parser("setup", help="configure global Agent MCP and start managed Chromium")
    commands.add_parser("open", help="open the managed desktop browser")
    commands.add_parser("approve", help=argparse.SUPPRESS)
    return root


def main(argv: list[str]) -> int:
    args = parser().parse_args(argv)
    if args.command == "status":
        return print_status(args.json)
    if args.command == "setup":
        return setup()
    return open_browser()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
