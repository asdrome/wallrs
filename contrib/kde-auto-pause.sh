#!/usr/bin/env bash
#
# kde-auto-pause.sh - Automatic pause/resume for wallrs on KDE Plasma 6 (Wayland)
#
# KDE Plasma 6 deliberately does not implement the zwlr_foreign_toplevel_manager_v1
# Wayland protocol for privacy/security reasons.
#
# This helper script injects a lightweight KWin 6 script via KWin's D-Bus Scripting
# interface to monitor window states (fullscreen, maximized, minimized, and Show Desktop).
# When a window is maximized or fullscreen on the active desktop, it calls wallctl pause;
# when windows are floating, minimized, or the desktop is revealed (Meta+D), it calls wallctl resume.
#
# Requirements:
#   - qdbus6 or qdbus (bundled with KDE Plasma)
#   - dbus-monitor
#   - wallctl (in PATH, ~/.cargo/bin, or target/{debug,release})
#
# Usage:
#   ./contrib/kde-auto-pause.sh
#   Or add to KDE System Settings -> Autostart -> Add Login Script
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

# Detect available qdbus binary
QDBUS_BIN=""
if command -v qdbus6 >/dev/null 2>&1; then
    QDBUS_BIN="qdbus6"
elif command -v qdbus >/dev/null 2>&1; then
    QDBUS_BIN="qdbus"
else
    echo "Error: qdbus6 or qdbus is required on KDE Plasma." >&2
    exit 1
fi

if ! command -v dbus-monitor >/dev/null 2>&1; then
    echo "Error: dbus-monitor is required." >&2
    exit 1
fi

KWIN_PLUGIN_NAME="wallrs_kwin_auto_pause"
TEMP_JS="$(mktemp /tmp/wallrs_kwin_XXXXXX.js)"

cleanup() {
    trap - EXIT INT TERM
    echo "[wallrs-kde] Unloading KWin auto-pause script..." >&2
    "${QDBUS_BIN}" org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "${KWIN_PLUGIN_NAME}" >/dev/null 2>&1 || true
    rm -f "${TEMP_JS}"
    if [ -n "${MONITOR_PID:-}" ]; then
        kill "${MONITOR_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# Generate KWin ECMAScript
# Mode 0 = not maximized, 1 = vertical, 2 = horizontal, 3 = fully maximized
cat << 'EOF' > "${TEMP_JS}"
function checkState() {
    var curDesk = workspace.currentDesktop;
    var wins = workspace.windowList();
    var shouldPause = false;

    for (var i = 0; i < wins.length; i++) {
        var w = wins[i];
        if (!w.normalWindow || w.minimized || w.hiddenByShowDesktop) {
            continue;
        }

        var onCurrent = w.onAllDesktops;
        if (!onCurrent && w.desktops) {
            for (var j = 0; j < w.desktops.length; j++) {
                if (w.desktops[j] === curDesk) {
                    onCurrent = true;
                    break;
                }
            }
        }

        if (!onCurrent) {
            continue;
        }

        // Pause if any window on current desktop is fullscreen or maximized (mode 3)
        if (w.fullScreen || w.maximizeMode === 3) {
            shouldPause = true;
            break;
        }
    }

    callDBus('org.kde.wallrs', '/wallrs', 'org.kde.wallrs', 'setPause', shouldPause);
}

function hookWindow(w) {
    if (!w.normalWindow) return;
    w.maximizedChanged.connect(checkState);
    w.fullScreenChanged.connect(checkState);
    w.minimizedChanged.connect(checkState);
    w.hiddenByShowDesktopChanged.connect(checkState);
    w.desktopsChanged.connect(checkState);
}

workspace.windowAdded.connect(function(w) {
    hookWindow(w);
    checkState();
});

workspace.windowRemoved.connect(checkState);
workspace.windowActivated.connect(checkState);
workspace.currentDesktopChanged.connect(checkState);

var initialWins = workspace.windowList();
for (var k = 0; k < initialWins.length; k++) {
    hookWindow(initialWins[k]);
}

checkState();
EOF

# Ensure any previous instance is unloaded
"${QDBUS_BIN}" org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "${KWIN_PLUGIN_NAME}" >/dev/null 2>&1 || true

# Load and start script in KWin
SCRIPT_ID="$("${QDBUS_BIN}" org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript "${TEMP_JS}" "${KWIN_PLUGIN_NAME}" 2>/dev/null || echo "")"

if [ -z "${SCRIPT_ID}" ]; then
    echo "Error: Failed to register script with KWin /Scripting." >&2
    exit 1
fi

"${QDBUS_BIN}" org.kde.KWin "/Scripting/Script${SCRIPT_ID}" org.kde.kwin.Script.run >/dev/null 2>&1 || true

echo "[wallrs-kde] Connected to KWin 6 scripting interface."
echo "[wallrs-kde] Using wallctl binary: ${WALLCTL_BIN}"
echo "[wallrs-kde] Monitoring window state (Maximized/Fullscreen -> Pause, Floating/Desktop -> Resume)..."

CURRENT_PAUSED=""

# Listen for D-Bus notifications from the KWin script
dbus-monitor "type='method_call',interface='org.kde.wallrs',member='setPause'" 2>/dev/null | while read -r line; do
    case "${line}" in
        *"boolean true"*)
            if [ "${CURRENT_PAUSED}" != "true" ]; then
                CURRENT_PAUSED="true"
                echo "[wallrs-kde] Maximized/Fullscreen window active -> Pausing wallpaper"
                "${WALLCTL_BIN}" pause >/dev/null 2>&1 || true
            fi
            ;;
        *"boolean false"*)
            if [ "${CURRENT_PAUSED}" != "false" ]; then
                CURRENT_PAUSED="false"
                echo "[wallrs-kde] Desktop visible / Floating windows -> Resuming wallpaper"
                "${WALLCTL_BIN}" resume >/dev/null 2>&1 || true
            fi
            ;;
    esac
done
