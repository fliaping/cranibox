#!/usr/bin/env bash
set -euo pipefail

PACKAGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXTENSIONS_DIR="${CODE_SERVER_EXTENSIONS_DIR:-/config/.local/share/code-server/extensions}"
CODE_SERVER_BIN="${CODE_SERVER_BIN:-/config/opt/code-server/bin/code-server}"
NPX_BIN="${NPX_BIN:-$(command -v npx || true)}"
VSCE_PACKAGE="${VSCE_PACKAGE:-@vscode/vsce@3.9.2}"
PACKAGE_TMP="$(mktemp -d "${TMPDIR:-/tmp}/agent-workspace-extensions.XXXXXX")"

cleanup() {
    rm -rf -- "${PACKAGE_TMP}"
}
trap cleanup EXIT

if [[ ! -x "${CODE_SERVER_BIN}" ]]; then
    echo "[extensions] code-server CLI not found: ${CODE_SERVER_BIN}" >&2
    exit 1
fi

if [[ -z "${NPX_BIN}" || ! -x "${NPX_BIN}" ]]; then
    echo "[extensions] npx not found: ${NPX_BIN}" >&2
    exit 1
fi

install_extension() {
    local name="$1"
    local source="${PACKAGE_ROOT}/extensions/${name}"
    local package_name
    local publisher
    local version
    local vsix

    if [[ ! -f "${source}/package.json" ]]; then
        echo "[extensions] missing package.json: ${source}" >&2
        return 1
    fi

    package_name="$(node -e "process.stdout.write(require('${source}/package.json').name)")"
    publisher="$(node -e "process.stdout.write(require('${source}/package.json').publisher || 'agent-workspace')")"
    version="$(node -e "process.stdout.write(require('${source}/package.json').version)")"
    vsix="${PACKAGE_TMP}/${publisher}.${package_name}-${version}.vsix"

    mkdir -p "${EXTENSIONS_DIR}"

    (
        cd "${source}"
        "${NPX_BIN}" --yes "${VSCE_PACKAGE}" package \
            --no-dependencies \
            --allow-missing-repository \
            --skip-license \
            --out "${vsix}"
    )

    # Remove the legacy local publisher through the CLI so extensions.json and
    # the filesystem stay consistent.
    env -u VSCODE_IPC_HOOK_CLI -u CODE_SERVER_PARENT_PID -u NODE_EXEC_PATH \
        "${CODE_SERVER_BIN}" \
        --uninstall-extension "self-conf.${package_name}" \
        --extensions-dir "${EXTENSIONS_DIR}" >/dev/null 2>&1 || true

    # Official VSIX installation invalidates the extension manifest cache and
    # makes a subsequent Developer: Reload Window sufficient.
    env -u VSCODE_IPC_HOOK_CLI -u CODE_SERVER_PARENT_PID -u NODE_EXEC_PATH \
        "${CODE_SERVER_BIN}" \
        --install-extension "${vsix}" \
        --force \
        --extensions-dir "${EXTENSIONS_DIR}"
    echo "[extensions] installed ${publisher}.${package_name}@${version}"
}

uninstall_extension() {
    local extension_id="$1"
    env -u VSCODE_IPC_HOOK_CLI -u CODE_SERVER_PARENT_PID -u NODE_EXEC_PATH \
        "${CODE_SERVER_BIN}" \
        --uninstall-extension "${extension_id}" \
        --extensions-dir "${EXTENSIONS_DIR}" >/dev/null 2>&1 || true
}

install_marketplace_extension() {
    local extension_id="$1"
    if ! env -u VSCODE_IPC_HOOK_CLI -u CODE_SERVER_PARENT_PID -u NODE_EXEC_PATH \
        "${CODE_SERVER_BIN}" \
        --install-extension "${extension_id}" \
        --force \
        --extensions-dir "${EXTENSIONS_DIR}"; then
        return 1
    fi
    echo "[extensions] installed ${extension_id}"
}

agent_extension_installed() {
    local extension_id="$1"
    [[ -n "$(find "${EXTENSIONS_DIR}" -maxdepth 1 -mindepth 1 -type d \
        -iname "${extension_id}-*" -print -quit 2>/dev/null)" ]]
}

install_agent_extension() {
    local extension_id="$1"
    if agent_extension_installed "${extension_id}"; then
        echo "[extensions] ${extension_id} is already installed"
        return 0
    fi
    if ! install_marketplace_extension "${extension_id}"; then
        echo "[extensions] warning: failed to install optional Agent extension ${extension_id}" >&2
        return 0
    fi
}

# Control Center replaces the separate service and Caddy activity-bar entries.
# The source directories stay in the repository for compatibility and focused
# development, but normal installs expose a single operational surface.
install_extension "control-center"
install_extension "selkies-desktop"
install_marketplace_extension "MS-CEINTL.vscode-language-pack-zh-hans"
bash "${PACKAGE_ROOT}/scripts/install-workspacectl.sh"
"${PACKAGE_ROOT}/scripts/workspacectl" locale --register-language-pack

# Agent-specific IDE extensions normally install together with their CLI. This
# also repairs installations where the Agent existed before code-server.
if command -v codex >/dev/null 2>&1; then
    install_agent_extension "openai.chatgpt"
fi
if command -v claude >/dev/null 2>&1; then
    install_agent_extension "Anthropic.claude-code"
fi

uninstall_extension "agent-workspace.unified-service-manager"
uninstall_extension "agent-workspace.service-manager"
uninstall_extension "agent-workspace.caddy-proxy-manager"
uninstall_extension "self-conf.unified-service-manager"
uninstall_extension "self-conf.service-manager"
uninstall_extension "self-conf.caddy-proxy-manager"

echo "[extensions] Control Center migration complete; reload the code-server window"
