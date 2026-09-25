#!/usr/bin/env bash
# ==============================================================================
# wallrs-theme-sync.sh - Desktop-Agnostic Theme & Accent Color Synchronizer
# ==============================================================================
# Description:
#   Dispatched automatically (e.g., via wallrs-theme-sync.service or user hook)
#   whenever wallrs changes the active wallpaper. Inspects the running desktop
#   environment via $XDG_CURRENT_DESKTOP and triggers the corresponding accent
#   color or theming tool (KDE Plasma, Matugen for Hyprland, Noctalia for Niri).
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

if [[ -z "${DESKTOP:-}" || "${DESKTOP}" == "unknown" ]]; then
    if [[ -n "${NIRI_SOCKET:-}" ]]; then
        DESKTOP="niri"
    elif [[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]]; then
        DESKTOP="Hyprland"
    elif [[ -n "${SWAYSOCK:-}" ]]; then
        DESKTOP="sway"
    fi
fi

case "$DESKTOP" in
    *KDE*|*Plasma*)
        if [[ -x "$CONTRIB_DIR/kde-accent-color.sh" ]]; then
            exec "$CONTRIB_DIR/kde-accent-color.sh" "$@"
        else
            echo "wallrs-theme-sync: kde-accent-color.sh not found in $CONTRIB_DIR" >&2
            exit 1
        fi
        ;;
    *niri*|*Niri*)
        PREVIEW_PATH="$("$WALLCTL" preview "$@")"
        if command -v noctalia &>/dev/null; then
            # Sync wallpaper and reload Material You templates (Niri borders in noctalia.kdl, Kitty, GTK)
            noctalia msg wallpaper-set "$PREVIEW_PATH" 2>/dev/null || true
            noctalia theme "$PREVIEW_PATH" --builtin-config 2>/dev/null || true
            noctalia msg templates-apply 2>/dev/null || true
            exit 0
        elif command -v matugen &>/dev/null; then
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
    *Hyprland*|*sway*|*wlroots*|*Sway*)
        ML4W_WALLPAPER=""
        if command -v ml4w-wallpaper &>/dev/null; then
            ML4W_WALLPAPER="ml4w-wallpaper"
        elif [[ -x "$HOME/.config/ml4w/scripts/ml4w-wallpaper" ]]; then
            ML4W_WALLPAPER="$HOME/.config/ml4w/scripts/ml4w-wallpaper"
        fi

        if command -v noctalia &>/dev/null && pgrep -x noctalia &>/dev/null; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            noctalia msg wallpaper-set "$PREVIEW_PATH" 2>/dev/null || true
            noctalia theme "$PREVIEW_PATH" --builtin-config 2>/dev/null || true
            noctalia msg templates-apply 2>/dev/null || true
            exit 0
        elif [[ -n "$ML4W_WALLPAPER" ]]; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            exec "$ML4W_WALLPAPER" "$PREVIEW_PATH" --skip-wallpaper
        elif command -v matugen &>/dev/null && [[ -x "$CONTRIB_DIR/hyprland-matugen.sh" ]]; then
            exec "$CONTRIB_DIR/hyprland-matugen.sh" "$@"
        elif command -v matugen &>/dev/null; then
            PREVIEW_PATH="$("$WALLCTL" preview "$@")"
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        fi
        ;;
    *)
        # Fallback: check if noctalia, matugen, or pywal are available regardless of compositor
        PREVIEW_PATH="$("$WALLCTL" preview "$@")"
        if command -v noctalia &>/dev/null && pgrep -x noctalia &>/dev/null; then
            noctalia msg wallpaper-set "$PREVIEW_PATH" 2>/dev/null || true
            noctalia theme "$PREVIEW_PATH" --builtin-config 2>/dev/null || true
            noctalia msg templates-apply 2>/dev/null || true
            exit 0
        elif command -v matugen &>/dev/null; then
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
esac
