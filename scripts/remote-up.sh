#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${AGENT_WORKSPACE_ENV_FILE:-${ROOT}/.env.remote}"
COMPOSE_FILE="${ROOT}/docker-compose.remote.yml"

if [[ ! -f "$ENV_FILE" ]]; then
  if command -v openssl >/dev/null 2>&1; then
    password="$(openssl rand -base64 24 | tr -d '\n')"
  else
    password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')"
  fi

  agent="${AGENT_WORKSPACE_AGENT:-}"
  if [[ -z "$agent" && -t 0 ]]; then
    echo "Choose one ready-to-use Agent:"
    echo "  1) Codex (recommended)"
    echo "  2) Claude Code"
    echo "  3) Hermes Agent"
    echo "  4) DeepSeek Harness"
    echo "  5) OpenClaw"
    echo "  6) None"
    read -r -p "Agent [1]: " choice
    case "${choice:-1}" in
      1) agent=codex ;;
      2) agent=claude-code ;;
      3) agent=hermes ;;
      4) agent=deepseek-harness ;;
      5) agent=openclaw ;;
      6) agent=none ;;
      *) echo "Invalid Agent choice" >&2; exit 1 ;;
    esac
  fi
  agent="${agent:-codex}"
  case "$agent" in
    codex|claude-code|hermes|deepseek-harness|openclaw|none) ;;
    *) echo "Unsupported AGENT_WORKSPACE_AGENT: $agent" >&2; exit 1 ;;
  esac

  umask 077
  {
    echo 'AGENT_WORKSPACE_IMAGE=xuping/agent-workspace:ubuntu-xfce'
    echo 'AGENT_WORKSPACE_DATA=./webtop-data'
    echo 'AGENT_WORKSPACE_USER=agent'
    printf 'AGENT_WORKSPACE_PASSWORD=%s\n' "$password"
    printf 'AGENT_WORKSPACE_AGENT=%s\n' "$agent"
    echo 'DESKTOP_PORT=3001'
    echo 'CODE_SERVER_PORT=8443'
    echo 'TZ=Asia/Taipei'
  } >"$ENV_FILE"
  echo "Created $ENV_FILE"
  echo "User: agent"
  echo "Password: $password"
  echo "Agent: $agent"
else
  echo "Using existing $ENV_FILE"
fi

docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" up -d

desktop_port="$(sed -n 's/^DESKTOP_PORT=//p' "$ENV_FILE" | tail -n 1)"
code_port="$(sed -n 's/^CODE_SERVER_PORT=//p' "$ENV_FILE" | tail -n 1)"
selected_agent="$(sed -n 's/^AGENT_WORKSPACE_AGENT=//p' "$ENV_FILE" | tail -n 1)"
echo "On this Docker host:"
echo "  Desktop: https://localhost:${desktop_port:-3001}"
echo "  code-server: https://localhost:${code_port:-8443}"
echo "From another machine, use the server hostname/IP when those ports are reachable."
echo "If inbound ports are closed, create an SSH tunnel from the client:"
echo "ssh -N -L ${desktop_port:-3001}:127.0.0.1:${desktop_port:-3001} -L ${code_port:-8443}:127.0.0.1:${code_port:-8443} user@server"
echo "First boot installs the remote foundation; follow progress with:"
echo "docker logs -f agent-workspace"
echo "Then open code-server; Agent Workspace Control Center starts the first-use guide."
case "${selected_agent:-codex}" in
  codex) echo "Then open a terminal in /config/Workspace and run: codex" ;;
  claude-code) echo "Then open a terminal in /config/Workspace and run: claude" ;;
  hermes) echo "Then open a terminal in /config/Workspace and run: hermes setup --portal" ;;
  deepseek-harness) echo "Then open the authenticated code-server path: /proxy/3080/" ;;
  openclaw) echo "Then open the authenticated code-server path: /proxy/18789/" ;;
esac
