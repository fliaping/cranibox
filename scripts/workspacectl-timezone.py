#!/usr/bin/env python3
"""Persistent Workspace timezone configuration; never infer location from an IP."""
import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

ROOT = Path(os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config"))
STATE = ROOT / ".config/agent-workspace/timezone"
PREVIOUS = STATE.with_name("timezone.previous")
ZONE_ROOT = Path("/usr/share/zoneinfo")

def validate(zone):
    if not re.fullmatch(r"[A-Za-z0-9_+.-]+(?:/[A-Za-z0-9_+.-]+)*", zone) or ".." in zone.split("/"):
        raise ValueError("Use an IANA timezone, for example America/Phoenix")
    try:
        ZoneInfo(zone)
    except (ValueError, ZoneInfoNotFoundError) as exc:
        raise ValueError("Unknown IANA timezone: " + zone) from exc
    if not (ZONE_ROOT / zone).is_file():
        raise ValueError("System zoneinfo file is missing: " + zone)
    return zone

def read(path):
    return path.read_text().strip() if path.is_file() else None

def atomic(path, text, mode=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    current_mode = path.stat().st_mode & 0o777 if path.exists() else 0o600
    temporary = path.with_name(path.name + ".tmp-" + str(os.getpid()))
    try:
        temporary.write_text(text)
        temporary.chmod(current_mode if mode is None else mode)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)

def system_zone():
    try:
        return str(Path("/etc/localtime").resolve().relative_to(ZONE_ROOT))
    except ValueError:
        return None

def apply_system(zone):
    prefix = [] if os.geteuid() == 0 else ["sudo", "-n"]
    subprocess.run(prefix + ["ln", "-snf", str(ZONE_ROOT / validate(zone)), "/etc/localtime"], check=True)

def install_hooks():
    state_path = shlex.quote(str(STATE))
    block = ("# BEGIN Workspace timezone\n"
             + "if [ -r " + state_path + " ]; then\n"
             + "  IFS= read -r TZ < " + state_path + "\n  export TZ\nfi\n"
             + "# END Workspace timezone\n")
    for name in (".profile", ".bashrc"):
        path = ROOT / name
        original = path.read_text() if path.exists() else ""
        content = re.sub(r"(?m)^# BEGIN Workspace timezone\n.*?^# END Workspace timezone\n?", "", original, flags=re.S)
        # Replace only simple timezone exports; retain all unrelated shell settings.
        content = re.sub(r"(?m)^export TZ=[A-Za-z0-9_/+.-]+\s*$", "", content)
        # Read TZ before interactive-shell guards or commands that may block.
        content = block + "\n" + content.strip() + "\n"
        if content != original:
            backup = path.with_name(path.name + ".before-workspace-timezone")
            if path.exists() and not backup.exists():
                atomic(backup, original)
            atomic(path, content)
    # The existing persistent custom-service registrar restores system state at boot.
    helper = shlex.quote(str(Path(__file__).resolve()))
    service = ROOT / "custom-services.d/workspace-timezone/run"
    script = ("#!/bin/sh\n"
              + "AGENT_WORKSPACE_CONFIG_ROOT=" + shlex.quote(str(ROOT))
              + " python3 " + helper + " apply || exit $?\nexec sleep infinity\n")
    atomic(service, script, 0o755)

def set_zone(zone, config_only=False):
    zone = validate(zone)
    previous = read(STATE) or system_zone()
    if not config_only:
        apply_system(zone)
    install_hooks()
    if previous and previous != zone:
        atomic(PREVIOUS, previous + "\n")
    atomic(STATE, zone + "\n")
    atomic(STATE.with_name("timezone.env"), "TZ=" + zone + "\n")
    atomic(STATE.with_name("timezone.compose.yaml"),
           "services:\n  agent-workspace:\n    environment:\n      TZ: " + json.dumps(zone) + "\n")

def status():
    zone = read(STATE)
    now = datetime.now(ZoneInfo(validate(zone))) if zone else None
    return {"configured": zone, "system": system_zone(),
            "process_TZ": os.environ.get("TZ"), "previous": read(PREVIOUS),
            "offset": now.strftime("%z") if now else None,
            "local_time": now.isoformat() if now else None,
            "container_env_file": str(STATE.with_name("timezone.env")),
            "compose_override": str(STATE.with_name("timezone.compose.yaml")),
            "browser_verification": "not_checked",
            "note": "Existing shells, services and browsers may require restart; IP and fonts are unchanged."}

def main(argv=None):
    parser = argparse.ArgumentParser(prog="workspacectl timezone")
    parser.add_argument("command", choices=["status", "set", "rollback", "apply"], nargs="?", default="status")
    parser.add_argument("zone", nargs="?")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--config-only", action="store_true", help="persist settings without changing /etc/localtime")
    args = parser.parse_args(argv)
    try:
        if args.zone and args.command != "set":
            parser.error("a timezone argument is only accepted by set")
        if args.command in ("set", "rollback"):
            zone = read(PREVIOUS) if args.command == "rollback" else args.zone
            if not zone:
                raise ValueError("Supply a timezone, or ensure a previous setting exists for rollback")
            set_zone(zone, args.config_only)
        elif args.command == "apply":
            zone = read(STATE)
            if not zone:
                raise ValueError("No persisted timezone; run workspacectl timezone set <IANA-zone>")
            if not args.config_only:
                apply_system(zone)
        payload = status()
        print(json.dumps(payload, ensure_ascii=False, indent=2) if args.json else
              "\n".join(f"{key}: {value}" for key, value in payload.items()))
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        print("Timezone configuration failed: " + str(exc), file=sys.stderr)
        return 1

if __name__ == "__main__":
    sys.exit(main())
