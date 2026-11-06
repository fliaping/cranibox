#!/usr/bin/env bash
# The Agent Workspace container installer moved to github.com/fliaping/agent-workspace.
# This stub keeps the old one-liner working:
#   curl -fsSL https://raw.githubusercontent.com/fliaping/cranibox/main/install.sh | sudo bash
set -euo pipefail
url="https://raw.githubusercontent.com/fliaping/agent-workspace/main/install.sh"
echo "Agent Workspace moved to https://github.com/fliaping/agent-workspace; running its installer." >&2
script="$(mktemp)"
trap 'rm -f "$script"' EXIT
curl -fsSL "$url" -o "$script"
bash "$script" "$@"
