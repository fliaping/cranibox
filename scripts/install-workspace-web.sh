#!/usr/bin/env bash
set -euo pipefail
source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
if [[ -e "$config_root/.config/agent-workspace/unified-server" || -x "${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}/cranibox" ]]; then
  exec bash "$(dirname "${BASH_SOURCE[0]}")/install-cranibox.sh" "$@"
fi
start_service=true
ensure=false
for arg in "$@"; do
  case "$arg" in
    --ensure) ensure=true ;;
    --no-start) start_service=false ;;
    *) echo "Usage: $0 [--ensure] [--no-start]" >&2; exit 2 ;;
  esac
done
binary="${AGENT_WORKSPACE_WEB_BIN:-}"
if $ensure && [[ -x "$config_root/opt/agent-workspace-web/workspace-web" ]]; then
  binary="$config_root/opt/agent-workspace-web/workspace-web"
fi
if [[ -z "$binary" && -x /usr/local/lib/agent-workspace/workspace-web ]]; then
  binary=/usr/local/lib/agent-workspace/workspace-web
fi
target_root="${CARGO_TARGET_DIR:-$source_root/target}"
if [[ -z "$binary" ]]; then
  cargo build --manifest-path "$source_root/Cargo.toml" --target-dir "$target_root" --locked --release -p agent-workspace-web
  binary="$target_root/release/workspace-web"
fi
"$binary" --version
python3 - "$config_root" <<'PY'
import sys
if any(c in sys.argv[1] for c in '\n\r"\\%'):
    raise SystemExit('Unsupported config root for service unit')
PY
runtime="$config_root/opt/agent-workspace-web"
units="$config_root/.config/systemd/user"
mkdir -p "$runtime" "$units"
temporary=""
unit_tmp=""
trap '[[ -z "$temporary" ]] || rm -f "$temporary"; [[ -z "$unit_tmp" ]] || rm -f "$unit_tmp"' EXIT
if ! $ensure || [[ ! -x "$runtime/workspace-web" ]]; then
  temporary="$(mktemp "$runtime/.workspace-web.XXXXXX")"
  install -m 755 "$binary" "$temporary"
  mv -f "$temporary" "$runtime/workspace-web"
fi
if ! $ensure || [[ ! -e "$units/agent-workspace-web.service" ]]; then
  unit_tmp="$(mktemp "$units/.agent-workspace-web.XXXXXX")"
  cat > "$unit_tmp" <<EOF
[Unit]
Description=Agent Workspace Web Control Center (Rust)

[Service]
Type=simple
Environment="HOME=$config_root"
Environment="AGENT_WORKSPACE_CONFIG_ROOT=$config_root"
Environment="PATH=$config_root/bin:$config_root/.local/bin:$config_root/.npm-global/bin:$config_root/node/bin:$config_root/.linuxbrew/bin:/usr/local/bin:/usr/bin:/bin"
ExecStart="$runtime/workspace-web" --config-root "$config_root"
Restart=on-failure
RestartSec=3
TimeoutStopSec=10

[Install]
WantedBy=default.target
EOF
  chmod 644 "$unit_tmp"
  mv -f "$unit_tmp" "$units/agent-workspace-web.service"
fi
if $start_service; then
  export HOME="$config_root"
  manager=systemctl
  [[ ! -x "$config_root/bin/systemctl" ]] || manager="$config_root/bin/systemctl"
  "$manager" --user daemon-reload
  "$manager" --user enable agent-workspace-web.service
  "$manager" --user restart agent-workspace-web.service
fi
echo "Rust browser control center installed: $runtime/workspace-web"
