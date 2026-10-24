#!/usr/bin/with-contenv bash
# shellcheck shell=bash
# Keep XFCE panel rows and icons in step with the Wayland output scale.
#
# Pixelflux changes the compositor scale with wlr-randr, but XFCE 4.20 keeps
# the panel's row and icon sizes at their 1x values.  At fractional scaling
# this can make text and icons overflow the top panel.  This watcher only
# reads the output scale and updates existing xfconf properties; it does not
# restart the panel, desktop session, or compositor.

set -u

case "${XFCE_PANEL_SCALING:-true}" in
    false|FALSE|False|0|no|NO|off|OFF)
        echo "[xfce-panel-scale] Disabled by XFCE_PANEL_SCALING"
        exec sleep infinity
        ;;
esac

case "${PIXELFLUX_WAYLAND:-false}" in
    true|TRUE|True|1|yes|YES|on|ON) ;;
    *)
        echo "[xfce-panel-scale] Pixelflux Wayland mode is not enabled, disabled"
        exec sleep infinity
        ;;
esac

if ! command -v xfce4-panel >/dev/null 2>&1 \
    || ! command -v xfconf-query >/dev/null 2>&1 \
    || ! command -v wlr-randr >/dev/null 2>&1; then
    echo "[xfce-panel-scale] XFCE/Wayland tools not present, disabled"
    exec sleep infinity
fi

POLL_INTERVAL="${XFCE_PANEL_SCALE_INTERVAL:-5}"
MAX_SCALE="${XFCE_PANEL_MAX_SCALE:-3.0}"
PANEL1_BASE_SIZE="${XFCE_PANEL1_BASE_SIZE:-26}"
PANEL1_BASE_ICON_SIZE="${XFCE_PANEL1_BASE_ICON_SIZE:-16}"
PANEL2_BASE_SIZE="${XFCE_PANEL2_BASE_SIZE:-48}"

case "$POLL_INTERVAL" in
    ''|*[!0-9]*) POLL_INTERVAL=5 ;;
esac
[ "$POLL_INTERVAL" -ge 1 ] 2>/dev/null || POLL_INTERVAL=5

[[ "$MAX_SCALE" =~ ^[0-9]+([.][0-9]+)?$ ]] || MAX_SCALE=3.0
for variable_name in PANEL1_BASE_SIZE PANEL1_BASE_ICON_SIZE PANEL2_BASE_SIZE; do
    value="${!variable_name}"
    if ! [[ "$value" =~ ^[1-9][0-9]*$ ]]; then
        case "$variable_name" in
            PANEL1_BASE_SIZE) PANEL1_BASE_SIZE=26 ;;
            PANEL1_BASE_ICON_SIZE) PANEL1_BASE_ICON_SIZE=16 ;;
            PANEL2_BASE_SIZE) PANEL2_BASE_SIZE=48 ;;
        esac
    fi
done

SESSION_PID=""
SESSION_ENV=""
DBUS_ADDRESS=""
SESSION_DISPLAY=""
SESSION_WAYLAND_DISPLAY=""
SESSION_RUNTIME_DIR=""

# /proc/<pid>/environ of another user needs CAP_SYS_PTRACE, which Docker does
# not grant to root (test -r still succeeds), so read it as the session owner.
read_session_environ() {
    s6-setuidgid abc cat "/proc/${SESSION_PID}/environ" 2>/dev/null | tr '\0' '\n'
}

read_session_value() {
    local key="$1"
    printf '%s\n' "$SESSION_ENV" \
        | sed -n "s/^${key}=//p" \
        | head -n 1
}

load_session() {
    local waiting_logged=0
    while true; do
        SESSION_PID=$(pgrep -u abc -x xfce4-session 2>/dev/null | head -n 1)
        if [ -n "$SESSION_PID" ]; then
            SESSION_ENV=$(read_session_environ)
            DBUS_ADDRESS=$(read_session_value DBUS_SESSION_BUS_ADDRESS)
            SESSION_DISPLAY=$(read_session_value DISPLAY)
            SESSION_WAYLAND_DISPLAY=$(read_session_value WAYLAND_DISPLAY)
            SESSION_RUNTIME_DIR=$(read_session_value XDG_RUNTIME_DIR)

            if [ -n "$DBUS_ADDRESS" ] \
                && [ -n "$SESSION_WAYLAND_DISPLAY" ] \
                && [ -n "$SESSION_RUNTIME_DIR" ]; then
                echo "[xfce-panel-scale] Connected to XFCE session ${SESSION_PID} (${SESSION_WAYLAND_DISPLAY})"
                return 0
            fi
        fi
        if [ "$waiting_logged" -eq 0 ]; then
            echo "[xfce-panel-scale] Waiting for the XFCE session environment"
            waiting_logged=1
        fi
        sleep 2
    done
}

run_wayland() {
    s6-setuidgid abc env \
        XDG_RUNTIME_DIR="$SESSION_RUNTIME_DIR" \
        WAYLAND_DISPLAY="$SESSION_WAYLAND_DISPLAY" \
        "$@"
}

run_xfconf() {
    s6-setuidgid abc env \
        DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDRESS" \
        DISPLAY="$SESSION_DISPLAY" \
        XDG_RUNTIME_DIR="$SESSION_RUNTIME_DIR" \
        WAYLAND_DISPLAY="$SESSION_WAYLAND_DISPLAY" \
        "$@"
}

read_output_scale() {
    run_wayland wlr-randr 2>/dev/null \
        | awk '/^[[:space:]]*Scale:/ { printf "%.3f\n", $2; exit }'
}

scaled_value() {
    local base="$1"
    local scale="$2"
    awk -v base="$base" -v scale="$scale" -v max_scale="$MAX_SCALE" 'BEGIN {
        if (scale < 1.0) scale = 1.0
        if (scale > max_scale) scale = max_scale
        printf "%d\n", int(base * scale + 0.5)
    }'
}

set_existing_uint() {
    local property="$1"
    local value="$2"
    local current

    current=$(run_xfconf xfconf-query -c xfce4-panel -p "$property" 2>/dev/null) || return 0
    [ "$current" = "$value" ] && return 0
    run_xfconf xfconf-query -c xfce4-panel -p "$property" -s "$value" >/dev/null 2>&1
}

apply_panel_scale() {
    local scale="$1"
    local panel1_size panel1_icon_size panel2_size
    local update_failed=0

    panel1_size=$(scaled_value "$PANEL1_BASE_SIZE" "$scale")
    panel1_icon_size=$(scaled_value "$PANEL1_BASE_ICON_SIZE" "$scale")
    panel2_size=$(scaled_value "$PANEL2_BASE_SIZE" "$scale")

    set_existing_uint /panels/panel-1/size "$panel1_size" || update_failed=1
    set_existing_uint /panels/panel-1/icon-size "$panel1_icon_size" || update_failed=1
    set_existing_uint /panels/panel-2/size "$panel2_size" || update_failed=1

    echo "[xfce-panel-scale] Scale ${scale}: panel-1=${panel1_size}, icons=${panel1_icon_size}, panel-2=${panel2_size}"
    return "$update_failed"
}

load_session

last_scale=""
while true; do
    if [ ! -d "/proc/${SESSION_PID}" ]; then
        load_session
        last_scale=""
    fi

    scale=$(read_output_scale)
    if [[ "$scale" =~ ^[0-9]+([.][0-9]+)?$ ]] && [ "$scale" != "$last_scale" ]; then
        if pgrep -u abc -x xfce4-panel >/dev/null 2>&1; then
            if apply_panel_scale "$scale"; then
                last_scale="$scale"
            fi
        fi
    fi

    sleep "$POLL_INTERVAL"
done
