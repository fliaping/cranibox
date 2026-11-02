#!/usr/bin/env python3
"""Persist the Cranibox UI locale and restart code-server safely."""

from __future__ import annotations

import json
import hashlib
import os
import pathlib
import re
import subprocess
import sys
import time


CONFIG_ROOT = pathlib.Path(os.environ.get("AGENT_WORKSPACE_CONFIG_ROOT", "/config"))
STATE_FILE = CONFIG_ROOT / ".local/state/agent-workspace/locale"
CODE_SERVER_CONFIG = pathlib.Path(
    os.environ.get(
        "CODE_SERVER_CONFIG",
        str(CONFIG_ROOT / ".config/code-server/config.yaml"),
    )
)
RESTART_LOG = CONFIG_ROOT / ".local/log/user-systemd/code-server-restart.log"
EXTENSIONS_ROOT = CONFIG_ROOT / ".local/share/code-server/extensions"
LANGUAGE_PACKS_FILE = CONFIG_ROOT / ".local/share/code-server/languagepacks.json"
CHINESE_PACK_ID = "ms-ceintl.vscode-language-pack-zh-hans"
SUPPORTED = {"en": "English", "zh-cn": "简体中文"}
ALIASES = {
    "en": "en",
    "en-us": "en",
    "english": "en",
    "zh": "zh-cn",
    "zh-cn": "zh-cn",
    "zh-hans": "zh-cn",
    "chinese": "zh-cn",
}


def normalize(value: str) -> str:
    locale = ALIASES.get(value.strip().lower())
    if not locale:
        raise ValueError("locale must be en or zh-cn")
    return locale


def atomic_write(path: pathlib.Path, content: str, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(content, encoding="utf-8")
    os.chmod(temporary, mode)
    temporary.replace(path)


def configured_locale() -> str:
    try:
        return normalize(STATE_FILE.read_text(encoding="utf-8").splitlines()[0])
    except (OSError, IndexError, ValueError):
        pass
    try:
        config = CODE_SERVER_CONFIG.read_text(encoding="utf-8")
    except OSError:
        return "en"
    match = re.search(r"(?m)^\s*locale\s*:\s*['\"]?([^\s#'\"]+)", config)
    if not match:
        return "en"
    try:
        return normalize(match.group(1))
    except ValueError:
        return "en"


def update_code_server_config(locale: str) -> None:
    try:
        original = CODE_SERVER_CONFIG.read_text(encoding="utf-8")
        mode = CODE_SERVER_CONFIG.stat().st_mode & 0o777
    except FileNotFoundError:
        original = ""
        mode = 0o600
    pattern = re.compile(r"(?m)^[ \t]*locale[ \t]*:.*$")
    replacement = f"locale: {locale}"
    if pattern.search(original):
        updated = pattern.sub(replacement, original, count=1)
    else:
        updated = original
        if updated and not updated.endswith("\n"):
            updated += "\n"
        updated += replacement + "\n"
    atomic_write(CODE_SERVER_CONFIG, updated, mode)


def register_chinese_language_pack() -> None:
    """Build VS Code's language pack cache after a CLI extension install."""
    try:
        installed = json.loads(
            (EXTENSIONS_ROOT / "extensions.json").read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(
            "Simplified Chinese language pack is not registered; reinstall code-server extensions"
        ) from exc

    entry = next(
        (
            item
            for item in installed
            if isinstance(item, dict)
            and str(item.get("identifier", {}).get("id", "")).lower()
            == CHINESE_PACK_ID
        ),
        None,
    )
    if not entry:
        raise RuntimeError(
            "Simplified Chinese language pack is missing; run: "
            "agent-workspace-manager install code-server-extensions"
        )
    location = entry.get("location", {})
    extension_path = pathlib.Path(
        str(location.get("fsPath") or location.get("path") or "")
    )
    try:
        manifest = json.loads((extension_path / "package.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError("Simplified Chinese language pack manifest is unreadable") from exc

    localizations = manifest.get("contributes", {}).get("localizations", [])
    localization = next(
        (
            item
            for item in localizations
            if isinstance(item, dict) and item.get("languageId") == "zh-cn"
        ),
        None,
    )
    if not localization:
        raise RuntimeError("Simplified Chinese translations are unavailable")

    identifier = {"id": CHINESE_PACK_ID}
    uuid = entry.get("identifier", {}).get("uuid") or entry.get("metadata", {}).get("id")
    if uuid:
        identifier["uuid"] = str(uuid)
    version = str(manifest.get("version") or entry.get("version") or "")
    translations = {
        str(item["id"]): str((extension_path / str(item["path"])).resolve())
        for item in localization.get("translations", [])
        if isinstance(item, dict) and item.get("id") and item.get("path")
    }
    digest = hashlib.md5(usedforsecurity=False)
    digest.update(str(uuid or CHINESE_PACK_ID).encode("utf-8"))
    digest.update(version.encode("utf-8"))
    pack = {
        "hash": digest.hexdigest(),
        "extensions": [{"extensionIdentifier": identifier, "version": version}],
        "translations": translations,
        "label": str(
            localization.get("localizedLanguageName")
            or localization.get("languageName")
            or "简体中文"
        ),
    }
    try:
        cache = json.loads(LANGUAGE_PACKS_FILE.read_text(encoding="utf-8"))
        if not isinstance(cache, dict):
            cache = {}
        mode = LANGUAGE_PACKS_FILE.stat().st_mode & 0o777
    except (OSError, json.JSONDecodeError):
        cache = {}
        mode = 0o600
    cache["zh-cn"] = pack
    atomic_write(
        LANGUAGE_PACKS_FILE,
        json.dumps(cache, ensure_ascii=False, separators=(",", ":")),
        mode,
    )


def restart_worker() -> int:
    time.sleep(3)
    env = os.environ.copy()
    env.setdefault("HOME", str(CONFIG_ROOT))
    result = subprocess.run(
        ["systemctl", "--user", "restart", "code-server.service"],
        check=False,
        env=env,
    )
    return result.returncode


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


def state_payload() -> dict[str, object]:
    current = configured_locale()
    return {
        "current": current,
        "label": SUPPORTED[current],
        "supported": [
            {"id": locale, "label": label} for locale, label in SUPPORTED.items()
        ],
        "restart_required": True,
    }


def main(argv: list[str]) -> int:
    if argv == ["--restart-worker"]:
        return restart_worker()
    if argv == ["--register-language-pack"]:
        try:
            register_chinese_language_pack()
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1
        print("Simplified Chinese language pack registered")
        return 0
    json_output = "--json" in argv
    restart = "--restart" in argv
    positional = [item for item in argv if item not in {"--json", "--restart"}]
    if len(positional) > 1:
        print("Usage: workspacectl locale [en|zh-cn] [--restart] [--json]", file=sys.stderr)
        return 2
    if positional:
        try:
            locale = normalize(positional[0])
        except ValueError as exc:
            print(str(exc), file=sys.stderr)
            return 2
        if locale == "zh-cn":
            try:
                register_chinese_language_pack()
            except RuntimeError as exc:
                print(str(exc), file=sys.stderr)
                return 1
        atomic_write(STATE_FILE, locale + "\n")
        update_code_server_config(locale)
        if restart:
            schedule_restart()
    payload = state_payload()
    if json_output:
        print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    else:
        print(f"Interface language: {payload['label']} ({payload['current']})")
        if positional and restart:
            print("code-server restart scheduled")
        elif positional:
            print("Restart code-server.service to apply the new language")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
