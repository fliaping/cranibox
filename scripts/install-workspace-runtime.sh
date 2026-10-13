#!/usr/bin/env bash
set -euo pipefail

source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
if [[ -e "$config_root/.config/agent-workspace/unified-server" || -x "${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}/cranibox" ]]; then
  exec bash "$(dirname "${BASH_SOURCE[0]}")/install-cranibox.sh" "$@"
fi
start_service=true
ensure=false
binary="${AGENT_WORKSPACE_RUNTIME_BIN:-}"
web_upstream="http://127.0.0.1:9080"
for arg in "$@"; do
  case "$arg" in
    --ensure) ensure=true ;;
    --no-start) start_service=false ;;
    --standalone) web_upstream="" ;;
    *) echo "Usage: $0 [--ensure] [--no-start] [--standalone]" >&2; exit 2 ;;
  esac
done
if [[ "$config_root" == *$'\n'* || "$config_root" == *'"'* || "$config_root" == *'\'* || "$config_root" == *'%'* ]]; then
  echo "Unsupported characters in config root for a systemd unit" >&2
  exit 2
fi
target_root="${CARGO_TARGET_DIR:-$source_root/target}"
if [[ "$target_root" != /* ]]; then target_root="$PWD/$target_root"; fi
if $ensure && [[ -x "$config_root/bin/workspace-runtime" ]]; then
  binary="$config_root/bin/workspace-runtime"
fi
if [[ -z "$binary" && -x /usr/local/lib/agent-workspace/workspace-runtime ]]; then
  binary=/usr/local/lib/agent-workspace/workspace-runtime
fi
if [[ -z "$binary" ]]; then
  command -v cargo >/dev/null || { echo "Set AGENT_WORKSPACE_RUNTIME_BIN to a prebuilt runtime, or install Rust/Cargo" >&2; exit 1; }
  cargo build --manifest-path "$source_root/Cargo.toml" --target-dir "$target_root" -p agent-workspace-runtime --release --locked
  binary="$target_root/release/workspace-runtime"
fi
[[ -x "$binary" ]] || { echo "Runtime binary is not executable: $binary" >&2; exit 1; }
mkdir -p "$config_root/bin" "$config_root/.config/systemd/user"
runtime_tmp=""
unit_tmp=""
trap '[[ -z "$runtime_tmp" ]] || rm -f "$runtime_tmp"; [[ -z "$unit_tmp" ]] || rm -f "$unit_tmp"' EXIT
if ! $ensure || [[ ! -x "$config_root/bin/workspace-runtime" ]]; then
  runtime_tmp="$(mktemp "$config_root/bin/.workspace-runtime.XXXXXX")"
  install -m 755 "$binary" "$runtime_tmp"
  mv -f "$runtime_tmp" "$config_root/bin/workspace-runtime"
fi
credentials="$config_root/.config/agent-workspace/runtime.json"
if [[ ! -f "$credentials" ]]; then
  "$config_root/bin/workspace-runtime" init --credentials "$credentials"
fi
chmod 600 "$credentials"
upstream_args=""
if [[ -n "$web_upstream" ]]; then upstream_args=" --web-upstream $web_upstream"; fi
unit="$config_root/.config/systemd/user/agent-workspace-runtime.service"
if ! $ensure || [[ ! -e "$unit" ]]; then
  unit_tmp="$(mktemp "$config_root/.config/systemd/user/.agent-workspace-runtime.XXXXXX")"
  cat > "$unit_tmp" <<EOF
[Unit]
Description=Agent Workspace Rust HTTP/WebSocket Gateway

[Service]
Type=simple
Environment="HOME=$config_root"
Environment="AGENT_WORKSPACE_CONFIG_ROOT=$config_root"
ExecStart="$config_root/bin/workspace-runtime" serve --credentials "$credentials" --listen 127.0.0.1:9090$upstream_args
Restart=on-failure
RestartSec=3
KillSignal=SIGTERM
TimeoutStopSec=10

[Install]
WantedBy=default.target
EOF
  chmod 644 "$unit_tmp"
  mv -f "$unit_tmp" "$unit"
fi
if $start_service; then
  manager=systemctl
  [[ ! -x "$config_root/bin/workspace-services" ]] || manager="$config_root/bin/workspace-services"
  HOME="$config_root" "$manager" --user daemon-reload
  HOME="$config_root" "$manager" --user enable agent-workspace-runtime.service
  HOME="$config_root" "$manager" --user restart agent-workspace-runtime.service
fi
echo "Rust gateway installed: $config_root/bin/workspace-runtime"
echo "Gateway listens on 127.0.0.1:9090; configure TLS proxy for remote node access."
