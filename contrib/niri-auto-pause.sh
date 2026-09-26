#!/usr/bin/env bash
#
# niri-auto-pause.sh - Automatic pause/resume for wallrs on Niri
#
# Note: Fullscreen pausing is already handled natively by wallrsd via the
# Wayland zwlr_foreign_toplevel_manager_v1 protocol without needing any script.
#
# This script specifically solves the scrollable tiling window scenario on Niri:
# It monitors active workspaces and overview state via Niri's IPC event stream.
# - If an opaque window occupies the active workspace, it calls `wallctl pause`.
# - If the workspace is empty, or contains ONLY transparent windows (e.g. terminals),
#   it calls `wallctl resume` so the wallpaper remains visible through blur/transparency.
# - If Niri overview mode is toggled open, it calls `wallctl resume` so the wallpaper
#   is displayed while zooming out across workspaces.
#
# Requirements:
#   - niri (running compositor and CLI)
#   - jq
#   - wallctl (in PATH, ~/.cargo/bin, or target/{debug,release})
#
# Usage in config.kdl:
#   spawn-at-startup "/path/to/wallrs/contrib/niri-auto-pause.sh"
#

set -euo pipefail

# Locate wallctl binary
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WALLCTL_BIN="${WALLCTL_BIN:-}"

if [ -z "${WALLCTL_BIN}" ]; then
    if command -v wallctl >/dev/null 2>&1; then
        WALLCTL_BIN="wallctl"
    elif [ -x "/usr/local/bin/wallctl" ]; then
        WALLCTL_BIN="/usr/local/bin/wallctl"
    elif [ -x "/usr/bin/wallctl" ]; then
        WALLCTL_BIN="/usr/bin/wallctl"
    elif [ -x "${HOME}/.cargo/bin/wallctl" ]; then
        WALLCTL_BIN="${HOME}/.cargo/bin/wallctl"
    elif [ -x "${SCRIPT_DIR}/../target/release/wallctl" ]; then
        WALLCTL_BIN="${SCRIPT_DIR}/../target/release/wallctl"
    elif [ -x "${SCRIPT_DIR}/../target/debug/wallctl" ]; then
        WALLCTL_BIN="${SCRIPT_DIR}/../target/debug/wallctl"
    else
        echo "Error: wallctl not found in PATH, /usr/local/bin, /usr/bin, ~/.cargo/bin, or target/{release,debug}" >&2
        exit 1
    fi
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "Error: jq is required for niri-auto-pause.sh" >&2
    exit 1
fi

if ! command -v niri >/dev/null 2>&1; then
    echo "Error: niri command not found in PATH" >&2
    exit 1
fi

# Configuration:
# Regex of window app_id / classes treated as transparent (terminals, cava, etc.)
# If a workspace contains ONLY transparent windows or is empty, the wallpaper remains active.
TRANSPARENT_CLASSES="${TRANSPARENT_CLASSES:-kitty|Alacritty|foot|wezterm|ghostty}"

# Ignore floating windows (e.g. popups, dialogs, calculators) when deciding to pause
IGNORE_FLOATING="${IGNORE_FLOATING:-1}"

cleanup() {
    trap - EXIT INT TERM
    echo "[wallrs-niri] Disconnecting from Niri IPC..." >&2
}
trap cleanup EXIT INT TERM

CURRENT_PAUSED=""

check_and_update() {
    # 1. Check if Niri overview is currently open
    local is_overview
    is_overview="$(niri msg --json overview-state 2>/dev/null | jq -r '.is_open // false' 2>/dev/null || echo 'false')"
    if [ "${is_overview}" = "true" ]; then
        if [ "${CURRENT_PAUSED}" != "false" ]; then
            CURRENT_PAUSED="false"
            echo "[wallrs-niri] Niri overview open -> Resuming wallpaper"
            "${WALLCTL_BIN}" resume >/dev/null 2>&1 || true
        fi
        return
    fi

    # 2. Query focused workspace
    local ws_json
    ws_json="$(niri msg --json workspaces 2>/dev/null || echo '[]')"
    local ws_id
    ws_id="$(echo "${ws_json}" | jq -r '([.[] | select(.is_focused == true)][0] // [.[] | select(.is_active == true)][0]).id // empty' 2>/dev/null || echo '')"

    if [ -z "${ws_id}" ]; then
        return
    fi

    # 3. Query windows on the focused workspace
    local windows_json
    windows_json="$(niri msg --json windows 2>/dev/null || echo '[]')"

    local opaque_count
    opaque_count="$(echo "${windows_json}" | jq -r \
        --argjson ws "${ws_id}" \
        --arg re "${TRANSPARENT_CLASSES}" \
        --argjson ign_float "${IGNORE_FLOATING}" '
        [ .[] | select(
            .workspace_id == $ws and
            (if $ign_float == 1 then (.is_floating | not) else true end) and
            ((.app_id // "") | test($re; "i") | not)
        ) ] | length
    ' 2>/dev/null || echo "0")"

    if [ "${opaque_count}" -gt 0 ]; then
        if [ "${CURRENT_PAUSED}" != "true" ]; then
            CURRENT_PAUSED="true"
            echo "[wallrs-niri] Opaque window(s) active on workspace ${ws_id} -> Pausing wallpaper"
            "${WALLCTL_BIN}" pause >/dev/null 2>&1 || true
        fi
    else
        if [ "${CURRENT_PAUSED}" != "false" ]; then
            CURRENT_PAUSED="false"
            echo "[wallrs-niri] Workspace ${ws_id} clear or transparent window(s) only -> Resuming wallpaper"
            "${WALLCTL_BIN}" resume >/dev/null 2>&1 || true
        fi
    fi
}

echo "[wallrs-niri] Connected to Niri IPC"
echo "[wallrs-niri] Using wallctl binary: ${WALLCTL_BIN}"
echo "[wallrs-niri] Transparent classes: ${TRANSPARENT_CLASSES}"
echo "[wallrs-niri] Monitoring workspace and overview state (Opaque windows -> Pause, Empty/Transparent/Overview -> Resume)..."

# Run initial check
check_and_update

# Listen to Niri event stream
niri msg --json event-stream 2>/dev/null | while read -r line; do
    case "${line}" in
        *WorkspacesChanged*|*WindowsChanged*|*OverviewOpenedOrClosed*|*WorkspaceActivated*|*WindowFocusChanged*|*WindowOpenedOrChanged*|*WindowClosed*)
            check_and_update
            ;;
    esac
done
