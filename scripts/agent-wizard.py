#!/usr/bin/env python3
"""
Agent Workspace - Setup Wizard (Textual TUI)

Interactive TUI for selecting and installing agents inside the container.
Supports --non-interactive mode for automated installations.
Supports Chinese/English based on LC_ALL environment variable.

Usage:
    python3 agent-wizard.py                         # Interactive TUI
    python3 agent-wizard.py --non-interactive codex              # Non-interactive
    python3 agent-wizard.py --china-mirror           # Use China mirrors
"""

import argparse
import asyncio
import os
import shutil
import subprocess
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# i18n — detect language from LC_ALL / LANG
# ---------------------------------------------------------------------------

def _detect_lang() -> str:
    lc = os.environ.get("LC_ALL", "") or os.environ.get("LANG", "")
    return "zh" if "zh" in lc.lower() else "en"

LANG = _detect_lang()

_TEXTS: dict[str, dict[str, str]] = {
    "en": {
        "app_title":           "Agent Workspace",
        "app_subtitle":        "Setup Wizard",
        # Screen 1
        "select_title":        "Agent Workspace - Setup Wizard",
        "select_hint":         "Select agents to install:",
        "select_keys":         "(1-8=toggle  a=all  ↑↓=move)",
        "key_next":            "Next",
        "key_cancel":          "Cancel",
        "key_toggle":          "1-8 Toggle",
        "key_select_all":      "All",
        "key_move":            "↑↓ Move",
        "no_agent_selected":   "Please select at least one agent",
        # Screen 2
        "config_title":        "Agent Configuration",
        "openfang_config":     "Openfang Configuration",
        "lbl_provider":        "LLM Provider:",
        "lbl_api_key":         "API Key:",
        "lbl_base_url":        "API Base URL (optional):",
        "lbl_model":           "Default Model:",
        "onboard_note":        "{name} will run its own setup wizard after installation.",
        "key_back":            "Back",
        "key_install":         "Install",
        # Screen 3
        "installing_title":    "Installing Agents...",
        "install_binary":      "installing...",
        "install_bin_ok":      "binary installed",
        "install_bin_fail":    "installation failed",
        "configuring":         "Configuring Openfang...",
        "configured":          "Openfang configured",
        "config_error":        "Config error",
        "onboard_hint":        "{name}: run '{cmd}' after installation to configure",
        "creating_service":    "Creating systemd service...",
        "service_ok":          "Service {name} enabled and started",
        "service_error":       "Service error",
        "install_complete":    "Installation complete! Press Enter to continue.",
        "key_done":            "Done",
        # Screen 4
        "summary_title":       "Installation Complete",
        "management":          "Management:",
        "run_to_finish":       "Run these to finish setup:",
        # non-interactive
        "ni_no_agents":        "No agents specified for non-interactive mode",
        "ni_unknown":          "Unknown agent",
        "ni_installing":       "Installing {name}...",
        "ni_install_fail":     "Failed to install {name}",
        "ni_bin_ok":           "{name} binary installed",
        "ni_svc_ok":           "{name} service started",
        "ni_cli_ok":           "{name} is ready to launch",
        "ni_all_ok":           "All agents installed successfully",
        "ni_run_hint":         "Note: Run the following to configure agents interactively:",
        # agent descriptions
        "desc_codex":          "OpenAI coding agent (native CLI)",
        "desc_claude_code":    "Anthropic coding agent (native CLI)",
        "desc_hermes":         "General autonomous agent (native CLI)",
        "desc_deepseek_harness": "DeepSeek's official plugin-based coding Agent (Web UI)",
        "desc_openclaw":       "AI assistant gateway (npm)",
        "desc_pi":             "Pi coding agent (npm)",
        "desc_openfang":       "Rust Agent OS (cargo)",
        "desc_zeroclaw":       "Ultra-light runtime (brew)",
    },
    "zh": {
        "app_title":           "Agent 工作区",
        "app_subtitle":        "安装向导",
        "select_title":        "Agent 工作区 - 安装向导",
        "select_hint":         "选择要安装的 Agent：",
        "select_keys":         "(1-8=切换  a=全选  ↑↓=移动)",
        "key_next":            "下一步",
        "key_cancel":          "取消",
        "key_toggle":          "1-8 切换",
        "key_select_all":      "全选",
        "key_move":            "↑↓ 移动",
        "no_agent_selected":   "请至少选择一个 Agent",
        "config_title":        "Agent 配置",
        "openfang_config":     "Openfang 配置",
        "lbl_provider":        "LLM 提供商：",
        "lbl_api_key":         "API 密钥：",
        "lbl_base_url":        "API 地址（可选）：",
        "lbl_model":           "默认模型：",
        "onboard_note":        "{name} 将在安装后运行自带的配置向导。",
        "key_back":            "返回",
        "key_install":         "安装",
        "installing_title":    "正在安装 Agent...",
        "install_binary":      "正在安装...",
        "install_bin_ok":      "安装完成",
        "install_bin_fail":    "安装失败",
        "configuring":         "正在配置 Openfang...",
        "configured":          "Openfang 配置完成",
        "config_error":        "配置错误",
        "onboard_hint":        "{name}：安装完成后请运行 '{cmd}' 进行配置",
        "creating_service":    "正在创建 systemd 服务...",
        "service_ok":          "服务 {name} 已启用并启动",
        "service_error":       "服务错误",
        "install_complete":    "安装完成！按 Enter 继续。",
        "key_done":            "完成",
        "summary_title":       "安装完成",
        "management":          "管理命令：",
        "run_to_finish":       "请运行以下命令完成配置：",
        "ni_no_agents":        "非交互模式未指定 Agent",
        "ni_unknown":          "未知 Agent",
        "ni_installing":       "正在安装 {name}...",
        "ni_install_fail":     "安装 {name} 失败",
        "ni_bin_ok":           "{name} 安装完成",
        "ni_svc_ok":           "{name} 服务已启动",
        "ni_cli_ok":           "{name} 已可启动",
        "ni_all_ok":           "所有 Agent 安装成功",
        "ni_run_hint":         "提示：请运行以下命令进行交互式配置：",
        "desc_codex":          "OpenAI 编程 Agent（原生 CLI）",
        "desc_claude_code":    "Anthropic 编程 Agent（原生 CLI）",
        "desc_hermes":         "通用自主 Agent（原生 CLI）",
        "desc_deepseek_harness": "DeepSeek 官方插件化编程 Agent（Web UI）",
        "desc_openclaw":       "AI 助手网关 (npm)",
        "desc_pi":             "Pi 编程 Agent (npm)",
        "desc_openfang":       "Rust Agent 操作系统 (cargo)",
        "desc_zeroclaw":       "超轻量 Agent 运行时 (brew)",
    },
}


def t(key: str, **kwargs) -> str:
    """Get translated text. Falls back to English."""
    text = _TEXTS.get(LANG, _TEXTS["en"]).get(key)
    if text is None:
        text = _TEXTS["en"].get(key, key)
    if kwargs:
        text = text.format(**kwargs)
    return text


# ---------------------------------------------------------------------------
# Agent definitions
# ---------------------------------------------------------------------------

AGENTS = {
    "codex": {
        "label": "Codex",
        "desc_key": "desc_codex",
        "mode": "cli",
        "binary": "codex",
        "install_type": "codex-native",
        "code_server_extension": "openai.chatgpt",
        "onboard": "codex",
    },
    "claude-code": {
        "label": "Claude Code",
        "desc_key": "desc_claude_code",
        "mode": "cli",
        "binary": "claude",
        "install_type": "claude-native",
        "code_server_extension": "Anthropic.claude-code",
        "onboard": "claude",
    },
    "hermes": {
        "label": "Hermes Agent",
        "desc_key": "desc_hermes",
        "mode": "cli",
        "binary": "hermes",
        "install_type": "hermes-native",
        "onboard": "hermes setup --portal",
    },
    "deepseek-harness": {
        "label": "DeepSeek Harness",
        "desc_key": "desc_deepseek_harness",
        "port": 3080,
        "mode": "service",
        "binary": "deepseek-harness",
        "install_type": "deepseek-harness-native",
        "onboard": "open /proxy/3080/ and configure a model",
        "service_cmd": "web --no-open --port 3080",
        "service_extra_env": "Environment=DSH_HOME=/config/.dsh",
    },
    "openclaw": {
        "label": "OpenClaw",
        "desc_key": "desc_openclaw",
        "port": 18789,
        "install_type": "npm",
        "mode": "service",
        "binary": "openclaw",
        "onboard": "openclaw onboard",
        "service_cmd": "gateway run",
        "service_extra_env": "Environment=NODE_OPTIONS=--max-old-space-size=2048\nEnvironment=OPENCLAW_SUPERVISOR_MODE=external",
    },
    "openfang": {
        "label": "Openfang",
        "desc_key": "desc_openfang",
        "port": 4200,
        "install_type": "cargo",
        "mode": "service",
        "binary": "openfang",
        "onboard": "openfang init",
        "service_cmd": "daemon start",
        "service_extra_env": "",
    },
    "zeroclaw": {
        "label": "Zeroclaw",
        "desc_key": "desc_zeroclaw",
        "port": 42617,
        "install_type": "brew",
        "mode": "service",
        "binary": "zeroclaw",
        "onboard": "zeroclaw onboard",
        "service_cmd": "gateway",
        "service_extra_env": "",
    },
}

AGENTS["pi"] = {
    "label": "Pi", "desc_key": "desc_pi", "mode": "cli",
    "binary": "pi", "install_type": "npm",
    "package": "@mariozechner/pi-coding-agent", "onboard": "pi",
}
AGENT_KEYS = list(AGENTS.keys())

FULL_PATH = (
    "/config/bin:/config/.local/bin:/config/node/bin"
    ":/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    ":/config/.npm-global/bin:/config/.linuxbrew/bin:/home/linuxbrew/.linuxbrew/bin"
    ":/usr/local/cargo/bin"
)

LLM_PROVIDERS = ["Anthropic", "OpenAI", "Groq", "Custom"]
LLM_ENV_KEYS = {
    "Anthropic": "ANTHROPIC_API_KEY",
    "OpenAI": "OPENAI_API_KEY",
    "Groq": "GROQ_API_KEY",
    "Custom": "LLM_API_KEY",
}
LLM_DEFAULT_MODELS = {
    "Anthropic": "claude-sonnet-4-20250514",
    "OpenAI": "gpt-4o",
    "Groq": "llama-3.3-70b-versatile",
    "Custom": "",
}

USE_CHINA_MIRROR = os.environ.get("USE_CHINA_MIRROR", "false") == "true"


# ---------------------------------------------------------------------------
# Shared install helpers (used by both TUI and non-interactive)
# ---------------------------------------------------------------------------

def get_npm_registry() -> str:
    if USE_CHINA_MIRROR:
        return "--registry=https://registry.npmmirror.com"
    return ""


def find_binary(name: str) -> str:
    """Find binary path, checking common locations."""
    path = shutil.which(name)
    if path:
        return path
    for d in [
        "/config/bin",
        "/config/.local/bin",
        "/config/node/bin",
        "/config/.npm-global/bin",
        "/usr/local/bin",
        "/usr/local/cargo/bin",
        "/config/.linuxbrew/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ]:
        p = os.path.join(d, name)
        if os.path.isfile(p):
            return p
    try:
        prefix = subprocess.check_output(
            ["npm", "config", "get", "prefix", "--global"],
            stderr=subprocess.DEVNULL, text=True,
        ).strip()
        p = os.path.join(prefix, "bin", name)
        if os.path.isfile(p):
            return p
    except Exception:
        pass
    return name


def build_install_command(agent: str) -> list[str]:
    """Return the shell command list to install an agent binary."""
    info = AGENTS[agent]
    if info["install_type"] == "codex-native":
        url = os.environ.get("CODEX_INSTALL_URL", "https://chatgpt.com/codex/install.sh")
        return ["bash", "-c", 'curl -fsSL "$1" | sh', "codex-installer", url]
    elif info["install_type"] == "claude-native":
        url = os.environ.get("CLAUDE_CODE_INSTALL_URL", "https://claude.ai/install.sh")
        return ["bash", "-c", 'curl -fsSL "$1" | bash', "claude-installer", url]
    elif info["install_type"] == "hermes-native":
        url = os.environ.get(
            "HERMES_INSTALL_URL",
            "https://hermes-agent.nousresearch.com/install.sh",
        )
        return ["bash", "-c", 'curl -fsSL "$1" | bash', "hermes-installer", url]
    elif info["install_type"] == "deepseek-harness-native":
        installer = Path(__file__).resolve().parent.parent / "addons/deepseek-harness/install.sh"
        return ["bash", str(installer)]
    elif info["install_type"] == "npm":
        package = info.get("package", agent)
        cmd = f"npm install -g {package}@latest {get_npm_registry()}"
        return ["bash", "-c", cmd]
    elif info["install_type"] == "cargo":
        return ["bash", "-c",
                "curl -fsSL https://openfang.sh/install | sh"]
    elif info["install_type"] == "brew":
        brew = shutil.which("brew") or "/config/.linuxbrew/bin/brew"
        return [brew, "install", agent]
    return ["echo", f"Unknown install type for {agent}"]


def code_server_extension_installed(agent: str) -> bool:
    """Return whether an Agent's official IDE extension is already present."""
    extension_id = AGENTS[agent].get("code_server_extension")
    if not extension_id:
        return False

    extensions_dir = Path(
        os.environ.get(
            "CODE_SERVER_EXTENSIONS_DIR",
            "/config/.local/share/code-server/extensions",
        )
    )
    prefix = f"{extension_id}-".lower()
    try:
        return any(
            path.is_dir() and path.name.lower().startswith(prefix)
            for path in extensions_dir.iterdir()
        )
    except OSError:
        return False


def build_code_server_extension_command(agent: str) -> list[str] | None:
    """Return the code-server command for a missing official IDE extension."""
    extension_id = AGENTS[agent].get("code_server_extension")
    if not extension_id or code_server_extension_installed(agent):
        return None

    configured_bin = os.environ.get(
        "CODE_SERVER_BIN", "/config/opt/code-server/bin/code-server"
    )
    code_server = (
        configured_bin
        if os.path.isfile(configured_bin)
        else shutil.which("code-server")
    )
    if not code_server:
        return None

    extensions_dir = os.environ.get(
        "CODE_SERVER_EXTENSIONS_DIR",
        "/config/.local/share/code-server/extensions",
    )
    Path(extensions_dir).mkdir(parents=True, exist_ok=True)
    return [
        shutil.which("env") or "env",
        "-u",
        "VSCODE_IPC_HOOK_CLI",
        "-u",
        "CODE_SERVER_PARENT_PID",
        "-u",
        "NODE_EXEC_PATH",
        code_server,
        "--install-extension",
        extension_id,
        "--extensions-dir",
        extensions_dir,
    ]


def create_systemd_service(agent: str) -> None:
    """Write a user-level systemd unit file for the agent.

    If the agent's onboard wizard already created a user-level service,
    skip generating a new one.
    """
    info = AGENTS[agent]
    if info.get("mode") != "service":
        raise ValueError(f"{agent} is an interactive CLI and does not need a service")

    user_dir = Path.home() / ".config" / "systemd" / "user"
    if user_dir.is_dir() and list(user_dir.glob(f"{agent}*.service")):
        return

    binary = find_binary(info.get("binary", agent))
    extra_env = info.get("service_extra_env", "")
    if extra_env:
        extra_env = f"\n{extra_env}"

    unit = f"""\
[Unit]
Description={info['label']} Agent
After=network.target

[Service]
Type=simple
ExecStart={binary} {info['service_cmd']}
Restart=always
RestartSec=5
Environment=PATH={FULL_PATH}{extra_env}

[Install]
WantedBy=default.target
"""
    user_dir.mkdir(parents=True, exist_ok=True)
    service_path = user_dir / f"{agent}.service"
    service_path.write_text(unit)


def write_openfang_config(provider: str, api_key: str,
                          api_base_url: str, model: str) -> None:
    """Write Openfang configuration files."""
    config_dir = Path.home() / ".openfang"
    config_dir.mkdir(parents=True, exist_ok=True)
    config_file = config_dir / "config.toml"

    toml_lines = ['[llm]', f'provider = "{provider.lower()}"']
    if model:
        toml_lines.append(f'model = "{model}"')
    if api_base_url:
        toml_lines.append(f'api_base_url = "{api_base_url}"')
    config_file.write_text("\n".join(toml_lines) + "\n")

    env_key = LLM_ENV_KEYS.get(provider, "LLM_API_KEY")
    env_lines = [
        "#!/bin/sh",
        f'export {env_key}="{api_key}"',
    ]
    if api_base_url:
        env_lines.append(f'export LLM_API_BASE_URL="{api_base_url}"')

    profile_path = Path("/etc/profile.d/openfang.sh")
    try:
        profile_path.write_text("\n".join(env_lines) + "\n")
        profile_path.chmod(0o644)
    except PermissionError:
        pass

    os.environ[env_key] = api_key
    if api_base_url:
        os.environ["LLM_API_BASE_URL"] = api_base_url


# ---------------------------------------------------------------------------
# Non-interactive mode
# ---------------------------------------------------------------------------

def run_non_interactive(agents: list[str], china_mirror: bool) -> None:
    """Install agents with defaults, no TUI, no onboard."""
    global USE_CHINA_MIRROR
    USE_CHINA_MIRROR = china_mirror

    if not agents:
        print(f"[ERROR] {t('ni_no_agents')}")
        sys.exit(1)

    failures = []
    installed = []
    for name in agents:
        if name not in AGENTS:
            print(f"[ERROR] {t('ni_unknown')}: {name}")
            sys.exit(1)

    for name in agents:
        info = AGENTS[name]
        print(f"[INFO]  {t('ni_installing', name=info['label'])}")

        cmd = build_install_command(name)
        result = subprocess.run(cmd, text=True)
        if result.returncode != 0:
            print(f"[ERROR] {t('ni_install_fail', name=info['label'])}")
            failures.append(name)
            continue

        print(f"[OK]    {t('ni_bin_ok', name=info['label'])}")
        installed.append(name)

        extension_id = info.get("code_server_extension")
        if extension_id:
            extension_cmd = build_code_server_extension_command(name)
            if extension_cmd:
                print(f"[INFO]  Installing code-server extension {extension_id}...")
                extension = subprocess.run(extension_cmd, text=True)
                if extension.returncode == 0:
                    print(f"[OK]    code-server extension {extension_id} installed")
                else:
                    print(
                        f"[WARNING] {info['label']} is installed, but code-server "
                        f"extension {extension_id} failed"
                    )
            elif code_server_extension_installed(name):
                print(
                    f"[OK]    code-server extension {extension_id} already installed"
                )
            else:
                print(
                    f"[INFO]  code-server unavailable; skipped optional extension {extension_id}"
                )

        if info.get("mode") == "service":
            create_systemd_service(name)
            service = subprocess.run(
                ["setsid", "systemctl", "--user", "enable", "--now", name],
                text=True,
            )
            if service.returncode != 0:
                failures.append(name)
                continue
            print(f"[OK]    {t('ni_svc_ok', name=info['label'])}")
        else:
            print(f"[OK]    {t('ni_cli_ok', name=info['label'])}")

    print()
    if not failures:
        print(f"[OK]    {t('ni_all_ok')}")
    print()
    print(t("ni_run_hint"))
    for name in installed:
        info = AGENTS[name]
        print(f"  - {info['label']}: {info['onboard']}")
    if failures:
        raise SystemExit(1)


# ---------------------------------------------------------------------------
# Textual TUI
# ---------------------------------------------------------------------------

def run_tui(agents_preselect: list[str], china_mirror: bool) -> None:
    """Launch the interactive Textual TUI wizard."""
    global USE_CHINA_MIRROR
    USE_CHINA_MIRROR = china_mirror

    try:
        from textual.app import App, ComposeResult
        from textual.binding import Binding
        from textual.containers import (
            Center,
            Horizontal,
            Vertical,
            VerticalScroll,
        )
        from textual.screen import Screen
        from textual.widgets import (
            Button,
            Checkbox,
            Footer,
            Header,
            Input,
            Label,
            RadioButton,
            RadioSet,
            RichLog,
            Rule,
            Select,
            Static,
        )
    except ImportError:
        print("[ERROR] textual not installed. Run: pip3 install textual")
        sys.exit(1)

    # ── Screen 1: Agent Selection ──────────────────────────────────────

    class AgentSelectScreen(Screen):
        BINDINGS = [
            Binding("enter", "next", t("key_next"), show=True),
            Binding("escape", "cancel", t("key_cancel"), show=True),
            Binding("1", "toggle('codex')", t("key_toggle"), show=True),
            Binding("2", "toggle('claude-code')", show=False),
            Binding("3", "toggle('hermes')", show=False),
            Binding("4", "toggle('deepseek-harness')", show=False),
            Binding("5", "toggle('openclaw')", show=False),
            Binding("6", "toggle('openfang')", show=False),
            Binding("7", "toggle('zeroclaw')", show=False),
            Binding("8", "toggle('pi')", show=False),
            Binding("a", "toggle_all", t("key_select_all"), show=True),
            Binding("up", "focus_prev", t("key_move"), show=True),
            Binding("down", "focus_next", show=False),
        ]
        CSS = """
        AgentSelectScreen {
            layout: grid;
            grid-size: 1;
            grid-rows: auto 1fr auto auto;
            align-horizontal: center;
        }
        #select-title {
            text-align: center;
            text-style: bold;
            padding: 1 0;
            background: $surface;
            width: 100%;
        }
        #select-content {
            width: 60;
            padding: 0 2;
        }
        .agent-cb {
            margin: 0 2;
            height: auto;
        }
        #select-buttons {
            height: auto;
            align: center middle;
            padding: 1 0;
        }
        #select-buttons Button {
            margin: 0 1;
        }
        """

        def compose(self) -> ComposeResult:
            yield Header()
            yield Label(t("select_title"), id="select-title")
            with VerticalScroll(id="select-content"):
                yield Label(
                    f"{t('select_hint')}  [dim]{t('select_keys')}[/dim]"
                )
                yield Static("")
                for i, (name, info) in enumerate(AGENTS.items(), 1):
                    checked = name in agents_preselect
                    yield Checkbox(
                        f"{i}) {info['label']}  — {t(info['desc_key'])}",
                        value=checked,
                        id=f"cb-{name}",
                        classes="agent-cb",
                    )
            with Horizontal(id="select-buttons"):
                yield Button(t("key_next"), variant="primary", id="btn-next")
                yield Button(
                    t("key_cancel"), variant="default", id="btn-cancel"
                )
            yield Footer()

        def on_mount(self) -> None:
            self.query_one(f"#cb-{AGENT_KEYS[0]}").focus()

        def action_focus_prev(self) -> None:
            self.focus_previous()

        def action_focus_next(self) -> None:
            self.focus_next()

        def action_toggle(self, agent: str) -> None:
            cb = self.query_one(f"#cb-{agent}", Checkbox)
            cb.value = not cb.value
            cb.focus()

        def action_toggle_all(self) -> None:
            cbs = [self.query_one(f"#cb-{n}", Checkbox) for n in AGENTS]
            all_on = all(cb.value for cb in cbs)
            for cb in cbs:
                cb.value = not all_on

        def action_next(self) -> None:
            selected = []
            for name in AGENTS:
                cb = self.query_one(f"#cb-{name}", Checkbox)
                if cb.value:
                    selected.append(name)
            if not selected:
                self.notify(t("no_agent_selected"), severity="error")
                return
            self.app.selected_agents = selected
            self.app.push_screen(ConfigScreen())

        def action_cancel(self) -> None:
            self.app.exit()

        def on_button_pressed(self, event: Button.Pressed) -> None:
            if event.button.id == "btn-cancel":
                self.action_cancel()
            elif event.button.id == "btn-next":
                self.action_next()

    # ── Screen 2: Configuration ────────────────────────────────────────

    class ConfigScreen(Screen):
        BINDINGS = [
            Binding("escape", "back", t("key_back"), show=True),
            Binding("up", "focus_prev", show=False),
            Binding("down", "focus_next", show=False),
        ]
        CSS = """
        ConfigScreen {
            layout: grid;
            grid-size: 1;
            grid-rows: auto auto 1fr auto auto;
            align-horizontal: center;
        }
        #config-title {
            text-align: center;
            text-style: bold;
            padding: 1 0;
            background: $surface;
            width: 100%;
        }
        #config-content {
            width: 65;
            padding: 0 2;
        }
        .config-section {
            margin: 1 0;
            text-style: bold;
        }
        .config-label {
            margin: 1 0 0 0;
        }
        .config-note {
            color: $text-muted;
            margin: 0 2;
        }
        #config-buttons {
            height: auto;
            align: center middle;
            padding: 1 0;
        }
        #config-buttons Button {
            margin: 0 1;
        }
        """

        def compose(self) -> ComposeResult:
            yield Header()
            selected = self.app.selected_agents
            yield Label(t("config_title"), id="config-title")
            with VerticalScroll(id="config-content"):

                if "openfang" in selected:
                    yield Label(t("openfang_config"), classes="config-section")
                    yield Label(t("lbl_provider"), classes="config-label")
                    yield Select(
                        [(p, p) for p in LLM_PROVIDERS],
                        value="Anthropic",
                        id="sel-provider",
                    )
                    yield Label(t("lbl_api_key"), classes="config-label")
                    yield Input(
                        placeholder="sk-ant-...",
                        password=True,
                        id="inp-apikey",
                    )
                    yield Label(t("lbl_base_url"), classes="config-label")
                    yield Input(
                        placeholder="https://api.anthropic.com",
                        id="inp-baseurl",
                    )
                    yield Label(t("lbl_model"), classes="config-label")
                    yield Input(
                        value="claude-sonnet-4-20250514",
                        id="inp-model",
                    )
                    yield Rule()

                onboard_agents = [
                    a
                    for a in selected
                    if a != "openfang" and AGENTS[a].get("onboard")
                ]
                if onboard_agents:
                    for a in onboard_agents:
                        info = AGENTS[a]
                        yield Static(
                            f"[dim]{t('onboard_note', name=info['label'])}[/dim]",
                            classes="config-note",
                        )
                    yield Rule()

            with Horizontal(id="config-buttons"):
                yield Button(
                    t("key_install"), variant="primary", id="btn-install"
                )
                yield Button(
                    t("key_back"), variant="default", id="btn-back"
                )
            yield Footer()

        def on_select_changed(self, event: Select.Changed) -> None:
            if event.select.id == "sel-provider":
                provider = str(event.value)
                model_input = self.query_one("#inp-model", Input)
                model_input.value = LLM_DEFAULT_MODELS.get(provider, "")

        def on_mount(self) -> None:
            try:
                self.query_one("#sel-provider").focus()
            except Exception:
                self.query_one("#btn-install").focus()

        def action_focus_prev(self) -> None:
            self.focus_previous()

        def action_focus_next(self) -> None:
            self.focus_next()

        def action_install(self) -> None:
            selected = self.app.selected_agents
            if "openfang" in selected:
                self.app.openfang_config = {
                    "provider": str(
                        self.query_one("#sel-provider", Select).value
                    ),
                    "api_key": self.query_one("#inp-apikey", Input).value,
                    "api_base_url": self.query_one("#inp-baseurl", Input).value,
                    "model": self.query_one("#inp-model", Input).value,
                }
            self.app.push_screen(InstallScreen())

        def action_back(self) -> None:
            self.app.pop_screen()

        def on_input_submitted(self, event: Input.Submitted) -> None:
            """Move to next field on Enter, or install if on last field."""
            field_order = ["inp-apikey", "inp-baseurl", "inp-model"]
            if event.input.id in field_order:
                idx = field_order.index(event.input.id)
                if idx < len(field_order) - 1:
                    self.query_one(f"#{field_order[idx + 1]}").focus()
                else:
                    self.action_install()

        def on_button_pressed(self, event: Button.Pressed) -> None:
            if event.button.id == "btn-back":
                self.action_back()
            elif event.button.id == "btn-install":
                self.action_install()

    # ── Screen 3: Install Progress ─────────────────────────────────────

    class InstallScreen(Screen):
        BINDINGS = [
            Binding("escape", "cancel_install", t("key_cancel"), show=True),
            Binding("enter", "done", t("key_done"), show=False),
        ]
        CSS = """
        InstallScreen {
            layout: grid;
            grid-size: 1;
            grid-rows: auto auto 1fr auto auto;
        }
        #install-title {
            text-align: center;
            text-style: bold;
            padding: 1 0;
            background: $surface;
        }
        #install-log {
            margin: 0 2;
            border: tall $surface;
        }
        #install-buttons {
            height: auto;
            align: center middle;
            padding: 1 0;
        }
        """

        def compose(self) -> ComposeResult:
            yield Header()
            yield Label(t("installing_title"), id="install-title")
            yield RichLog(id="install-log", highlight=True, markup=True)
            with Horizontal(id="install-buttons"):
                yield Button(
                    t("key_cancel"),
                    variant="error",
                    id="btn-cancel-install",
                )
            yield Footer()

        def on_mount(self) -> None:
            self._cancelled = False
            self._install_done = False
            self.run_worker(self.do_install(), exclusive=True)

        async def do_install(self) -> None:
            log = self.query_one("#install-log", RichLog)
            selected = self.app.selected_agents
            results = {}

            for agent_name in selected:
                if self._cancelled:
                    break

                info = AGENTS[agent_name]
                log.write(
                    f"[bold blue]▶ {info['label']}[/] "
                    f"— {t('install_binary')}"
                )

                # Step 1: Install binary
                cmd = build_install_command(agent_name)
                success = await self._run_logged(cmd, log)
                if not success:
                    log.write(
                        f"[bold red]✗ {info['label']}[/] "
                        f"— {t('install_bin_fail')}"
                    )
                    results[agent_name] = "failed"
                    continue

                log.write(
                    f"[green]✓ {info['label']}[/] — {t('install_bin_ok')}"
                )

                extension_id = info.get("code_server_extension")
                if extension_id:
                    extension_cmd = build_code_server_extension_command(agent_name)
                    if extension_cmd:
                        log.write(
                            f"[blue]  ⟳ Installing code-server extension {extension_id}...[/]"
                        )
                        extension_ok = await self._run_logged(extension_cmd, log)
                        if extension_ok:
                            log.write(
                                f"[green]  ✓ code-server extension {extension_id} installed[/]"
                            )
                        else:
                            log.write(
                                f"[yellow]  ⚠ {info['label']} is installed, but "
                                f"code-server extension {extension_id} failed[/]"
                            )
                    elif code_server_extension_installed(agent_name):
                        log.write(
                            f"[green]  ✓ code-server extension {extension_id} "
                            "already installed[/]"
                        )
                    else:
                        log.write(
                            f"[dim]  ℹ code-server unavailable; skipped optional "
                            f"extension {extension_id}[/]"
                        )

                # Step 2: Configuration
                if (
                    agent_name == "openfang"
                    and hasattr(self.app, "openfang_config")
                ):
                    cfg = self.app.openfang_config
                    log.write(f"[blue]  ⟳ {t('configuring')}[/]")
                    try:
                        await self._run_logged(
                            ["bash", "-c", "openfang init"], log
                        )
                        write_openfang_config(
                            cfg["provider"],
                            cfg["api_key"],
                            cfg["api_base_url"],
                            cfg["model"],
                        )
                        log.write(f"[green]  ✓ {t('configured')}[/]")
                    except Exception as e:
                        log.write(
                            f"[yellow]  ⚠ {t('config_error')}: {e}[/]"
                        )

                elif info.get("onboard"):
                    log.write(
                        f"[dim]  ℹ {t('onboard_hint', name=info['label'], cmd=info['onboard'])}[/]"
                    )

                # Step 3: Create a service only for daemon-style agents.
                if info.get("mode") == "service":
                    log.write(f"[blue]  ⟳ {t('creating_service')}[/]")
                    try:
                        create_systemd_service(agent_name)
                        proc = await asyncio.create_subprocess_exec(
                            "setsid",
                            "systemctl",
                            "--user",
                            "enable",
                            "--now",
                            agent_name,
                            stdout=asyncio.subprocess.PIPE,
                            stderr=asyncio.subprocess.PIPE,
                        )
                        await proc.wait()
                        if proc.returncode != 0:
                            raise RuntimeError(f"systemctl exited with {proc.returncode}")
                        log.write(
                            f"[green]  ✓ {t('service_ok', name=agent_name)}[/]"
                        )
                        results[agent_name] = "running"
                    except Exception as e:
                        log.write(
                            f"[yellow]  ⚠ {t('service_error')}: {e}[/]"
                        )
                        results[agent_name] = "installed"
                else:
                    results[agent_name] = "ready"

                log.write("")

            self.app.install_results = results

            if not self._cancelled:
                log.write(f"[bold green]{t('install_complete')}[/]")
                self._install_done = True
                btn = self.query_one("#btn-cancel-install", Button)
                btn.label = f"{t('key_done')} →"
                btn.variant = "primary"
                btn.focus()

        async def _run_logged(self, cmd: list[str], log: RichLog) -> bool:
            """Run a subprocess and stream output to the RichLog widget."""
            try:
                proc = await asyncio.create_subprocess_exec(
                    *cmd,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.STDOUT,
                )
                while True:
                    line = await proc.stdout.readline()
                    if not line:
                        break
                    text = line.decode("utf-8", errors="replace").rstrip()
                    if text:
                        log.write(f"  [dim]> {text}[/]")
                await proc.wait()
                return proc.returncode == 0
            except Exception as e:
                log.write(f"  [red]Error: {e}[/]")
                return False

        def action_cancel_install(self) -> None:
            self._cancelled = True
            self.app.exit()

        def action_done(self) -> None:
            if self._install_done:
                self.app.push_screen(SummaryScreen())

        def on_button_pressed(self, event: Button.Pressed) -> None:
            if event.button.id == "btn-cancel-install":
                if self._install_done:
                    self.action_done()
                else:
                    self.action_cancel_install()

    # ── Screen 4: Summary ──────────────────────────────────────────────

    class SummaryScreen(Screen):
        BINDINGS = [
            Binding("enter", "exit_app", t("key_done"), show=True),
            Binding("escape", "exit_app", show=False),
        ]
        CSS = """
        SummaryScreen {
            layout: grid;
            grid-size: 1;
            grid-rows: auto auto 1fr auto auto;
            align-horizontal: center;
        }
        #summary-title {
            text-align: center;
            text-style: bold;
            padding: 1 0;
            background: $surface;
            width: 100%;
        }
        #summary-content {
            width: 60;
            padding: 0 2;
        }
        .summary-agent {
            margin: 0 2;
        }
        .summary-mgmt {
            color: $text-muted;
            margin: 0 2;
        }
        #summary-buttons {
            height: auto;
            align: center middle;
            padding: 1 0;
        }
        """

        def compose(self) -> ComposeResult:
            yield Header()
            results = getattr(self.app, "install_results", {})
            yield Label(t("summary_title"), id="summary-title")
            with VerticalScroll(id="summary-content"):

                for agent_name, status in results.items():
                    info = AGENTS[agent_name]
                    if status in {"running", "ready"}:
                        icon = "[green]✓[/green]"
                    elif status == "installed":
                        icon = "[yellow]✓[/yellow]"
                    else:
                        icon = "[red]✗[/red]"
                    detail = f"port {info['port']} — {status}" if info.get("port") else status
                    yield Static(
                        f"{icon} {info['label']}  — {detail}",
                        classes="summary-agent",
                    )

                if any(AGENTS[a].get("mode") == "service" for a in results):
                    yield Rule()
                    yield Label(t("management"))
                    yield Static(
                        "[dim]  workspacectl services[/dim]",
                        classes="summary-mgmt",
                    )
                    yield Static(
                        "[dim]  workspacectl service restart <agent>[/dim]",
                        classes="summary-mgmt",
                    )

                onboard_agents = [
                    a
                    for a in results
                    if AGENTS[a].get("onboard") and results[a] != "failed"
                ]
                if onboard_agents:
                    yield Rule()
                    yield Label(t("run_to_finish"))
                    for a in onboard_agents:
                        info = AGENTS[a]
                        yield Static(
                            f"[dim]  {info['onboard']}[/dim]",
                            classes="summary-mgmt",
                        )

            with Horizontal(id="summary-buttons"):
                yield Button(
                    t("key_done"), variant="primary", id="btn-exit"
                )
            yield Footer()

        def on_mount(self) -> None:
            self.query_one("#btn-exit").focus()

        def action_exit_app(self) -> None:
            self.app.exit()

        def on_button_pressed(self, event: Button.Pressed) -> None:
            if event.button.id == "btn-exit":
                self.action_exit_app()

    # ── Main App ───────────────────────────────────────────────────────

    class AgentWizardApp(App):
        TITLE = t("app_title")
        SUB_TITLE = t("app_subtitle")
        BINDINGS = [
            Binding("ctrl+c", "quit", "Force Quit", show=False),
        ]
        CSS = """
        Screen {
            background: $surface;
        }
        """

        def __init__(self) -> None:
            super().__init__()
            self.selected_agents: list[str] = (
                list(agents_preselect) if agents_preselect else []
            )
            self.openfang_config: dict = {}
            self.install_results: dict = {}

        def on_mount(self) -> None:
            self.push_screen(AgentSelectScreen())

    app = AgentWizardApp()
    app.run()


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Agent Workspace Setup Wizard",
    )
    parser.add_argument(
        "--non-interactive",
        action="store_true",
        help="Install with defaults, skip TUI and config wizards",
    )
    parser.add_argument(
        "--china-mirror",
        action="store_true",
        help="Use China mirror sources",
    )
    parser.add_argument(
        "agents",
        nargs="*",
        choices=list(AGENTS.keys()),
        default=[],
        help=(
            "Agents to install (deepseek-harness, codex, claude-code, pi, hermes, "
            "openclaw, openfang, zeroclaw)"
        ),
    )

    args = parser.parse_args()

    if args.china_mirror:
        os.environ["USE_CHINA_MIRROR"] = "true"

    if args.non_interactive:
        run_non_interactive(args.agents, args.china_mirror)
    else:
        run_tui(args.agents, args.china_mirror)


if __name__ == "__main__":
    main()
