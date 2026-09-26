# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.2.0] - 2026-09-25

### Added
- **GStreamer Video Engine (`wallrs-content-video`)**: Migrated the video wallpaper engine from `libmpv` to GStreamer using `playbin` and `appsink`. Direct zero-copy GPU texture uploads (`t_y` and `t_uv`) in NV12 format with shader color conversion, completely eliminating intermediate CPU RGBA conversions.
- **Dynamic Colorimetry & Double-Gamma Compensation (`wallrs-content-video`)**: Automatic detection of video color range (Full 0-255 vs Studio 16-235) and color matrix (ITU-R BT.709 vs BT.601) via GStreamer `VideoInfo`. Injects $R'G'B' \to \text{linear}$ conversion when rendering to sRGB surfaces, eliminating the washed-out milky look and preserving original video contrast 1:1 with fail-safe fallback.
- **Intelligent Hardware Postprocessor Injection (`vapostproc`)**: Dynamic detection of VA-API decoders via `element-setup` and hardware context verification (`/dev/dri/renderD128`). NVIDIA (`nvdec`) and software (`avdec`) decoders bypass `vapostproc` cleanly, preventing pipeline negotiation errors and PCIe bandwidth waste.
- **GStreamer Ambient Audio Engine (`wallrs-audio`)**: Replaced `libmpv` in `BackgroundAudioPlayer` with a dedicated, headless GStreamer audio pipeline utilizing `pipewiresink` (with fallback to `autoaudiosink`), native PipeWire integration, and lean thread model.
- **Complete Retirement of `libmpv`**: Fully removed `libmpv2` and `libmpv2-sys` from the workspace. Updated Arch Linux PKGBUILD, Fedora RPM spec, Debian package definitions, and documentation to reflect modern GStreamer dependencies (`gstreamer1`, `gst-plugins-base`, `gst-plugins-good`).
- **Plugin Architecture & Dynamic Factory Registry (`wallrs-render` & `wallrs-core`)**: Fully decoupled `wallrs-core` from direct dependencies on concrete content renderers (`wallrs-content-image`, `wallrs-content-shader`, `wallrs-content-video`). Introduced `RendererFactory`, `RendererRegistry`, and `RendererCapabilities` in `wallrs-render`, enabling dynamic wallpaper renderer registration and inversion of control.
- **Content Renderer Factories**: Implemented `ImageRendererFactory`, `ShaderRendererFactory`, and `VideoRendererFactory` in their respective crates, encapsulating asset validation and instantiation without early GPU resource allocation.
- **Dynamic Dependency Injection in Daemon (`wallrsd`)**: Configured `wallrs-daemon` to register all content factories into `RendererRegistry` at startup and inject the registry into `EngineConfig`.
- **Unified Engine & IPC Dispatch Pipeline**: Replaced hardcoded type-switching branches in `wallrs-core/src/ipc.rs` (`apply_wallpaper` and `validate_wallpaper_manifest`) with a single unified, factory-driven lookup and construction loop.
- **Unified Audio Lifecycle Controller (`AudioController`)**: Consolidated background audio playback, volume management, mute/unmute state, pause dispatch, and PipeWire spectrum attachment into `AudioController` on `OutputSurface`.
- **Decoupled CLI Dependency Tree**: Removed unused content render dependencies from `wallrs-cli` (`wallctl`).
- **Dynamic HiDPI Scaling on Wayland (`wallrs-core`)**: Implemented `CompositorHandler::scale_factor_changed` and `OutputHandler` scale factor tracking. Surfaces dynamically call `wl_surface::set_buffer_scale`, recompute physical framebuffer dimensions ($\text{Physical} = \text{Logical} \times \text{Scale}$), and reconfigure WGPU swapchains and renderer viewports on scale factor changes without requiring daemon restarts.
- **HiDPI-Aware Pointer Coordinate Normalization**: Separated `logical_width` and `logical_height` from physical framebuffer dimensions on `OutputSurface`, ensuring cursor coordinates from Wayland pointer motion events are normalized accurately across $[-1.0, 1.0]$ on high-density displays (e.g. 4K at 2x) without restricting cursor parallax to the top-left quadrant.
- **Dynamic Named Shader Uniforms**: Authors can now define arbitrary property names in `[shader.uniforms]` (e.g. `speed = 1.0`, `glow = 0.5`) and specify optional slot ordering via `[shader] uniform_mapping`. `wallrs` auto-injects helper functions directly into WGSL (e.g. `fn speed() -> f32`), enabling seamless shader access. Properties can be tuned at runtime via `wallctl set-property <name> <val>`.
- **Shader Framerate Ceiling (`fps` / `target_fps`)**: Added configurable `fps = <u32>` ceiling to `[shader]` in `wallpaper.toml` (defaults to 60 FPS ceiling, `0` for uncapped), preventing GPU and thermal waste on 144Hz–240Hz displays. Can also be adjusted on the fly with `wallctl set-property fps <num>`.
- **Unified Color Parser (`wallrs-proto`)**: Centralized `wallrs_proto::parse_color` across `wallrs-cli` and `wallrs-daemon`, with support for `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, bare hex values, and comma-separated floats `r,g,b[,a]`.
- **Niri Auto-Pause Driver (`contrib/niri-auto-pause.sh`)**: Added dedicated auto-pause integration for Niri compositors via Niri's IPC event stream (`niri msg --json event-stream`). Tracks focused workspaces, pauses rendering when opaque windows are active, and automatically resumes when workspaces are empty, occupied only by transparent windows, or when Niri overview mode is toggled open.
- **Niri Compositor Detection in Dispatcher (`contrib/wallrs-auto-pause.sh`)**: Added native detection for Niri sessions via `$NIRI_SOCKET` and `$XDG_CURRENT_DESKTOP`. Seamlessly launches `niri-auto-pause.sh` when available or idles gracefully with native `zwlr_foreign_toplevel_manager_v1` support, preventing premature systemd service termination.
- **Niri & Noctalia Theme Synchronization (`contrib/wallrs-theme-sync.sh`)**: Added native detection for Niri sessions and automatic Material You palette generation via Noctalia (`noctalia theme --builtin-config` and `noctalia msg templates-apply`), synchronizing wallpaper accent colors in real time across Niri window borders (`noctalia.kdl`), Noctalia UI, Kitty, and GTK.
- **Dedicated IPC `Mute` / `Unmute` Commands**: Added explicit `Command::Mute` and `Command::Unmute` socket messages to control audio output deterministically without manual boolean toggles.
- **Atomic `--unmute` on Set Wallpaper**: Integrated `unmute: bool` directly into `Command::SetWallpaper`. Applying a wallpaper with `wallctl set <path> --unmute` now executes atomically in a single IPC transaction without a second socket call.

### Fixed
- **Clean Daemon Shutdown & Segfault Elimination (`wallrs-core`)**: Resolved a segmentation fault on daemon shutdown (`wallctl kill`) caused by Mesa Vulkan Wayland WSI attempting to destroy swapchain buffers on a prematurely dropped `wl_display`. Reordered `Engine` so `Connection` is dropped strictly last, ordered `wgpu_surface` before `layer_surface` in `OutputSurface`, added GPU pipeline drain via `device.poll(Maintain::wait_indefinitely())`, and added explicit surface teardown and connection flushing. Eliminates the systemd service restart loop on graceful kill.
- **Theme Synchronization on State Restore (`wallrs-core` & `contrib/wallrs-theme-sync.sh`)**: Added automatic re-saving of the session state file in `try_restore_output_state` upon daemon startup, ensuring inotify / `wallrs-theme-sync.path` triggers theme synchronization immediately after restoring the previous session's wallpaper. Added preview retry polling in `wallrs-theme-sync.sh` to safely await initial frame decode.
- **Video Renderer Initial Frame & Preroll Wait (`wallrs-content-video`)**: Implemented decoder sample and preroll wait in `VideoRenderer::update()` when textures are uninitialized or paused, ensuring instant frame extraction for `wallctl preview` and `wallctl screenshot` without producing blank or transparent images.
- **Noctalia Dynamic Theme Palette Synchronization (`contrib/wallrs-theme-sync.sh`)**: Updated Noctalia integration to query the active tonal scheme (`color-scheme-get`), persist the wallpaper path (`wallpaper-set`), and recompute the palette in memory using `color-scheme-set wallpaper <scheme>`, seamlessly synchronizing Niri window borders (`noctalia.kdl`), Noctalia shell, Kitty, and GTK.
- **`wallctl set` Command Alias (`wallrs-cli`)**: Added `wallctl set` as a direct alias for `wallctl wallpaper load`, streamlining CLI commands and ergonomics.
- **Screenshot Path Resolution (`wallctl screenshot`)**: Resolved relative destination paths against the CLI caller's `$PWD` and expanded `~` before sending IPC messages to `wallrsd`, fixing an issue where screenshots were saved relative to the daemon's working directory (`$HOME` or `/`) instead of the current terminal directory.
- **GitHub Actions CI/Release Node 24 Migration**: Upgraded runner actions across CI and Release workflows (`actions/checkout@v7`, `actions/upload-artifact@v7`, `actions/download-artifact@v7`, `actions/cache@v5`, `softprops/action-gh-release@v3`), fully resolving Node.js 20 deprecation warnings on GitHub Actions runners.

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