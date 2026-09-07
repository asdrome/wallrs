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

# 1. Locate wallctl binary
WALLCTL="${WALLCTL_BIN:-}"
if [[ -z "$WALLCTL" ]]; then
    if command -v wallctl &>/dev/null; then
        WALLCTL="wallctl"
    elif [[ -x "/usr/local/bin/wallctl" ]]; then
        WALLCTL="/usr/local/bin/wallctl"
    elif [[ -x "/usr/bin/wallctl" ]]; then
        WALLCTL="/usr/bin/wallctl"
    elif [[ -x "$HOME/.cargo/bin/wallctl" ]]; then
        WALLCTL="$HOME/.cargo/bin/wallctl"
    else
        WALLCTL="wallctl"
    fi
fi

# 2. Locate the contrib scripts directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRIB_DIR="$SCRIPT_DIR"
if [[ ! -f "$CONTRIB_DIR/kde-accent-color.sh" ]]; then
    if [[ -d "/usr/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/share/wallrs/contrib"
    elif [[ -d "/usr/local/share/wallrs/contrib" ]]; then
        CONTRIB_DIR="/usr/local/share/wallrs/contrib"
    fi
fi

# 3. Desktop Environment Detection
DESKTOP="${XDG_CURRENT_DESKTOP:-${DESKTOP_SESSION:-}}"

case "$DESKTOP" in
    *KDE*|*Plasma*)
        if [[ -x "$CONTRIB_DIR/kde-accent-color.sh" ]]; then
            exec "$CONTRIB_DIR/kde-accent-color.sh" "$@"
        else
            echo "wallrs-theme-sync: kde-accent-color.sh not found in $CONTRIB_DIR" >&2
            exit 1
        fi
        ;;
    *Hyprland*|*sway*|*wlroots*|*Sway*)
        if command -v matugen &>/dev/null && [[ -x "$CONTRIB_DIR/hyprland-matugen.sh" ]]; then
            exec "$CONTRIB_DIR/hyprland-matugen.sh" "$@"
        elif command -v matugen &>/dev/null; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            exec matugen image "$PREVIEW_PATH"
        fi
        ;;
    *)
        # Fallback: check if matugen or pywal are available regardless of compositor
        if command -v matugen &>/dev/null; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            exec matugen image "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
esac
