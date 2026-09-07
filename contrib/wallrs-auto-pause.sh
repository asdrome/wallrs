#!/usr/bin/env bash
# ==============================================================================
# wallrs-auto-pause.sh - Desktop-Agnostic Auto-Pause Service Dispatcher
# ==============================================================================
# Description:
#   Dispatched automatically by wallrs-auto-pause.service to provide window-state
#   driven pausing and resuming across different Wayland desktop environments.
#
# Supported Compositors:
#   - KDE Plasma 6: Dispatches to kde-auto-pause.sh (via KWin 6 D-Bus scripting).
#   - Hyprland:     Dispatches to hyprland-auto-pause.sh (via socket2 IPC).
#   - Sway/wlroots: Fullscreen pause is already handled natively by wallrsd via
#                   zwlr_foreign_toplevel_manager_v1.
# ==============================================================================

set -euo pipefail

# 1. Locate contrib directory (adjacent or in /usr/share or /usr/local/share)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRIB_DIR="$SCRIPT_DIR"

if [[ ! -f "$CONTRIB_DIR/kde-auto-pause.sh" ]]; then
    if [[ -d "/usr/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/share/wallrs/contrib"
    elif [[ -d "/usr/local/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/local/share/wallrs/contrib"
    fi
fi

# 2. Detect Desktop Environment
DESKTOP="${XDG_CURRENT_DESKTOP:-${DESKTOP_SESSION:-}}"

case "$DESKTOP" in
    *KDE*|*Plasma*)
        if [[ -x "$CONTRIB_DIR/kde-auto-pause.sh" ]]; then
            echo "[wallrs-auto-pause] KDE Plasma detected; launching KWin auto-pause driver..."
            exec "$CONTRIB_DIR/kde-auto-pause.sh" "$@"
        else
            echo "wallrs-auto-pause: kde-auto-pause.sh not found in $CONTRIB_DIR" >&2
            exit 1
        fi
        ;;
    *Hyprland*)
        if [[ -x "$CONTRIB_DIR/hyprland-auto-pause.sh" ]]; then
            echo "[wallrs-auto-pause] Hyprland detected; launching Hyprland IPC auto-pause driver..."
            exec "$CONTRIB_DIR/hyprland-auto-pause.sh" "$@"
        else
            echo "wallrs-auto-pause: hyprland-auto-pause.sh not found in $CONTRIB_DIR" >&2
            exit 1
        fi
        ;;
    *sway*|*Sway*|*wlroots*|*labwc*|*river*)
        echo "[wallrs-auto-pause] $DESKTOP (wlroots) detected."
        echo "[wallrs-auto-pause] Fullscreen auto-pause is natively handled by wallrsd via zwlr_foreign_toplevel_manager_v1."
        exec sleep infinity
        ;;
    *)
        echo "[wallrs-auto-pause] Desktop environment: ${DESKTOP:-unknown}."
        echo "[wallrs-auto-pause] If your compositor supports zwlr_foreign_toplevel_manager_v1, wallrsd handles fullscreen pause natively."
        exit 0
        ;;
esac
