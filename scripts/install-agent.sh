#!/bin/bash
# ============================================================
# install-agent.sh — Bootstrap for Agent Wizard
#
# Ensures textual is installed, downloads the latest
# agent-wizard.py (3 retries), then falls back to the
# built-in copy at /usr/local/bin/agent-wizard.py.
#
# Usage (inside container):
#   install-agent.sh [--china-mirror] [--non-interactive] [agents...]
#
# Environment:
#   USE_CHINA_MIRROR=true  — use China mirror for download URL
# ============================================================

set -e

WIZARD="agent-wizard.py"
BUILTIN="/usr/local/bin/$WIZARD"
LATEST="/tmp/$WIZARD"

# Forward --china-mirror flag to env
for arg in "$@"; do
    if [ "$arg" = "--china-mirror" ]; then
        export USE_CHINA_MIRROR="true"
    fi
done

# Auto-detect China mirror from LC_ALL
if [ "$USE_CHINA_MIRROR" != "true" ]; then
    case "${LC_ALL:-}" in
        zh_CN*) export USE_CHINA_MIRROR="true" ;;
    esac
fi

# ── Ensure textual is installed ──────────────────────────────
if ! python3 -c "import textual" 2>/dev/null; then
    echo "[bootstrap] Installing textual..."
    pip3 install textual --break-system-packages -q 2>/dev/null \
        || pip3 install textual -q 2>/dev/null \
        || true
fi

# ── Pick download URL based on mirror setting ────────────────
if [ "$USE_CHINA_MIRROR" = "true" ]; then
    URL="https://raw.gitcode.com/fliaping0/agent-workspace/raw/main/scripts/$WIZARD"
else
    URL="https://raw.githubusercontent.com/fliaping/cranibox/main/scripts/$WIZARD"
fi

# ── Download latest wizard (3 retries) ───────────────────────
for i in 1 2 3; do
    echo "[bootstrap] Downloading latest $WIZARD (attempt $i/3)..."
    if curl -fsSL --connect-timeout 10 -o "$LATEST" "$URL" 2>/dev/null && [ -s "$LATEST" ]; then
        chmod +x "$LATEST"
        exec python3 "$LATEST" "$@"
    fi
    sleep 2
done

# ── Fallback to built-in version ─────────────────────────────
if [ -f "$BUILTIN" ]; then
    echo "[bootstrap] Download failed, using built-in version"
    exec python3 "$BUILTIN" "$@"
fi

echo "[bootstrap] ERROR: No agent-wizard.py found"
exit 1
