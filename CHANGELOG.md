# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Dynamic HiDPI Scaling on Wayland (`wallrs-core`)**: Implemented `CompositorHandler::scale_factor_changed` and `OutputHandler` scale factor tracking. Surfaces dynamically call `wl_surface::set_buffer_scale`, recompute physical framebuffer dimensions ($\text{Physical} = \text{Logical} \times \text{Scale}$), and reconfigure WGPU swapchains and renderer viewports on scale factor changes without requiring daemon restarts.
- **HiDPI-Aware Pointer Coordinate Normalization**: Separated `logical_width` and `logical_height` from physical framebuffer dimensions on `OutputSurface`, ensuring cursor coordinates from Wayland pointer motion events are normalized accurately across $[-1.0, 1.0]$ on high-density displays (e.g. 4K at 2x) without restricting cursor parallax to the top-left quadrant.

## [1.2.0] - 2026-09-16

### Added
- **Dynamic Named Shader Uniforms**: Authors can now define arbitrary property names in `[shader.uniforms]` (e.g. `speed = 1.0`, `glow = 0.5`) and specify optional slot ordering via `[shader] uniform_mapping`. `wallrs` auto-injects helper functions directly into WGSL (e.g. `fn speed() -> f32`), enabling seamless shader access. Properties can be tuned at runtime via `wallctl set-property <name> <val>`.
- **Shader Framerate Ceiling (`fps` / `target_fps`)**: Added configurable `fps = <u32>` ceiling to `[shader]` in `wallpaper.toml` (defaults to 60 FPS ceiling, `0` for uncapped), preventing GPU and thermal waste on 144Hz–240Hz displays. Can also be adjusted on the fly with `wallctl set-property fps <num>`.
- **Unified Color Parser (`wallrs-proto`)**: Centralized `wallrs_proto::parse_color` across `wallrs-cli` and `wallrs-daemon`, with support for `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, bare hex values, and comma-separated floats `r,g,b[,a]`.
- **Dedicated IPC `Mute` / `Unmute` Commands**: Added explicit `Command::Mute` and `Command::Unmute` socket messages to control audio output deterministically without manual boolean toggles.
- **Atomic `--unmute` on Set Wallpaper**: Integrated `unmute: bool` directly into `Command::SetWallpaper`. Applying a wallpaper with `wallctl set <path> --unmute` now executes atomically in a single IPC transaction without a second socket call.

### Changed
- **Extended Shader Uniform Buffer (224 bytes)**: Appended `custom_extra: array<vec4<f32>, 2>` (slots 3 to 10) to the standard uniform layout at offset 192, ensuring 100% backward compatibility with 192-byte shaders while expanding custom uniform capacity to 11 float parameters.

## [1.1.4] - 2026-09-11

### Performance
- **Parallax Settling & Idle 0.0% CPU**: Added idle settling detection in `ImageRenderer` when cursor stops moving (`< 1e-4` threshold). Once settled, compositor frame callbacks are suspended, dropping CPU usage to **0.0%**.
- **Framerate Capping (`target_fps`)**: Enforced a default 60 FPS target for animated image wallpapers and added configurable `fps = <u32>` under `[image]` to prevent running uncapped at 144Hz–240Hz on high-refresh monitors.
- **Reactive On-Demand Frame Wakeup**: Pointer motion (`PointerEventKind::Motion`) and surface enter/leave in `EngineState` now immediately wake up the render loop via `out.request_frame(qh)` when pointer-reactive renderers are present.
- **Frame Callback Deduplication**: Added `frame_pending` state to `OutputSurface` to prevent duplicate Wayland frame callback requests caused by high-polling-rate mice (e.g. 1000Hz).
- **GPU Uniform Caching**: Cached raw 32-byte layer uniform representations (`last_uniform_bytes`) in `LoadedLayer` to eliminate redundant PCIe buffer transfers via `queue.write_buffer` and skip unneeded render passes (`is_dirty`).
- **Low-Frequency Day/Night Ticking**: Decoupled day/night cycle progression from high-frequency frame rendering via `WallpaperRenderer::wants_periodic_tick()`. When no continuous animations are active, the event loop triggers lighting updates at 1 Hz instead of 144 Hz.

### Fixed
- **Parallax Landscape Example**: Removed vertical oscillation on foreground mountain layer (`fg.png`) in `examples/parallax-landscape/wallpaper.toml` so mountains remain grounded, and configured `[image] fps = 60`.
- **Fallback Shader Uniform Layout**: Aligned `UNIVERSAL_FALLBACK_WGSL` in `wallrs-cli` with the standard 192-byte `ShaderUniforms` layout (with `resolution` at offset 0 and `time` at offset 8), resolving an issue where fallback shaders failed to animate over time.
- **Background Audio Initial Leak Guard**: Initialized `BackgroundAudioPlayer` in a strictly muted state with `ao = "null"` before starting playback, eliminating transient audio bursts or pops upon wallpaper load when audio is disabled.
- **Atomic Session State Persistence**: Implemented genuine atomic state saving in `wallrs-core` using temporary files and filesystem rename, preventing potential file corruption on abrupt daemon termination.
- **Event Loop Periodic Timer Drift**: Dynamically calculated remaining timeout before next tick via `saturating_sub` in `Engine::run()`, guaranteeing accurate 1.00 Hz day/night updates even when interim Wayland/pointer events wake the dispatch loop.
- **Default Video Volume Harmonization**: Aligned CLI validator and scaffolding defaults to 0.0% (muted by default), matching `VideoRenderer` behavior.
- **Daemon Shorthand Hex Parsing**: Added support for 3-digit (`#RGB`) and 4-digit (`#RGBA`) hex color formats to `wallrsd --color`, matching `wallctl`.

## [1.1.3] - 2026-09-10

### Added
- **Dedicated Day/Night Sky Layers**: Replaced legacy `bg.png` in `examples/parallax-landscape` with procedural `sky-day.png` and `sky-night.png` crossfading smoothly at dawn/dusk.
- **Day/Night Schedule & Tint Curve Overrides**: Added `[image.day_night]` configuration to `wallrs-proto` and `ImageRenderer` allowing custom dawn/day/dusk/night times and customizable 24-hour ambient tint curves.
- **Interactive Runtime Lighting Controls**: Extended `wallctl set-property` to support simulated hour (`hour "14:30"` or decimal float), daylight factor (`daylight 0.8`), and ambient tint (`tint "#RRGGBB"` or `"r,g,b"`), with `"reset"` / `"auto"` keywords to restore live astronomical clock time.
- **Hyprland ML4W Dotfiles Integration**: Updated `contrib/wallrs-theme-sync.sh` and `ml4w-wallpaper` to detect ML4W and synchronize Quickshell, Waybar, and Matugen color themes on startup and wallpaper changes without hanging on `awww`.

## [1.1.2] - 2026-09-08

### Added
- **Native Wallpaper Scaffolding**: Added `wallctl new` (with alias `wallctl init`) to bootstrap custom wallpapers conforming to XDG Base Directory conventions and decoupled Single Responsibility Principle modules.
- **Formal Technical Authoring Guide**: Authored comprehensive documentation in `docs/authoring-guide.md` defining `wallpaper.toml` schemas, external tooling guidelines, and the exact 192-byte `ShaderUniforms` GPU memory layout.
- **Renderer Framerate Ceiling (`target_fps`)**: Added `target_fps(&self) -> Option<f64>` to `WallpaperRenderer` allowing procedural renderers or user constraints (`--max-fps`) to enforce framerate limits.
- **Dirty Frame Guard (`is_dirty`)**: Added `is_dirty(&self) -> bool` to `WallpaperRenderer` and skipped surface texture acquisition, render passes, and swapchain presentations in `OutputSurface` when the video decoder has not produced a new frame, delegating presentation timing and pulldown cadence directly to `libmpv`.

### Performance
- **Native Video Resolution Rendering**: Render size is matched to native video dimensions up to screen resolution, delegating presentation scaling to GPU hardware bilinear filtering and eliminating CPU software upscaling to 4K on high-DPI displays (up to 75% memory bandwidth reduction).
- **Idle Video Suspension (0.0% CPU on Pause/EOF)**: Implemented dynamic `is_animated()` lifecycle tracking for `VideoRenderer` to suspend Wayland frame callbacks on pause or non-looping EOF.

### Fixed
- **Audio Leak on Initial Video Load**: Fixed a race condition where videos with audio would briefly play sound upon loading before being muted by the daemon. MPV is now initialized in a strictly muted state with `ao = "null"`, only enabling audio threads when unmuted explicitly or by policy.

### Removed
- **Redundant Example Packages**: Removed `examples/video-with-sound` and `examples/parallax-with-audio`, consolidating `examples/video-wallpaper` (with volume configurable and muted by default) and `examples/parallax-landscape` to reduce repository and package weight.

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