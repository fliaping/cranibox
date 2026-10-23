#!/usr/bin/env bash
# Install the Workspace server build of cranibox (the device CLI is the same program).
set -euo pipefail
source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
ensure=false
start=true
for arg in "$@"; do
  case "$arg" in
    --ensure) ensure=true ;;
    --no-start) start=false ;;
    *) echo "Usage: $0 [--ensure] [--no-start]" >&2; exit 2 ;;
  esac
done
binary="${CRANIBOX_BIN:-}"
builtin_root="${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}"
if $ensure && [[ -x "$config_root/bin/cranibox" ]]; then
  binary="$config_root/bin/cranibox"
  # A rebuilt image brings its own program. Take it unless the installed one was
  # replaced on purpose: it is still the previous image's copy, or older than this one.
  if [[ -x "$builtin_root/cranibox" ]]; then
    installed_sum=$(sha256sum "$binary" | cut -d' ' -f1)
    builtin_sum=$(sha256sum "$builtin_root/cranibox" | cut -d' ' -f1)
    recorded_sum=$(cat "$config_root/.local/state/cranibox/builtin.sha256" 2>/dev/null || true)
    installed_version=$("$binary" --version 2>/dev/null | awk '{print $2}')
    builtin_version=$("$builtin_root/cranibox" --version | awk '{print $2}')
    newest=$(printf '%s\n%s\n' "$installed_version" "$builtin_version" | sort -V | tail -n1)
    if [[ "$installed_sum" != "$builtin_sum" ]] && { [[ "$installed_sum" == "$recorded_sum" ]] ||
        [[ "$installed_version" != "$builtin_version" && "$newest" == "$builtin_version" ]]; }; then
      binary="$builtin_root/cranibox"
    fi
  fi
elif [[ -z "$binary" && -x "$builtin_root/cranibox" ]]; then
  binary="$builtin_root/cranibox"
fi
if [[ -z "$binary" ]]; then
  target_root="${CARGO_TARGET_DIR:-$source_root/target}"
  [[ "$target_root" == /* ]] || target_root="$PWD/$target_root"
  cargo build --manifest-path "$source_root/Cargo.toml" --target-dir "$target_root" --locked --release -p cranibox
  binary="$target_root/release/cranibox"
fi
"$binary" --version
export HOME="$config_root" AGENT_WORKSPACE_CONFIG_ROOT="$config_root"
if [[ ! -x "$config_root/bin/herdr" && -x "$builtin_root/herdr" ]]; then
  mkdir -p "$config_root/bin"
  install -m 755 "$builtin_root/herdr" "$config_root/bin/herdr"
fi
python3 "$(dirname "${BASH_SOURCE[0]}")/install-cranibox.py" "$config_root" "$binary" "$ensure" "$start" "$builtin_root/cranibox"
