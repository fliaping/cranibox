#!/usr/bin/env python3
"""Safe mutation backend for global Cranibox MCP and Skill resources."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
from collections.abc import Sequence


CONFIG_ROOT = os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config")
VALID_NAME = re.compile(r"^[A-Za-z0-9_.@-]+$")
AGENT_BINARIES = {
    "codex": "codex",
    "claude-code": "claude",
    "hermes": "hermes",
    "deepseek-harness": "deepseek-harness",
}
DEEPSEEK_REGISTRY = pathlib.Path(CONFIG_ROOT) / ".local/share/agent-workspace/deepseek-mcp.json"
DEEPSEEK_PATCH = pathlib.Path(CONFIG_ROOT) / ".dsh/agent-workspace-mcp.cordis.yml"


def run(
    args: Sequence[str],
    *,
    timeout: float = 15 * 60,
    extra_env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.setdefault("HOME", CONFIG_ROOT)
    if extra_env:
        env.update(extra_env)
    try:
        return subprocess.run(
            list(args),
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        return subprocess.CompletedProcess(list(args), 127, "", str(exc))


def emit_result(label: str, result: subprocess.CompletedProcess[str]) -> None:
    print(f"[{label}] {'complete' if result.returncode == 0 else 'failed'}")
    output = (result.stdout or "").strip()
    error = (result.stderr or "").strip()
    if output:
        print(output)
    if error:
        print(error, file=sys.stderr)


def validate_name(value: str) -> str:
    if not VALID_NAME.fullmatch(value):
        raise ValueError("names may contain only letters, numbers, dot, underscore, @, and dash")
    return value


def parse_agents(value: str) -> list[str]:
    requested = list(AGENT_BINARIES) if value == "all" else [item.strip() for item in value.split(",")]
    if not requested or any(item not in AGENT_BINARIES for item in requested):
        raise ValueError(
            "agents must be all or a comma-separated subset of "
            "codex,claude-code,hermes,deepseek-harness"
        )
    return list(dict.fromkeys(requested))


def skills_command(action: str, name_or_source: str | None) -> int:
    npx = shutil.which("npx")
    if not npx:
        print("npx is required for global Skill management", file=sys.stderr)
        return 1

    if action == "add":
        source = (name_or_source or "").strip()
        if not source or source.startswith("-") or "\x00" in source:
            print("a valid Skill package, repository URL, or local path is required", file=sys.stderr)
            return 2
        args = [npx, "--yes", "skills", "add", source, "--global", "--agent", "*", "--yes"]
    elif action == "update":
        args = [npx, "--yes", "skills", "update"]
        if name_or_source:
            args.append(validate_name(name_or_source))
        args.extend(["--global", "--yes"])
    elif action == "remove":
        args = [
            npx,
            "--yes",
            "skills",
            "remove",
            validate_name(name_or_source or ""),
            "--global",
            "--yes",
        ]
    else:
        raise ValueError(f"unsupported Skill action: {action}")

    result = run(args)
    emit_result(f"skills:{action}", result)
    return result.returncode


def mcp_add_for_agent(
    agent: str,
    name: str,
    transport: str,
    url: str,
    command: list[str],
) -> subprocess.CompletedProcess[str]:
    binary = shutil.which(AGENT_BINARIES[agent])
    if not binary:
        return subprocess.CompletedProcess([AGENT_BINARIES[agent]], 127, "", "Agent CLI is not installed")

    if agent == "deepseek-harness":
        try:
            update_deepseek_mcp(name, transport, url, command)
        except OSError as exc:
            return subprocess.CompletedProcess([binary], 1, "", str(exc))
        return subprocess.CompletedProcess([binary], 0, "DeepSeek Harness MCP patch updated", "")

    if agent == "codex":
        args = [binary, "mcp", "add", name]
        args.extend(["--url", url] if transport == "http" else ["--", *command])
    elif agent == "claude-code":
        args = [binary, "mcp", "add", "--scope", "user"]
        if transport == "http":
            args.extend(["--transport", "http", name, url])
        else:
            args.extend([name, "--", *command])
    else:
        args = [binary, "mcp", "add", name]
        if transport == "http":
            args.extend(["--url", url])
        else:
            args.extend(["--command", command[0], "--args", *command[1:]])
    return run(args, timeout=3 * 60)


def mcp_remove_for_agent(agent: str, name: str) -> subprocess.CompletedProcess[str]:
    binary = shutil.which(AGENT_BINARIES[agent])
    if not binary:
        return subprocess.CompletedProcess([AGENT_BINARIES[agent]], 127, "", "Agent CLI is not installed")
    if agent == "deepseek-harness":
        try:
            remove_deepseek_mcp(name)
        except OSError as exc:
            return subprocess.CompletedProcess([binary], 1, "", str(exc))
        return subprocess.CompletedProcess([binary], 0, "DeepSeek Harness MCP patch updated", "")
    if agent == "claude-code":
        args = [binary, "mcp", "remove", "--scope", "user", name]
    else:
        args = [binary, "mcp", "remove", name]
    return run(args, timeout=2 * 60)


def atomic_write(path: pathlib.Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(content, encoding="utf-8")
    os.chmod(temporary, 0o600)
    temporary.replace(path)


def load_deepseek_registry() -> dict[str, dict[str, object]]:
    try:
        loaded = json.loads(DEEPSEEK_REGISTRY.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    servers = loaded.get("servers", {}) if isinstance(loaded, dict) else {}
    if not isinstance(servers, dict):
        return {}
    return {
        str(name): config
        for name, config in servers.items()
        if VALID_NAME.fullmatch(str(name)) and isinstance(config, dict)
    }


def deepseek_server_name(name: str) -> str:
    normalized = re.sub(r"[^A-Za-z0-9_-]", "_", name)
    if normalized and len(normalized) <= 32:
        return normalized
    digest = hashlib.sha256(name.encode()).hexdigest()[:8]
    prefix = (normalized or "server")[:23]
    return f"{prefix}_{digest}"


def render_deepseek_patch(servers: dict[str, dict[str, object]]) -> None:
    atomic_write(
        DEEPSEEK_REGISTRY,
        json.dumps({"version": 1, "servers": servers}, indent=2, sort_keys=True) + "\n",
    )
    if not servers:
        DEEPSEEK_PATCH.unlink(missing_ok=True)
        return

    lines = [
        "# Managed by Cranibox. User profile patches remain separate.",
        "- insert:",
    ]
    for name in sorted(servers):
        config = servers[name]
        identifier = hashlib.sha256(name.encode()).hexdigest()[:12]
        lines.extend(
            [
                f"    - id: {json.dumps(f'agent-workspace-mcp-{identifier}')}",
                "      name: \"@deepseek-ai/dsh-mcp-client\"",
                "      config:",
                f"        serverName: {json.dumps(deepseek_server_name(name))}",
            ]
        )
        if config.get("transport") == "streamable-http":
            lines.extend(
                [
                    "        transport: \"streamable-http\"",
                    f"        url: {json.dumps(str(config.get('url', '')))}",
                ]
            )
        else:
            args = config.get("args", [])
            safe_args = [str(item) for item in args] if isinstance(args, list) else []
            lines.extend(
                [
                    "        transport: \"stdio\"",
                    f"        command: {json.dumps(str(config.get('command', '')))}",
                    f"        args: {json.dumps(safe_args)}",
                    f"        cwd: {json.dumps(str(pathlib.Path(CONFIG_ROOT) / 'Workspace'))}",
                ]
            )
    atomic_write(DEEPSEEK_PATCH, "\n".join(lines) + "\n")


def restart_deepseek_if_running() -> None:
    if not shutil.which("systemctl"):
        return
    state = run(
        ["systemctl", "--user", "show", "deepseek-harness.service", "--property", "ActiveState"],
        timeout=10,
    )
    if "ActiveState=active" in state.stdout:
        run(
            ["setsid", "systemctl", "--user", "restart", "deepseek-harness.service"],
            timeout=60,
        )


def update_deepseek_mcp(
    name: str,
    transport: str,
    url: str,
    command: list[str],
) -> None:
    servers = load_deepseek_registry()
    if transport == "http":
        servers[name] = {"transport": "streamable-http", "url": url, "enabled": True}
    else:
        servers[name] = {
            "transport": "stdio",
            "command": command[0],
            "args": command[1:],
            "enabled": True,
        }
    render_deepseek_patch(servers)
    restart_deepseek_if_running()


def remove_deepseek_mcp(name: str) -> None:
    servers = load_deepseek_registry()
    servers.pop(name, None)
    render_deepseek_patch(servers)
    restart_deepseek_if_running()


def mcp_command(args: argparse.Namespace) -> int:
    name = validate_name(args.name)
    agents = parse_agents(args.agents)
    failures = 0

    if args.action == "add":
        command: list[str] = []
        url = (args.url or "").strip()
        if args.transport == "http":
            if not re.match(r"^https?://[^\s]+$", url):
                raise ValueError("HTTP MCP URLs must begin with http:// or https://")
        else:
            command = shlex.split(args.command_line or "")
            if not command or command[0].startswith("-"):
                raise ValueError("a valid STDIO command line is required")

        mcpm = shutil.which("mcpm")
        if mcpm:
            mcpm_args = [mcpm, "new", name]
            if args.transport == "http":
                mcpm_args.extend(["--type", "remote", "--url", url])
            else:
                mcpm_args.extend(
                    [
                        "--type",
                        "stdio",
                        "--command",
                        command[0],
                        "--args",
                        shlex.join(command[1:]),
                    ]
                )
            mcpm_args.append("--force")
            result = run(
                mcpm_args,
                extra_env={"MCPM_NON_INTERACTIVE": "true", "MCPM_FORCE": "true"},
            )
            emit_result("mcpm", result)
            if result.returncode != 0:
                return result.returncode
            # All clients can consume the same global MCPM definition through
            # its stdio runner, including clients without a native MCPM adapter.
            command = [mcpm, "run", name]
            url = ""
            transport = "stdio"
        else:
            transport = args.transport

        for agent in agents:
            result = mcp_add_for_agent(agent, name, transport, url, command)
            emit_result(agent, result)
            failures += result.returncode != 0
    else:
        for agent in agents:
            result = mcp_remove_for_agent(agent, name)
            emit_result(agent, result)
            failures += result.returncode != 0
        if args.purge_global and shutil.which("mcpm"):
            result = run(
                [shutil.which("mcpm") or "mcpm", "uninstall", name, "--force"],
                extra_env={"MCPM_NON_INTERACTIVE": "true", "MCPM_FORCE": "true"},
            )
            emit_result("mcpm", result)
            failures += result.returncode != 0
    return 1 if failures else 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description="Manage global Cranibox MCP and Skill resources")
    groups = root.add_subparsers(dest="group", required=True)

    skill = groups.add_parser("skill")
    skill_commands = skill.add_subparsers(dest="action", required=True)
    add_skill = skill_commands.add_parser("add")
    add_skill.add_argument("source")
    update_skill = skill_commands.add_parser("update")
    update_skill.add_argument("name", nargs="?")
    remove_skill = skill_commands.add_parser("remove")
    remove_skill.add_argument("name")

    mcp = groups.add_parser("mcp")
    mcp_commands = mcp.add_subparsers(dest="action", required=True)
    add_mcp = mcp_commands.add_parser("add")
    add_mcp.add_argument("name")
    add_mcp.add_argument("--transport", choices=("stdio", "http"), required=True)
    add_mcp.add_argument("--url")
    add_mcp.add_argument("--command-line")
    add_mcp.add_argument("--agents", default="all")
    remove_mcp = mcp_commands.add_parser("remove")
    remove_mcp.add_argument("name")
    remove_mcp.add_argument("--agents", default="all")
    remove_mcp.add_argument("--purge-global", action="store_true")
    return root


def main(argv: list[str]) -> int:
    args = parser().parse_args(argv)
    try:
        if args.group == "skill":
            value = getattr(args, "source", None) or getattr(args, "name", None)
            return skills_command(args.action, value)
        return mcp_command(args)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
