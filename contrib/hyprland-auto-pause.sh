#!/usr/bin/env bash
#
# hyprland-auto-pause.sh - Automatic pause/resume for wallrs on Hyprland
#
# Hyprland's tiling model rarely utilizes traditional "maximized" states,
# and foreign-toplevel state alone may not reflect whether tiled windows
# cover the active workspace.
#
# This script listens to Hyprland's IPC socket2 events and pauses wallrs
# when windows are active on the current workspace, resuming rendering when
# the workspace is empty (revealing the desktop).
#
# Requirements:
#   - hyprctl (bundled with Hyprland)
#   - socat or netcat (nc)
#   - jq
#   - wallctl (in PATH or ~/.cargo/bin)
#
# Usage in hyprland.conf:
#   exec-once = /path/to/contrib/hyprland-auto-pause.sh
#

set -euo pipefail

# Ensure wallctl is discoverable
if ! command -v wallctl >/dev/null 2>&1; then
    if [ -x "${HOME}/.cargo/bin/wallctl" ]; then
        PATH="${HOME}/.cargo/bin:${PATH}"
    else
        echo "Error: wallctl not found in PATH or ~/.cargo/bin" >&2
        exit 1
    fi
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "Error: jq is required for hyprland-auto-pause.sh" >&2
    exit 1
fi

SOCKET_PATH="${XDG_RUNTIME_DIR}/hypr/${HYPRLAND_INSTANCE_SIGNATURE:-}/.socket2.sock"

if [ ! -S "${SOCKET_PATH}" ]; then
    echo "Error: Hyprland socket2 not found at ${SOCKET_PATH}" >&2
    exit 1
fi

# Configuration:
# PAUSE_ON_ANY_WINDOW: 1 = pause when any window is open on workspace
#                      0 = pause only when a window is fullscreen
PAUSE_ON_ANY_WINDOW="${PAUSE_ON_ANY_WINDOW:-1}"

check_and_update() {
    local ws_json
    ws_json="$(hyprctl activeworkspace -j 2>/dev/null || echo '{}')"
    local window_count
    window_count="$(echo "${ws_json}" | jq -r '.windows // 0')"
    local has_fullscreen
    has_fullscreen="$(echo "${ws_json}" | jq -r '.hasfullscreen // false')"

    if [ "${PAUSE_ON_ANY_WINDOW}" = "1" ]; then
        if [ "${window_count}" -gt 0 ]; then
            wallctl pause >/dev/null 2>&1 || true
        else
            wallctl resume >/dev/null 2>&1 || true
        fi
    else
        if [ "${has_fullscreen}" = "true" ]; then
            wallctl pause >/dev/null 2>&1 || true
        else
            wallctl resume >/dev/null 2>&1 || true
        fi
    fi
}

# Run initial check
check_and_update

# Listen to socket2 events
if command -v socat >/dev/null 2>&1; then
    socat -u "UNIX-CONNECT:${SOCKET_PATH}" - | while read -r event; do
        case "${event}" in
            workspace>>*|activewindow>>*|openwindow>>*|closewindow>>*|fullscreen>>*|changefloatingmode>>*)
                check_and_update
                ;;
        esac
    done
elif command -v nc >/dev/null 2>&1; then
    nc -U "${SOCKET_PATH}" | while read -r event; do
        case "${event}" in
            workspace>>*|activewindow>>*|openwindow>>*|closewindow>>*|fullscreen>>*|changefloatingmode>>*)
                check_and_update
                ;;
        esac
    done
else
    echo "Error: Neither socat nor netcat (nc) was found. Please install socat." >&2
    exit 1
fi

