#!/usr/bin/env bash
#
# kde-auto-pause.sh - Automatic pause/resume for wallrs on KDE Plasma 6 (Wayland)
#
# KDE Plasma 6 deliberately does not implement the zwlr_foreign_toplevel_manager_v1
# Wayland protocol for privacy/security reasons.
#
# This helper script monitors KWin's D-Bus interface (specifically "showingDesktopChanged"
# and active window changes) to automatically pause wallrs when windows cover the
# desktop, and resume rendering when the user reveals the desktop (e.g. Meta+D).
#
# Requirements:
#   - qdbus6 or qdbus (standard in KDE Plasma) or gdbus
#   - wallctl (in PATH or ~/.cargo/bin)
#
# Usage:
#   Add to KDE System Settings -> Autostart -> Add Login Script
#   or run in background: contrib/kde-auto-pause.sh &
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

# Detect available qdbus or gdbus binary
QDBUS_BIN=""
if command -v qdbus6 >/dev/null 2>&1; then
    QDBUS_BIN="qdbus6"
elif command -v qdbus >/dev/null 2>&1; then
    QDBUS_BIN="qdbus"
fi

check_and_update() {
    local showing_desktop="false"
    if [ -n "${QDBUS_BIN}" ]; then
        showing_desktop="$("${QDBUS_BIN}" org.kde.KWin /KWin org.kde.KWin.showingDesktop 2>/dev/null || echo "false")"
    elif command -v gdbus >/dev/null 2>&1; then
        showing_desktop="$(gdbus call --session --dest org.kde.KWin --object-path /KWin --method org.freedesktop.DBus.Properties.Get org.kde.KWin showingDesktop 2>/dev/null | grep -o 'true\|false' || echo "false")"
    fi

    if [ "${showing_desktop}" = "true" ]; then
        # Desktop is showing (Meta+D) -> resume wallpaper
        wallctl resume >/dev/null 2>&1 || true
    else
        # Normal state with windows -> pause to conserve GPU/CPU resources
        # (Customize as desired if you prefer rendering continuously)
        wallctl pause >/dev/null 2>&1 || true
    fi
}

# Run initial state check
check_and_update

# Listen to KWin D-Bus signals
if command -v gdbus >/dev/null 2>&1; then
    gdbus monitor --session --dest org.kde.KWin | while read -r line; do
        case "${line}" in
            *showingDesktopChanged*|*activeWindowChanged*)
                check_and_update
                ;;
        esac
    done
elif command -v dbus-monitor >/dev/null 2>&1; then
    dbus-monitor "type='signal',sender='org.kde.KWin',interface='org.kde.KWin'" | while read -r line; do
        case "${line}" in
            *showingDesktopChanged*|*activeWindowChanged*)
                check_and_update
                ;;
        esac
    done
else
    echo "Warning: Neither gdbus nor dbus-monitor found for signal monitoring. Falling back to polling." >&2
    while true; do
        sleep 2
        check_and_update
    done
fi

