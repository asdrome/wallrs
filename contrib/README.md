# contrib/ - Community and Helper Scripts

This directory contains optional, user-space helper scripts for integrating `wallrs` with specific desktop environments and window managers.

## Rationale

The core `wallrsd` daemon communicates strictly over standard Wayland protocols (`wlr-layer-shell-unstable-v1` and optionally `zwlr_foreign_toplevel_manager_v1`).

However:
1. **KDE Plasma 6 (KWin)** does not implement `zwlr_foreign_toplevel_manager_v1` due to KDE's privacy policy.
2. **Tiling Compositors (Hyprland, Sway)** rarely leave windows in maximized or fullscreen states, meaning `foreign_toplevel` alone cannot infer whether tiled windows are covering the active workspace. Note that **fullscreen pausing is already handled natively by `wallrsd` on Hyprland** without requiring any scripts.

Rather than polluting the pure Wayland Rust daemon with compositor-specific D-Bus or IPC libraries, these lightweight helper scripts leverage compositor-native signals to toggle `wallctl pause` and `wallctl resume`.

---

## Scripts

### 1. `hyprland-auto-pause.sh`
Monitors active workspaces over Hyprland's IPC `socket2.sock` event stream.

- **Default Behavior**: Pauses rendering when opaque, tiled windows occupy the active workspace; resumes when the workspace is empty or contains **only transparent windows** (terminals like Kitty, Alacritty, Foot) so wallpapers remain visible through blur and transparency.
- **Config**: Set `PAUSE_ON_ANY_WINDOW=0` to pause only when a window is fullscreen.
- **Environment Variables**:
  - `TRANSPARENT_CLASSES`: Regex of classes to treat as transparent (default: `kitty|Alacritty|foot|wezterm|ghostty`).
  - `IGNORE_FLOATING`: Set `1` (default) to ignore small floating dialogs/calculators, `0` to count them.
  - `WALLCTL_BIN`: Optional path to custom `wallctl` binary (defaults to searching `PATH`, `~/.cargo/bin`, and local `target/{release,debug}`).
- **Hyprland autostart**:
  ```ini
  # in ~/.config/hypr/hyprland.conf
  exec-once = /path/to/wallrs/contrib/hyprland-auto-pause.sh
  ```

### 2. `kde-auto-pause.sh`
Injects a lightweight KWin 6 script via KWin's D-Bus `/Scripting` interface to monitor window states in real time (`maximizeMode`, `fullScreen`, `hiddenByShowDesktop`).

- **Default Behavior**: Pauses rendering when a window is maximized or fullscreen on the active desktop. Resumes rendering when windows are restored to floating mode, minimized, or when "Show Desktop" (`Meta+D`) is activated.
- **KDE autostart**: Add to **System Settings -> Autostart -> Add Login Script**, or launch from `~/.config/plasma-workspace/env/`.

---

### 3. `hyprland-matugen.sh` (Material You System Theming)
Connects `wallrs` to [Matugen](https://github.com/InioX/matugen) to automatically theme Hyprland, Waybar, Mako, and terminal emulators.

- **Workflow**: Calls `wallctl preview` to resolve the current representative image (image wallpaper, manifest thumbnail, or live GPU snapshot for procedural shaders/videos), then passes it to `matugen image`.
- **Usage**:
  ```bash
  # Apply current wallpaper colors to your Matugen templates
  ./contrib/hyprland-matugen.sh

  # Force a live snapshot even if a thumbnail exists
  ./contrib/hyprland-matugen.sh --snapshot
  ```

### 4. `kde-accent-color.sh` (KDE Plasma 6 Accent Color)
Connects `wallrs` to KDE Plasma's native accent color manager.

- **Workflow**: Calls `wallctl preview`, extracts the dominant vibrant color via Python/Pillow (or ImageMagick), and applies the accent color cleanly via `plasma-apply-colorscheme` and `kwriteconfig6` without restarting `plasmashell` or dropping layer surfaces.
- **Usage**:
  ```bash
  # Apply current wallpaper color to KDE Plasma 6 accent color
  ./contrib/kde-accent-color.sh
  ```

---

### 5. `wallrs-theme-sync.sh` (Desktop-Agnostic Theming Dispatcher)
Dispatches theming updates automatically based on the running desktop environment (`$XDG_CURRENT_DESKTOP`).

- **Workflow**:
  - On KDE Plasma: executes `kde-accent-color.sh`.
  - On Hyprland / Sway: executes `hyprland-matugen.sh` (or `matugen` directly).
- **Systemd Integration (Recommended)**:
  Enable the included systemd path unit so theming updates automatically whenever `wallrsd` changes the wallpaper:
  ```bash
  systemctl --user enable --now wallrs-theme-sync.path
  ```

---

### 6. `wallrs-auto-pause.sh` (Desktop-Agnostic Auto-Pause Dispatcher)
Dispatches automatic wallpaper pause/resume based on the running desktop environment (`$XDG_CURRENT_DESKTOP`).

- **Workflow**:
  - On KDE Plasma: executes `kde-auto-pause.sh` (monitoring KWin D-Bus window states).
  - On Hyprland: executes `hyprland-auto-pause.sh` (monitoring socket2 IPC tiling state).
  - On Sway / wlroots: informs that fullscreen pause is handled natively by `wallrsd` via `zwlr_foreign_toplevel_manager_v1`, idling with zero resource consumption.
- **Systemd Integration (Recommended)**:
  Enable the included systemd service so window tracking and pausing runs automatically with your desktop session:
  ```bash
  systemctl --user enable --now wallrs-auto-pause.service
  ```



