# wallrs 🌌

[![Rust](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org/)
[![Wayland](https://img.shields.io/badge/wayland-native-blue.svg)](https://wayland.freedesktop.org/)
[![Vulkan](https://img.shields.io/badge/graphics-wgpu%20%2F%20vulkan-red.svg)](https://wgpu.rs/)
[![PipeWire](https://img.shields.io/badge/audio-pipewire-brightgreen.svg)](https://pipewire.org/)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-lightgrey.svg)](LICENSE-MIT)

A high-performance, Wayland-native live wallpaper daemon (`wallrsd`) and CLI controller (`wallctl`) written in pure Rust.

Designed from the ground up to replace bloated, legacy X11/GLX/PulseAudio tools with a clean, memory-safe, and energy-efficient architecture built upon **`wgpu` (Vulkan)**, **`smithay-client-toolkit` (`wlr-layer-shell`)**, **PipeWire audio streams**, and **`libmpv2`**.

---

## ✨ Features

- **True Wayland Native**: Integrates directly with `zwlr_layer_shell_v1` on the `background` layer. Zero Xwayland, zero GLX, zero PulseAudio dependencies.
- **Hardware-Accelerated Rendering (`wgpu`)**: High-performance Vulkan swapchains with explicit refresh rate synchronization via `wl_surface.frame()`.
- **Multi-Backend Wallpaper Content**:
  - **Static / Parallax Images**: Multi-layer compositing with real-time pointer parallax and continuous linear panning.
  - **Procedural Shaders**: Native WGSL shaders and automatic GLSL (Shadertoy) translation with built-in uniforms (`u_time`, `u_resolution`, `u_mouse`, `u_audio_spectrum`).
  - **Video Playback**: Low-overhead hardware/software video decoding and audio playback via `libmpv2`.
- **PipeWire Audio Spectrum Analyzer**: Zero-latency capture through `pw_stream` feeding a multi-band FFT analyzer (`RustFFT`) with logarithmic frequency binning for reactive music visualizers.
- **Battery & Resource Efficiency**:
  - **Fullscreen & Maximized Pausing**: Monitors toplevel window states via `zwlr_foreign_toplevel_management_v1`. Wallpaper rendering and MPV playback freeze automatically when a window is fullscreen or maximized, reducing GPU/CPU consumption to ~0%.
  - **Configurable FPS Ceiling (`--fps`)**: Enforce upper render frame rate limits without replacing compositor vblank callbacks.
- **Robust Output Fault Isolation**: Each monitor's render loop is guarded by `catch_unwind`. A panic or shader error on one monitor will never crash the daemon or affect other screens.
- **Instant Offscreen Screenshots**: Capture pixel-perfect wallpaper frames straight from GPU memory into PNG/JPEG/WebP via `wallctl screenshot`.
- **Ergonomic IPC Control**: Fast Unix domain socket server with zero-cost JSON serialization and human-friendly CLI commands.

---

## 🖥️ Compositor Compatibility Matrix

| Compositor          | Protocol (`wlr-layer-shell`) | Fullscreen / Maximize Pause | Status                                                       |
| :------------------ | :--------------------------: | :-------------------------: | :----------------------------------------------------------- |
| **Hyprland**        |            ✅ Yes             |            ✅ Yes            | **Fully Supported** (Primary target)                         |
| **Sway**            |            ✅ Yes             |            ✅ Yes            | **Fully Supported**                                          |
| **River**           |            ✅ Yes             |            ✅ Yes            | **Fully Supported**                                          |
| **Labwc / Wayfire** |            ✅ Yes             |            ✅ Yes            | **Fully Supported**                                          |
| **KDE Plasma 6**    |            ✅ Yes             |          ⚠️ Partial          | Supported (via kwin layer-shell integration)                 |
| **GNOME Mutter**    |             ❌ No             |            ❌ No             | **Unsupported** (GNOME does not implement `wlr-layer-shell`) |

---

## 📦 Prerequisites & System Dependencies

### Arch Linux / Manjaro
```bash
sudo pacman -S --needed rust cargo vulkan-icd-loader pipewire mpv wayland pkgconf
```

### Fedora
```bash
sudo dnf install rust cargo vulkan-loader-devel pipewire-devel mpv-devel wayland-devel pkgconf-pkg-config
```

### Ubuntu / Debian (24.04+)
```bash
sudo apt install cargo rustc libvulkan-dev libpipewire-0.3-dev libmpv-dev libwayland-dev pkg-config
```

---

## 🚀 Installation

### 1. Build and Install via Makefile (Recommended)
```bash
git clone https://github.com/asdromundo/wallrs.git
cd wallrs
make
sudo make install
```
*Installs `wallrsd` and `wallctl` to `/usr/local/bin`, and the systemd unit to `/usr/local/lib/systemd/user/`.*

### 2. Arch Linux (PKGBUILD)
```bash
cd extra/arch
makepkg -si
```

### 3. Cargo Install (From source)
```bash
cargo install --path crates/wallrs-daemon
cargo install --path crates/wallrs-cli
```

---

## ⚙️ Running the Daemon (`wallrsd`)

### Autostart Options

#### Option A: Systemd User Service (Recommended)
```bash
systemctl --user daemon-reload
systemctl --user enable --now wallrsd
```

#### Option B: In Hyprland (`hyprland.conf`)
```ini
exec-once = wallrsd
```

#### Option C: In Sway (`~/.config/sway/config`)
```ini
exec wallrsd
```

### Daemon CLI Options
```text
Usage: wallrsd [OPTIONS]

Options:
      --color <COLOR>              Initial background solid color (hex: #RRGGBB or comma-separated RGBA) [default: #0f172a]
      --fps <FPS>                  Maximum frames-per-second render ceiling (default: unlimited / display vblank)
      --no-fullscreen-pause        Disable automatic pausing of wallpaper rendering when a window is in fullscreen
      --no-pause-on-maximized      Disable automatic pausing of wallpaper rendering when a window is maximized
  -h, --help                       Print help
  -V, --version                    Print version
```

---

## 🎮 Command-Line Controller (`wallctl`)

`wallctl` communicates with the running daemon over the IPC socket.

```bash
# List all active display outputs and their current status
wallctl list

# Apply a wallpaper from a manifest folder or file
wallctl set-wallpaper examples/audio-visualizer
wallctl set-wallpaper examples/video-with-sound --output eDP-1

# Pause and resume rendering manually
wallctl pause
wallctl resume --output HDMI-A-1

# Toggle pause state (ideal for hotkeys)
wallctl toggle-pause
wallctl toggle

# Change solid background color on the fly
wallctl set-color "#1e1e2e"
wallctl set-color "#ff007f" --output eDP-1

# Adjust runtime properties
wallctl set-property volume 25.0
wallctl set-property speed 1.5 --output eDP-1
wallctl set-property color "#00ffcc"

# Capture a screenshot from GPU memory
wallctl screenshot eDP-1 ~/Pictures/wallpaper_snap.png

# Gracefully stop the daemon
wallctl kill
```

---

## 🎨 Wallpaper Manifest Guide (`wallpaper.toml`)

Wallpapers in `wallrs` are organized as directories containing a `wallpaper.toml` manifest and accompanying assets.

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
source = "visualizer.wgsl"
audio = true   # Connects to PipeWire capture and binds u_audio_spectrum
```

#### Shader Uniforms Provided by `wallrs`:
```wgsl
struct Uniforms {
    time: f32,                   // Elapsed time in seconds
    delta_time: f32,             // Delta time since last frame
    resolution: vec2<f32>,       // Output dimensions in pixels (width, height)
    mouse: vec2<f32>,            // Cursor position in pixels
    audio_spectrum: array<vec4<f32>, 16>, // 64 frequency bands (16 x vec4)
};
@group(0) @binding(0) var<uniform> u: Uniforms;
```

#### Shadertoy Compatibility:
To use existing Shadertoy GLSL shaders:
```toml
[wallpaper]
type = "shader"
name = "plasma-shadertoy"

[shader]
source = "plasma.glsl"
shadertoy = true
```

### 3. Video Wallpaper (Hardware Accelerated)
Seamless looping video playback with volume and speed controls via `libmpv2`.

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

---

## ⌨️ Hyprland Keybindings Example

Add these bindings to your `~/.config/hypr/hyprland.conf`:

```ini
# Toggle wallpaper rendering pause
bind = $mainMod, P, exec, wallctl toggle-pause

# Take a screenshot of the current wallpaper on the primary display
bind = $mainMod SHIFT, W, exec, wallctl screenshot eDP-1 ~/Pictures/wallpaper_$(date +%s).png

# Quick wallpaper cycling
bind = $mainMod CTRL, 1, exec, wallctl set-wallpaper ~/Wallpapers/synthwave
bind = $mainMod CTRL, 2, exec, wallctl set-wallpaper ~/Wallpapers/audio-visualizer
bind = $mainMod CTRL, 3, exec, wallctl set-wallpaper ~/Wallpapers/video-landscape
```

---

## 🏗️ Workspace Architecture

```text
wallrs/
├── crates/
│   ├── wallrs-proto/           # Shared IPC protocol types, JSON serialization, and manifest parser
│   ├── wallrs-render/          # WGPU rendering abstractions, swapchain helpers, color conversions
│   ├── wallrs-audio/           # PipeWire client, ring buffers, RustFFT spectrum analysis
│   ├── wallrs-content-image/   # Multi-layer parallax image engine
│   ├── wallrs-content-shader/  # WGSL & Shadertoy GLSL compiler and renderer
│   ├── wallrs-content-video/   # libmpv2 integration, video decode & software render loop
│   ├── wallrs-core/            # SCTK layer-shell event loop, toplevel detection, engine state
│   ├── wallrs-daemon/          # `wallrsd` executable
│   └── wallrs-cli/             # `wallctl` CLI tool
├── examples/                   # Ready-to-use sample wallpapers
├── extra/                      # Systemd user units and Arch Linux PKGBUILD
└── Makefile                    # Standard build and installation targets
```

---

## 📄 License

Dual-licensed under either of:
- **MIT License** ([LICENSE-MIT](LICENSE-MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

