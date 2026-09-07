#!/usr/bin/env bash
#
# hyprland-auto-pause.sh - Automatic pause/resume for wallrs on Hyprland
#
# Note: Fullscreen pausing is already handled natively by wallrsd via the
# Wayland zwlr_foreign_toplevel_manager_v1 protocol without needing any script.
#
# This script specifically solves the tiling window scenario:
# It monitors active workspaces over Hyprland's IPC socket2 event stream.
# - If an opaque window occupies the active workspace, it calls `wallctl pause`.
# - If the workspace is empty, or contains ONLY transparent windows (e.g. terminals),
#   it calls `wallctl resume` so the wallpaper remains visible through blur/transparency.
#
# Requirements:
#   - hyprctl (bundled with Hyprland)
#   - socat or netcat (nc)
#   - jq
#   - wallctl (in PATH, ~/.cargo/bin, or target/{debug,release})
#
# Usage in hyprland.conf:
#   exec-once = /path/to/wallrs/contrib/hyprland-auto-pause.sh
#

set -euo pipefail

# Locate wallctl binary
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WALLCTL_BIN="${WALLCTL_BIN:-}"

if [ -z "${WALLCTL_BIN}" ]; then
    if command -v wallctl >/dev/null 2>&1; then
        WALLCTL_BIN="wallctl"
    elif [ -x "${HOME}/.cargo/bin/wallctl" ]; then
        WALLCTL_BIN="${HOME}/.cargo/bin/wallctl"
    elif [ -x "${SCRIPT_DIR}/../target/release/wallctl" ]; then
        WALLCTL_BIN="${SCRIPT_DIR}/../target/release/wallctl"
    elif [ -x "${SCRIPT_DIR}/../target/debug/wallctl" ]; then
        WALLCTL_BIN="${SCRIPT_DIR}/../target/debug/wallctl"
    else
        echo "Error: wallctl not found in PATH, ~/.cargo/bin, or target/{release,debug}" >&2
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
    echo "Is Hyprland running?" >&2
    exit 1
fi

# Configuration:
# Regex of window classes treated as transparent (terminals, cava, etc.)
# If a workspace contains ONLY transparent windows or is empty, the wallpaper remains active.
TRANSPARENT_CLASSES="${TRANSPARENT_CLASSES:-kitty|Alacritty|foot|wezterm|ghostty}"

# Ignore floating windows (e.g. popups, dialogs, calculators) when deciding to pause
IGNORE_FLOATING="${IGNORE_FLOATING:-1}"

cleanup() {
    trap - EXIT INT TERM
    echo "[wallrs-hypr] Disconnecting from Hyprland IPC..." >&2
}
trap cleanup EXIT INT TERM

CURRENT_PAUSED=""

check_and_update() {
    local ws_json
    ws_json="$(hyprctl activeworkspace -j 2>/dev/null || echo '{}')"
    local ws_id
    ws_id="$(echo "${ws_json}" | jq -r '.id // empty')"

    if [ -z "${ws_id}" ]; then
        return
    fi

    # Query windows on the active workspace
    local clients_json
    clients_json="$(hyprctl clients -j 2>/dev/null || echo '[]')"

    # Count how many non-floating, non-hidden, opaque windows are on this workspace
    local opaque_count
    opaque_count="$(echo "${clients_json}" | jq -r \
        --argjson ws "${ws_id}" \
        --arg re "${TRANSPARENT_CLASSES}" \
        --argjson ign_float "${IGNORE_FLOATING}" '
        [ .[] | select(
            .workspace.id == $ws and
            (.hidden // false | not) and
            (if $ign_float == 1 then (.floating // false | not) else true end) and
            ((.class // "") | test($re; "i") | not)
        ) ] | length
    ' 2>/dev/null || echo "0")"

    if [ "${opaque_count}" -gt 0 ]; then
        if [ "${CURRENT_PAUSED}" != "true" ]; then
            CURRENT_PAUSED="true"
            echo "[wallrs-hypr] Opaque tiled window(s) active on workspace ${ws_id} -> Pausing wallpaper"
            "${WALLCTL_BIN}" pause >/dev/null 2>&1 || true
        fi
    else
        if [ "${CURRENT_PAUSED}" != "false" ]; then
            CURRENT_PAUSED="false"
            echo "[wallrs-hypr] Workspace ${ws_id} clear or transparent window(s) only -> Resuming wallpaper"
            "${WALLCTL_BIN}" resume >/dev/null 2>&1 || true
        fi
    fi
}

echo "[wallrs-hypr] Connected to Hyprland IPC socket2 (${SOCKET_PATH})"
echo "[wallrs-hypr] Using wallctl binary: ${WALLCTL_BIN}"
echo "[wallrs-hypr] Transparent classes: ${TRANSPARENT_CLASSES}"
echo "[wallrs-hypr] Monitoring workspace state (Opaque tiled windows -> Pause, Empty/Transparent -> Resume)..."

# Run initial check
check_and_update

# Listen to socket2 events (workspace switches, window open/close/move)
if command -v socat >/dev/null 2>&1; then
    socat -u "UNIX-CONNECT:${SOCKET_PATH}" - 2>/dev/null | while read -r event; do
        case "${event}" in
            "workspace>>"*|"focusedmon>>"*|"openwindow>>"*|"closewindow>>"*|"movewindow>>"*|"changefloatingmode>>"*)
                check_and_update
                ;;
        esac
    done
elif command -v nc >/dev/null 2>&1; then
    nc -U "${SOCKET_PATH}" 2>/dev/null | while read -r event; do
        case "${event}" in
            "workspace>>"*|"focusedmon>>"*|"openwindow>>"*|"closewindow>>"*|"movewindow>>"*|"changefloatingmode>>"*)
                check_and_update
                ;;
        esac
    done
else
    echo "Error: Neither socat nor netcat (nc) was found. Please install socat." >&2
    exit 1
fi
