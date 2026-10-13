#!/usr/bin/env bash
# Seed built-in services into the persistent volume before the boot oneshot starts them.
set -euo pipefail
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
builtin_root="${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}"
script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export HOME="$config_root" AGENT_WORKSPACE_CONFIG_ROOT="$config_root"
mkdir -p "$config_root/bin" "$config_root/.local/bin"

if [[ -x "$builtin_root/cranibox" || -x "$config_root/bin/cranibox" ]]; then
  bash "$script_root/install-cranibox.sh" --ensure --no-start
  exit 0
fi

# Keep binaries installed by subsequent project updates. Image binaries seed missing files.
for name in workspacectl workspace-services; do
  if [[ ! -x "$config_root/bin/$name" ]]; then
    temporary="$(mktemp "$config_root/bin/.$name.XXXXXX")"
    trap 'rm -f "$temporary"' EXIT
    install -m 755 "$builtin_root/$name" "$temporary"
    mv -f "$temporary" "$config_root/bin/$name"
  fi
done
if [[ ! -e "$config_root/.local/bin/workspacectl" ]]; then
  ln -sfn "$config_root/bin/workspacectl" "$config_root/.local/bin/workspacectl"
fi
for directory in bin .local/bin; do
  if [[ ! -e "$config_root/$directory/systemctl" ]]; then
    ln -sfn "$config_root/bin/workspace-services" "$config_root/$directory/systemctl"
  fi
done

for name in runtime web; do
  unit="$config_root/.config/systemd/user/agent-workspace-$name.service"
  existed=false
  [[ ! -e "$unit" ]] || existed=true
  if [[ "$name" == runtime ]]; then
    AGENT_WORKSPACE_RUNTIME_BIN="$builtin_root/workspace-runtime" \
      bash "$script_root/install-workspace-runtime.sh" --ensure --no-start
  else
    AGENT_WORKSPACE_WEB_BIN="$builtin_root/workspace-web" \
      bash "$script_root/install-workspace-web.sh" --ensure --no-start
  fi
  # Respect an existing service's disabled state on subsequent boots.
  if ! $existed; then
    "$config_root/bin/workspace-services" --user enable "agent-workspace-$name.service"
  fi
done
