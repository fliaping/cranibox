#!/usr/bin/env python3
"""Safe network configuration mutations for Cranibox."""

from __future__ import annotations

import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
import time


CONFIG_ROOT = pathlib.Path(os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config"))
PROXY_ROOT = CONFIG_ROOT / "proxyctl"
CODE_SERVER_CONFIG = CONFIG_ROOT / ".config/code-server/config.yaml"
RESTART_LOG = CONFIG_ROOT / ".local/log/user-systemd/code-server-restart.log"
DOMAIN_RE = re.compile(
    r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
    r"(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$"
)


def atomic_write(path: pathlib.Path, content: str, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(content, encoding="utf-8")
    os.chmod(temporary, mode)
    temporary.replace(path)


def env_values(path: pathlib.Path) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return values
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, raw = stripped.split("=", 1)
        try:
            parsed = shlex.split(raw, comments=True, posix=True)
        except ValueError:
            continue
        values[key.strip()] = parsed[0] if parsed else ""
    return values


def update_yaml_value(path: pathlib.Path, key: str, value: str) -> None:
    try:
        original = path.read_text(encoding="utf-8")
        mode = path.stat().st_mode & 0o777
    except FileNotFoundError:
        original = ""
        mode = 0o600
    pattern = re.compile(rf"(?m)^[ \t]*{re.escape(key)}[ \t]*:.*$")
    replacement = f"{key}: {value}"
    if pattern.search(original):
        updated = pattern.sub(replacement, original, count=1)
    else:
        updated = original
        if updated and not updated.endswith("\n"):
            updated += "\n"
        updated += replacement + "\n"
    atomic_write(path, updated, mode)


def restart_worker() -> int:
    time.sleep(3)
    env = os.environ.copy()
    env.setdefault("HOME", str(CONFIG_ROOT))
    return subprocess.run(
        ["systemctl", "--user", "restart", "code-server.service"],
        check=False,
        env=env,
    ).returncode


def schedule_restart() -> None:
    RESTART_LOG.parent.mkdir(parents=True, exist_ok=True)
    with RESTART_LOG.open("ab") as output:
        subprocess.Popen(
            [sys.executable, str(pathlib.Path(__file__).resolve()), "--restart-worker"],
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            close_fds=True,
        )


def configure_domain(raw_domain: str) -> int:
    domain = raw_domain.strip().lower().rstrip(".")
    if len(domain) > 253 or not DOMAIN_RE.fullmatch(domain):
        print("domain must be a bare hostname such as workspace.example.com", file=sys.stderr)
        return 2
    routing_installed = (PROXY_ROOT / "bin/caddy").is_file() and (PROXY_ROOT / "env").is_file()
    if routing_installed:
        workspacectl = shutil.which("workspacectl") or str(CONFIG_ROOT / "bin/workspacectl")
        result = subprocess.run([workspacectl, "proxy", "domain", domain], check=False)
    else:
        manager = shutil.which("agent-workspace-manager") or str(CONFIG_ROOT / "bin/agent-workspace-manager")
        env = os.environ.copy()
        env["PROXY_ROOT_DOMAIN"] = domain
        env.setdefault("CODE_SERVER_PROXY_DOMAIN", f"{{{{port}}}}.{domain}")
        env["PROXY_RECONFIGURE"] = "true"
        result = subprocess.run([manager, "install", "proxyctl"], check=False, env=env)
    if result.returncode != 0:
        return result.returncode

    proxy_env = env_values(PROXY_ROOT / "env")
    proxy_domain = proxy_env.get("CODE_SERVER_PROXY_DOMAIN") or f"{{{{port}}}}.{domain}"
    update_yaml_value(CODE_SERVER_CONFIG, "proxy-domain", proxy_domain)
    schedule_restart()
    print(f"custom domain configured: {domain}")
    print("code-server restart scheduled")
    print("Configure DNS, TLS, and gateway authentication before public exposure")
    return 0


def main(argv: list[str]) -> int:
    if argv == ["--restart-worker"]:
        return restart_worker()
    if len(argv) == 2 and argv[0] == "domain":
        return configure_domain(argv[1])
    print("Usage: workspacectl network domain <workspace.example.com>", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
