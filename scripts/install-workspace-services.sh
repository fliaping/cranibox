#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -s)" == Linux ]] || { echo 'workspace-services requires Linux and s6' >&2; exit 1; }
migrate=false
if [[ "${1:-}" == --migrate ]]; then migrate=true; shift; fi
[[ $# == 0 ]] || { echo 'Usage: install-workspace-services.sh [--migrate]' >&2; exit 1; }
source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
if [[ -e "$config_root/.config/agent-workspace/unified-server" || -x "${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}/cranibox" ]]; then
  bash "$(dirname "${BASH_SOURCE[0]}")/install-cranibox.sh" --ensure --no-start
  if $migrate; then
    mkdir -p "$config_root/opt/agent-workspace-services"
    install -m 755 "$(dirname "${BASH_SOURCE[0]}")/user-systemctl-legacy.sh" "$config_root/opt/agent-workspace-services/user-systemctl-legacy.sh"
    HOME="$config_root" "$config_root/bin/workspace-services" --user migrate --apply --rollback-manager="$config_root/opt/agent-workspace-services/user-systemctl-legacy.sh"
  fi
  exit 0
fi
binary="${AGENT_WORKSPACE_SERVICES_BIN:-}"
target_root="${CARGO_TARGET_DIR:-$source_root/target}"
command -v s6-svscan >/dev/null || { echo 's6 is required' >&2; exit 1; }
if [[ -z "$binary" ]]; then
  cargo build --manifest-path "$source_root/Cargo.toml" --target-dir "$target_root" --locked --release -p agent-workspace-services
  binary="$target_root/release/workspace-services"
fi
"$binary" --version
if $migrate; then
  HOME="$config_root" "$binary" --user migration-plan
fi
mkdir -p "$config_root/bin" "$config_root/.local/bin" "$config_root/opt/agent-workspace-services"
install -m 755 "$(dirname "${BASH_SOURCE[0]}")/user-systemctl-legacy.sh" "$config_root/opt/agent-workspace-services/user-systemctl-legacy.sh"
temporary="$(mktemp "$config_root/bin/.workspace-services.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
install -m 755 "$binary" "$temporary"
mv -f "$temporary" "$config_root/bin/workspace-services"
ln -sfn "$config_root/bin/workspace-services" "$config_root/bin/systemctl"
ln -sfn "$config_root/bin/workspace-services" "$config_root/.local/bin/systemctl"
if $migrate; then
  HOME="$config_root" "$config_root/bin/workspace-services" --user migrate --apply \
    --rollback-manager="$config_root/opt/agent-workspace-services/user-systemctl-legacy.sh"
else
  echo "Rust user service manager installed; use --migrate to preflight and migrate existing services."
fi
