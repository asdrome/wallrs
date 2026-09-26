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

# 2.5 Helper to safely query preview path (with retry during daemon startup)
get_preview_path() {
    local attempts=15
    local img=""
    while [[ $attempts -gt 0 ]]; do
        if img="$("$WALLCTL" preview "$@" 2>/dev/null)" && [[ -n "$img" && -f "$img" ]]; then
            echo "$img"
            return 0
        fi
        attempts=$((attempts - 1))
        sleep 0.2
    done
    # Final attempt with standard error visible for diagnosis
    "$WALLCTL" preview "$@"
}

# 3. Helper for Noctalia Shell Synchronization
sync_noctalia() {
    local preview="$1"
    if command -v noctalia &>/dev/null; then
        if pgrep -x noctalia &>/dev/null; then
            # 1. Store the new wallpaper path in Noctalia's state
            noctalia msg wallpaper-set "$preview" 2>/dev/null || true

            # 2. Query user's configured scheme (e.g. m3-fruit-salad, m3-tonal-spot, m3-content)
            local scheme="m3-fruit-salad"
            local current_scheme
            current_scheme="$(noctalia msg color-scheme-get 2>/dev/null || true)"
            if [[ -n "$current_scheme" ]]; then
                local parsed
                parsed="$(echo "$current_scheme" | awk '{print $2}')"
                if [[ -n "$parsed" ]]; then
                    scheme="$parsed"
                fi
            fi

            # 3. Trigger ThemeService to recompute and transition to the new palette in memory
            # This updates the Noctalia UI (bar, launcher) and renders all configured templates (Niri, Kitty, GTK, etc.)
            noctalia msg color-scheme-set wallpaper "$scheme" 2>/dev/null || true
            return 0
        else
            # Offline fallback when Noctalia daemon is not running
            noctalia theme "$preview" --builtin-config 2>/dev/null || true
            return 0
        fi
    fi
    return 1
}

# 4. Desktop Environment Detection
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
        PREVIEW_PATH="$(get_preview_path "$@")"
        if sync_noctalia "$PREVIEW_PATH"; then
            exit 0
        elif command -v matugen &>/dev/null; then
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
    *Hyprland*|*sway*|*wlroots*|*Sway*)
        PREVIEW_PATH="$(get_preview_path "$@")"
        if pgrep -x noctalia &>/dev/null && sync_noctalia "$PREVIEW_PATH"; then
            exit 0
        fi

        ML4W_WALLPAPER=""
        if command -v ml4w-wallpaper &>/dev/null; then
            ML4W_WALLPAPER="ml4w-wallpaper"
        elif [[ -x "$HOME/.config/ml4w/scripts/ml4w-wallpaper" ]]; then
            ML4W_WALLPAPER="$HOME/.config/ml4w/scripts/ml4w-wallpaper"
        fi

        if [[ -n "$ML4W_WALLPAPER" ]]; then
            exec "$ML4W_WALLPAPER" "$PREVIEW_PATH" --skip-wallpaper
        elif command -v matugen &>/dev/null && [[ -x "$CONTRIB_DIR/hyprland-matugen.sh" ]]; then
            exec "$CONTRIB_DIR/hyprland-matugen.sh" "$@"
        elif command -v matugen &>/dev/null; then
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        fi
        ;;
    *)
        # Fallback: check if noctalia, matugen, or pywal are available regardless of compositor
        PREVIEW_PATH="$(get_preview_path "$@")"
        if pgrep -x noctalia &>/dev/null && sync_noctalia "$PREVIEW_PATH"; then
            exit 0
        elif command -v matugen &>/dev/null; then
            exec matugen image --prefer saturation "$PREVIEW_PATH"
        elif command -v wal &>/dev/null; then
            exec wal -i "$PREVIEW_PATH" -n -q
        fi
        ;;
esac
