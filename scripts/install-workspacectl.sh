#!/usr/bin/env bash
set -euo pipefail

source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
config_root="${AGENT_WORKSPACE_CONFIG_ROOT:-/config}"
if [[ -e "$config_root/.config/agent-workspace/unified-server" || -x "${AGENT_WORKSPACE_BUILTIN_DIR:-/usr/local/lib/agent-workspace}/cranibox" ]]; then
  exec bash "$(dirname "${BASH_SOURCE[0]}")/install-cranibox.sh" --ensure --no-start
fi
target_root="${CARGO_TARGET_DIR:-$source_root/target}"
if [[ "$target_root" != /* ]]; then target_root="$PWD/$target_root"; fi
binary="${AGENT_WORKSPACE_CTL_BIN:-}"
stamp="$config_root/.local/state/agent-workspace/workspacectl-build.sha256"

if [[ -z "$binary" && -x /usr/local/lib/agent-workspace/workspacectl ]]; then
  binary=/usr/local/lib/agent-workspace/workspacectl
fi
if [[ -z "$binary" ]]; then
  # Avoid rebuilding on every bootstrap/capability install.
  fingerprint="$(
    { sha256sum "$source_root/Cargo.toml" "$source_root/Cargo.lock" "$source_root/crates/workspacectl/Cargo.toml";
      find "$source_root/crates/workspacectl/src" -type f -name '*.rs' -print0 | sort -z | xargs -0 sha256sum;
    } | sha256sum | cut -d' ' -f1
  )"
  if [[ -x "$config_root/bin/workspacectl" && ! -L "$config_root/bin/workspacectl" && -r "$stamp" && "$(cat "$stamp")" == "$fingerprint" ]]; then
    exit 0
  fi
  command -v cargo >/dev/null || { echo "Rust/Cargo is required to build workspacectl; alternatively set AGENT_WORKSPACE_CTL_BIN to a prebuilt binary." >&2; exit 1; }
  cargo build --manifest-path "$source_root/Cargo.toml" --target-dir "$target_root" -p agent-workspacectl --release --locked
  binary="$target_root/release/workspacectl"
else
  [[ -x "$binary" ]] || { echo "Prebuilt workspacectl is not executable: $binary" >&2; exit 1; }
  fingerprint="prebuilt:$(sha256sum "$binary" | cut -d' ' -f1)"
fi
"$binary" --version
mkdir -p "$config_root/bin" "$config_root/.local/bin" "$(dirname "$stamp")"
ctl_tmp="$(mktemp "$config_root/bin/.workspacectl.XXXXXX")"
stamp_tmp="$(mktemp "$(dirname "$stamp")/.workspacectl-build.XXXXXX")"
trap 'rm -f "$ctl_tmp" "$stamp_tmp"' EXIT
install -m 755 "$binary" "$ctl_tmp"
mv -f "$ctl_tmp" "$config_root/bin/workspacectl"
ln -sfn "$config_root/bin/workspacectl" "$config_root/.local/bin/workspacectl"
printf '%s\n' "$fingerprint" > "$stamp_tmp"
mv -f "$stamp_tmp" "$stamp"
echo "Rust workspacectl installed: $config_root/bin/workspacectl"
