# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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