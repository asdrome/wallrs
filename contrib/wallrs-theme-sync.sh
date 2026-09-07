#!/usr/bin/env bash
# ==============================================================================
# wallrs-theme-sync.sh - Desktop-Agnostic Theme & Accent Color Synchronizer
# ==============================================================================
# Description:
#   Dispatched automatically (e.g., via wallrs-theme-sync.service or user hook)
#   whenever wallrs changes the active wallpaper. Inspects the running desktop
#   environment via $XDG_CURRENT_DESKTOP and triggers the corresponding accent
#   color or theming tool (KDE Plasma, Matugen for Hyprland).
# ==============================================================================

set -euo pipefail

# Locate the contrib scripts directory (adjacent to this script or in /usr/share/wallrs/contrib)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRIB_DIR="$SCRIPT_DIR"
if [[ ! -f "$CONTRIB_DIR/kde-accent-color.sh" ]]; then
    if [[ -d "/usr/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/share/wallrs/contrib"
    elif [[ -d "/usr/local/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/local/share/wallrs/contrib"
    fi
fi

# 2. Desktop Environment Detection
DESKTOP="${XDG_CURRENT_DESKTOP:-${DESKTOP_SESSION:-}}"

case "$DESKTOP" in
    *KDE*|*Plasma*)
        if [[ -x "$CONTRIB_DIR/kde-accent-color.sh" ]]; then
            exec "$CONTRIB_DIR/kde-accent-color.sh" "$@"
        else
            echo "wallrs-theme-sync: kde-accent-color.sh not found in $CONTRIB_DIR" >&2
        fi
        ;;
    *Hyprland*|*sway*|*wlroots*|*Sway*)
        if command -v matugen &>/dev/null && [[ -x "$CONTRIB_DIR/hyprland-matugen.sh" ]]; then
            exec "$CONTRIB_DIR/hyprland-matugen.sh" "$@"
        elif command -v matugen &>/dev/null; then
            PREVIEW_PATH="$(wallctl preview "$@")"
            exec matugen image "$PREVIEW_PATH"
        fi
        ;;
    *)
        # Fallback: check if matugen or pywal are available regardless of compositor
        if command -v matugen &>/dev/null; then
            PREVIEW_PATH="$(wallctl preview "$@")"
            exec matugen image "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            PREVIEW_PATH="$(wallctl preview "$@")"
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
esac

