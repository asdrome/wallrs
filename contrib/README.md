# contrib/ - Community and Helper Scripts

This directory contains optional, user-space helper scripts for integrating `wallrs` with specific desktop environments and window managers.

## Rationale

The core `wallrsd` daemon communicates strictly over standard Wayland protocols (`wlr-layer-shell-unstable-v1` and optionally `zwlr_foreign_toplevel_manager_v1`).

However:
1. **KDE Plasma 6 (KWin)** does not implement `zwlr_foreign_toplevel_manager_v1` due to KDE's privacy policy.
2. **Tiling Compositors (Hyprland, Sway)** rarely leave windows in maximized or fullscreen states, meaning `foreign_toplevel` alone cannot infer whether tiled windows are covering the active workspace.

Rather than polluting the pure Wayland Rust daemon with compositor-specific D-Bus or IPC libraries, these lightweight helper scripts leverage compositor-native signals to toggle `wallctl pause` and `wallctl resume`.

---

## Scripts

### 1. `hyprland-auto-pause.sh`
Listens to Hyprland's IPC `socket2.sock` event stream (`workspace`, `activewindow`, `openwindow`, `closewindow`, `fullscreen`).

- **Default Behavior**: Pauses rendering when windows are open on the active workspace; resumes when the active workspace is empty.
- **Config**: Set `PAUSE_ON_ANY_WINDOW=0` to pause only when a window is fullscreen.
- **Hyprland autostart**:
  ```ini
  # in ~/.config/hypr/hyprland.conf
  exec-once = /path/to/wallrs/contrib/hyprland-auto-pause.sh
  ```

### 2. `kde-auto-pause.sh`
Monitors KWin's D-Bus interface (`org.kde.KWin.showingDesktop` and `activeWindowChanged`).

- **Default Behavior**: Resumes rendering when the desktop is shown (e.g. `Meta+D`), pauses when windows cover the screen.
- **KDE autostart**: Add to **System Settings -> Autostart -> Add Login Script**, or launch from `~/.config/plasma-workspace/env/`.

