#!/bin/sh
# Bootstrap fallback for images whose persistent native manager is not installed.
root="$AGENT_WORKSPACE_CONFIG_ROOT"
[ -n "$root" ] || root="$HOME"
[ -n "$root" ] || root=/config
if [ -x "$root/bin/workspace-services" ]; then
    exec "$root/bin/workspace-services" "$@"
fi
exec /usr/local/bin/user-systemctl-legacy.sh "$@"
