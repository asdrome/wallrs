# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.1] - 2026-09-07

### Fixed
- **Paused Wallpaper Update**: Fixed an issue where changing wallpapers while playback was paused did not present the new frame until unpaused. Replacing a wallpaper now immediately renders and commits the initial frame to Wayland.
- **Shader & Video Preview / Screenshot**: Fixed a `wgpu` validation error during `wallctl preview` or screenshot capture on shader and video wallpapers by aligning surface target formats (`Bgra8UnormSrgb`) and isolating render passes.
- **Automated Theme Synchronization**: Added `--prefer saturation` flag to `matugen` script calls in `contrib/` to eliminate interactive user prompts in non-interactive / systemd execution.

### Performance
- **Binary Footprint & Dependency Optimization**:
  - Replaced `rustfft` in `wallrs-audio` with an in-place Radix-2 Cooley-Tukey FFT with precomputed twiddle tables, reducing crate footprint by 89%.
  - Streamlined `tracing-subscriber` to basic `fmt` without heavy regex automata.
  - Configured release profile with `strip = "debuginfo"`.

## [1.1.0] - 2026-09-07

### Added
- **Event-Driven Idle Rendering (0.0% Idle CPU)**:
  - Added `fn is_animated(&self) -> bool` to `WallpaperRenderer` trait.
  - Implemented animation detection in `SolidColorRenderer` and `ImageRenderer` (returns `false` for static colors and images without pan or parallax).
  - Optimized `OutputSurface::render_frame` to stop requesting Wayland frame callbacks (`wl_surface.frame`) when wallpapers are non-animated, reducing idle CPU usage from ~10% down to **0.0%**.
  - Event-driven re-rendering upon dynamic property changes (`set_property`), window unpause, or display reconfiguration.
- **Session State Persistence Across Reboots**:
  - Atomic state snapshot engine saving active wallpapers, mute status, and custom properties to `$XDG_STATE_HOME/wallrs/state.json`.
  - Automatic session restoration on daemon startup and output reconnection.
  - Added `--no-restore` and `--state-file` CLI arguments to `wallrsd`.
- **Manifest Thumbnail & Representative Preview**:
  - Optional `thumbnail` metadata field in `wallpaper.toml` (`[wallpaper]` block).
  - Added `wallctl preview [--output <NAME>] [--snapshot]` to query canonical image paths or capture live GPU screenshot fallbacks.
  - Added `contrib/hyprland-matugen.sh` (Material You theming) and `contrib/kde-accent-color.sh` (KDE Plasma 6 accent color sync).
- **Desktop Integration & Systemd Services**:
  - Added `wallrs-theme-sync.service` and `wallrs-theme-sync.path` with inotify tracking over `state.json` to synchronize accent colors automatically across KDE Plasma and Hyprland.
  - Added `wallrs-auto-pause.service` and `wallrs-auto-pause.sh` desktop-agnostic dispatcher for automatic window occlusion pause on KDE Plasma 6 (KWin D-Bus) and Hyprland (socket2).
- **Packaging & Metadata**:
  - Validated FreeDesktop AppStream metainfo specification (`extra/metainfo/com.asdrome.wallrs.metainfo.xml`).
  - Standardized Asdrome organization URLs and package metadata across Debian, RPM, and Arch Linux packages.

## [1.0.0] - 2026-09-07

### Added
- **Wayland Core & Display Engine (`wallrsd`)**:
  - Native Wayland client built on `smithay-client-toolkit` rendering to the `background` layer via `zwlr_layer_shell_v1`.
  - Configurable namespace (`desktop` by default) ensuring native compatibility with KWin (KDE Plasma 6) desktop semantics and window minimizations.
  - Multi-monitor support with per-output isolated rendering loops protected by `std::panic::catch_unwind`.
  - High-performance Vulkan swapchain rendering managed via `wgpu`, synchronized with display refresh rate via `wl_surface.frame()`.
  - Configurable render frame rate ceiling (`--fps`) to constrain energy consumption.
  - Automatic fullscreen and maximized window detection via `zwlr_foreign_toplevel_manager_v1` to pause rendering when the desktop is obstructed.

- **Multi-Backend Content Renderers**:
  - **Static & Parallax Images (`wallrs-content-image`)**: Multi-layer compositing with cursor-driven parallax, continuous linear panning, and opacity adjustments.
  - **Procedural Shaders (`wallrs-content-shader`)**: Native WGSL pipeline and Shadertoy GLSL translation shim with built-in uniforms (`time`, `delta_time`, `resolution`, `mouse`, `audio_spectrum`).
  - **Video Playback (`wallrs-content-video`)**: Hardware-accelerated and software video decoding via `libmpv` with volume, mute, loop, and playback speed controls.
  - **Background Audio (`wallrs-audio`)**: Standalone ambient audio track support (`[audio]`) attached to image or shader wallpapers.

- **Audio Subsystem & Spectrum Analysis**:
  - Direct low-latency capture through PipeWire (`pw_stream`) with on-demand (lazy) capture lifecycle management.
  - Real-time 64-band logarithmic frequency binning using `RustFFT` with exponential smoothing for music visualizers.
  - Privacy-friendly passive audio node properties (`NODE_PASSIVE = "true"`, `NODE_VIRTUAL = "true"`) to prevent continuous microphone indicator alerts.

- **Audio Default Policy & Controls**:
  - Wallpapers with audio or video tracks default to muted (`mute = true`) on load to avoid unwanted desktop noise.
  - Daemon flag `--allow-audio` to opt into unmuted playback upon loading.
  - Subcommands in `wallctl`: `mute`, `unmute`, `toggle-mute`, and `--unmute` flag for `set-wallpaper`.
  - Audio status (`Muted` / `Unmuted`) reported in `wallctl list`.

- **Command-Line Controller (`wallctl`)**:
  - Fast JSON-based Unix domain socket client for runtime daemon management.
  - Dynamic property adjustments (`set-property volume`, `set-property color`, `set-property speed`).
  - Direct GPU framebuffer screenshot capture to PNG, JPEG, or WebP (`wallctl screenshot`).
  - Strict manifest validator and linter (`wallctl validate`).

- **Desktop Environment Integration Helpers (`contrib/`)**:
  - `contrib/kde-auto-pause.sh`: Dynamic KWin 6 ECMAScript tracking window states (`maximizeMode`, `fullScreen`, `hiddenByShowDesktop`) in real time over D-Bus.
  - `contrib/hyprland-auto-pause.sh`: Hyprland IPC `socket2.sock` event listener with transparency-aware window class filtering (`TRANSPARENT_CLASSES`).

- **Packaging & Deployment**:
  - Systemd user service unit (`extra/systemd/wallrsd.service`).
  - Debian package generation (`cargo-deb` / `make deb`).
  - Fedora / RHEL RPM spec (`extra/rpm/wallrs.spec`).
  - Arch Linux PKGBUILD (`extra/arch/PKGBUILD`).
  - Semantic versioning bump script (`scripts/bump-version.py`).