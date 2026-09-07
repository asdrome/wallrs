# wallrs

[![Rust](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org/)
[![Wayland](https://img.shields.io/badge/wayland-native-blue.svg)](https://wayland.freedesktop.org/)
[![Vulkan](https://img.shields.io/badge/graphics-wgpu%20%2F%20vulkan-red.svg)](https://wgpu.rs/)
[![PipeWire](https://img.shields.io/badge/audio-pipewire-brightgreen.svg)](https://pipewire.org/)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-lightgrey.svg)](LICENSE-MIT)

A Wayland-native live wallpaper daemon (`wallrsd`) and CLI client (`wallctl`) written in Rust using `wgpu` (Vulkan), `smithay-client-toolkit` (`wlr-layer-shell`), PipeWire, and `libmpv`.

---

## Features

- **Wayland native**: Renders to the `background` layer via `zwlr_layer_shell_v1` without X11 or Xwayland dependencies.
- **Hardware-accelerated rendering**: Vulkan swapchains managed via `wgpu`, synchronized with display refresh rate via `wl_surface.frame()`.
- **Multiple content backends**:
  - **Static and parallax images**: Multi-layer compositing with cursor-driven parallax and continuous linear panning.
  - **Procedural shaders**: Native WGSL shaders and translated Shadertoy GLSL shaders with built-in uniforms (`u_time`, `u_resolution`, `u_mouse`, `u_audio_spectrum`).
  - **Video playback**: Hardware and software decoding via `libmpv` with volume, mute, and speed controls.
  - **Background audio**: Ambient audio tracks (`[audio]`) attached to image or shader wallpapers. Audio is muted by default to prevent unwanted desktop noise.
- **Audio reactivity**: Real-time audio capture via PipeWire (`pw_stream`) with a 64-band logarithmic FFT analyzer (`RustFFT`). Captures are initiated on-demand and released when inactive.
- **Resource management**: Automatic pause and resume on fullscreen or maximized windows (`zwlr_foreign_toplevel_management_v1`). Configurable FPS ceiling via `--fps`.
- **Fault isolation**: Each display output runs an isolated render loop protected by `catch_unwind`, preventing an error on one monitor from affecting others.
- **Direct GPU screenshots**: Framebuffer capture straight to PNG, JPEG, or WebP via `wallctl screenshot`.
- **Unix socket IPC**: JSON-based control protocol over Unix domain sockets via `wallctl`.

---

## Compositor Compatibility

| Compositor              | `wlr-layer-shell` | Fullscreen / Maximize Pause | Status      | Notes                                                                  |
| :---------------------- | :---------------: | :-------------------------: | :---------- | :--------------------------------------------------------------------- |
| **Hyprland**            |        Yes        |             Yes             | Supported   | Tiling setups can use `contrib/hyprland-auto-pause.sh`                 |
| **Sway**                |        Yes        |             Yes             | Supported   | Native `foreign_toplevel` tracking                                     |
| **River**               |        Yes        |             Yes             | Supported   | Native `foreign_toplevel` tracking                                     |
| **Labwc / Wayfire**     |        Yes        |             Yes             | Supported   | Native `foreign_toplevel` tracking                                     |
| **KDE Plasma 6 (KWin)** |        Yes        |      External / Helper      | Supported   | Uses namespace `"desktop"`; auto-pause via `contrib/kde-auto-pause.sh` |
| **GNOME (Mutter)**      |        No         |             No              | Unsupported | Mutter does not implement `wlr-layer-shell`                            |

---

## Dependencies

### Runtime Dependencies
- `vulkan-loader`
- `pipewire`
- `mpv` (`libmpv.so.2` or `libmpv.so.1`)
- `wayland-client`

### Build Dependencies
- Rust 1.85+ (`cargo`, `rustc`)
- `pkg-config`
- `libvulkan-dev` / `vulkan-loader-devel`
- `libpipewire-0.3-dev` / `pipewire-devel`
- `libmpv-dev` / `mpv-devel`
- `libwayland-dev` / `wayland-devel`

#### Distribution Packages

**Arch Linux / Manjaro**:
```bash
sudo pacman -S --needed rust cargo vulkan-icd-loader pipewire mpv wayland pkgconf
```

**Fedora / RHEL**:
```bash
sudo dnf install rust cargo vulkan-loader-devel pipewire-devel mpv-devel wayland-devel pkgconf-pkg-config
```

**Ubuntu / Debian (24.04+)**:
```bash
sudo apt install cargo rustc libvulkan-dev libpipewire-0.3-dev libmpv-dev libwayland-dev pkg-config
```

---

## Installation

### Prebuilt Packages (GitHub Releases)

Precompiled packages are available on the [Releases](https://github.com/asdrome/wallrs/releases) page:
- **Debian / Ubuntu / Kubuntu (`.deb`)**:
  ```bash
  sudo apt install ./wallrs_*.deb
  ```
- **Fedora / RHEL (`.rpm`)**:
  ```bash
  sudo dnf install ./wallrs-*.rpm
  ```
- **Generic x86_64 tarball (`.tar.gz`)**: Contains binaries, documentation, and systemd user units.

### Building From Source

```bash
git clone https://github.com/asdrome/wallrs.git
cd wallrs
make
sudo make install
```
*Installs `wallrsd` and `wallctl` to `/usr/local/bin`, and the systemd unit to `/usr/local/lib/systemd/user/`.*

### Local Package Generation

**Debian / Ubuntu package (`.deb`)**:
```bash
make deb
sudo apt install ./target/debian/wallrs_*.deb
```

**Fedora / RPM package (`.rpm`)**:
```bash
make rpm
sudo dnf install ~/rpmbuild/RPMS/x86_64/wallrs-*.rpm
```

**Arch Linux (`PKGBUILD`)**:
```bash
cd extra/arch
makepkg -si
```

---

## Daemon Configuration (`wallrsd`)

### Autostart

#### Systemd user service (recommended)
```bash
systemctl --user daemon-reload
systemctl --user enable --now wallrsd
```

#### Hyprland (`hyprland.conf`)
```ini
exec-once = wallrsd
```

#### Sway (`~/.config/sway/config`)
```ini
exec wallrsd
```

### CLI Options

```text
Usage: wallrsd [OPTIONS]

Options:
      --color <COLOR>          Initial solid background color (#RRGGBB or RGBA) [default: #0f172a]
      --fps <FPS>              Render frame rate ceiling (default: display vblank)
      --no-fullscreen-pause    Disable automatic pause on fullscreen windows
      --no-pause-on-maximized  Disable automatic pause on maximized windows
      --allow-audio            Allow audio playback from wallpapers by default (default: muted)
  -h, --help                   Print help
  -V, --version                Print version
```

---

## CLI Usage (`wallctl`)

`wallctl` communicates with the running daemon over a Unix domain socket.

```bash
# List connected display outputs, resolutions, and statuses
wallctl list

# Apply a wallpaper from a folder or manifest (audio starts muted by default)
wallctl set-wallpaper examples/parallax-with-audio
wallctl set-wallpaper examples/video-sunset --output eDP-1

# Apply a wallpaper and unmute audio immediately
wallctl set-wallpaper examples/video-sunset --unmute

# Audio controls
wallctl mute
wallctl unmute --output eDP-1
wallctl toggle-mute

# Manual pause and resume
wallctl pause
wallctl resume --output HDMI-A-1
wallctl toggle-pause

# Change background color
wallctl set-color "#1e1e2e"
wallctl set-color "#ff007f" --output eDP-1

# Modify runtime properties
wallctl set-property volume 25.0
wallctl set-property speed 1.5 --output eDP-1

# Direct GPU screenshot
wallctl screenshot eDP-1 ~/Pictures/wallpaper_snap.png

# Lint and validate a wallpaper before loading
wallctl validate examples/aurora-shader

# Terminate the daemon
wallctl kill
```

---

## Wallpaper Configuration (`wallpaper.toml`)

Wallpapers are stored in directories containing a `wallpaper.toml` manifest and media assets. See the [Authoring Guide](docs/authoring-guide.md) for full specification and templates.

### 1. Parallax Image Wallpaper
Multi-layer image with pointer-driven parallax and continuous linear pan.

```toml
[wallpaper]
type = "image"
name = "cyberpunk-street"
author = "Artist Name"
description = "Multi-layer parallax cityscape"

[[image.layers]]
path = "background.png"

[[image.layers]]
path = "midground.png"
parallax = 0.25
pan = { speed = 0.01, axis = "x" }

[[image.layers]]
path = "foreground.png"
parallax = 0.6
```

### 2. Audio-Reactive Shader Wallpaper
GPU shaders with automatic uniform binding and PipeWire audio reactivity.

```toml
[wallpaper]
type = "shader"
name = "audio-visualizer"
description = "Spectrum-reactive soundwave visualizer"

[shader]
entry = "visualizer.wgsl"
audio = true   # Connects PipeWire capture and binds u_audio_spectrum
```

#### Uniform layout provided by `wallrs`:
```wgsl
struct Uniforms {
    time: f32,                            // Elapsed time in seconds
    delta_time: f32,                      // Delta time since last frame
    resolution: vec2<f32>,                // Output dimensions in pixels (width, height)
    mouse: vec2<f32>,                     // Cursor position in pixels
    audio_spectrum: array<vec4<f32>, 16>, // 64 frequency bands (16 x vec4)
};
@group(0) @binding(0) var<uniform> u: Uniforms;
```

#### Shadertoy GLSL translation:
```toml
[wallpaper]
type = "shader"
name = "plasma-shadertoy"

[shader]
entry = "plasma.glsl"
```

### 3. Video Wallpaper
Looping video playback with volume and speed controls via `libmpv`.

```toml
[wallpaper]
type = "video"
name = "lofi-cafe"

[video]
path = "video.mp4"
loop = true
volume = 40.0
mute = false
speed = 1.0
```

### 4. Background Audio (`[audio]`)
Attach an ambient audio track to any image or shader wallpaper:

```toml
[wallpaper]
type = "image"
name = "cyber-cafe"

[[image.layers]]
path = "cafe.png"

[audio]
path = "ambient.ogg"
volume = 35.0
loop = true
```

---

## Window Tracking & Community Helpers (`contrib/`)

Because KDE Plasma 6 does not expose `zwlr_foreign_toplevel_manager_v1` and tiling window managers (Hyprland, Sway) rarely leave windows in maximized/fullscreen states, optional helper scripts are provided in [`contrib/`](contrib/):

- **[`contrib/hyprland-auto-pause.sh`](contrib/hyprland-auto-pause.sh)**: Watches Hyprland's IPC socket2 event stream and pauses rendering when windows occupy the active workspace.
- **[`contrib/kde-auto-pause.sh`](contrib/kde-auto-pause.sh)**: Monitors KWin's D-Bus interface (`org.kde.KWin.showingDesktop`) to pause rendering when windows cover the screen and resume when the desktop is exposed (`Meta+D`).

---

## Workspace Architecture

```text
wallrs/
├── crates/
│   ├── wallrs-proto/           # IPC protocol types, JSON serialization, manifest parser
│   ├── wallrs-render/          # WGPU abstractions, swapchain management, color conversion
│   ├── wallrs-audio/           # PipeWire client, on-demand stream lifecycle, RustFFT spectrum analysis
│   ├── wallrs-content-image/   # Multi-layer parallax and panning image engine
│   ├── wallrs-content-shader/  # WGSL and Shadertoy GLSL compiler and renderer
│   ├── wallrs-content-video/   # libmpv integration, video decode and render loop
│   ├── wallrs-core/            # SCTK layer-shell event loop, toplevel detection, engine state
│   ├── wallrs-daemon/          # wallrsd daemon executable
│   └── wallrs-cli/             # wallctl CLI tool
├── contrib/                    # Window manager and desktop integration helper scripts
├── examples/                   # Sample wallpapers (image, shader, video, audio)
├── extra/                      # Packaging files (RPM spec, PKGBUILD, systemd unit)
├── scripts/                    # Maintenance scripts (SemVer bumper)
└── Makefile                    # Standard build and installation targets
```

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
