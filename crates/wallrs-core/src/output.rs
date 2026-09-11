use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    shell::{WaylandSurface, wlr_layer::LayerSurface},
};
use std::ptr::NonNull;
use std::time::{Duration, Instant};
use thiserror::Error;
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{wl_compositor, wl_output},
};

use crate::engine::EngineState;
use wallrs_render::{FrameContext, WallpaperRenderer};

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("Failed to create wgpu surface: {0}")]
    SurfaceCreation(String),

    #[error("Failed to initialize wallpaper renderer: {0}")]
    RendererInit(String),

    #[error("Renderer error: {0}")]
    Renderer(String),
}

/// Represents an active Wayland output and its associated background layer surface + wgpu pipeline.
pub struct OutputSurface {
    pub name: Option<String>,
    pub wl_output: wl_output::WlOutput,
    pub layer_surface: LayerSurface,
    pub wgpu_surface: Option<wgpu::Surface<'static>>,
    pub surface_config: Option<wgpu::SurfaceConfiguration>,
    pub renderer: Option<Box<dyn WallpaperRenderer>>,
    pub width: u32,
    pub height: u32,
    pub configured: bool,
    pub manual_paused: bool,
    pub fullscreen_paused: bool,
    pub frame_pending: bool,
    pub max_fps: Option<u32>,
    pub start_time: Instant,
    pub last_frame_time: Option<Instant>,
    pub last_rendered_frame_time: Option<Instant>,
    pub cursor_position: Option<(f32, f32)>,
    pub audio_handle: Option<wallrs_audio::SpectrumHandle>,
    pub audio_track: Option<wallrs_audio::BackgroundAudioPlayer>,
    pub audio_muted: bool,
    pub current_wallpaper: Option<std::path::PathBuf>,
}

/// Bundles WGPU rendering context references passed to output configuration.
pub struct GpuContext<'a> {
    pub instance: &'a wgpu::Instance,
    pub adapter: &'a wgpu::Adapter,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

impl OutputSurface {
    pub fn new(
        name: Option<String>,
        wl_output: wl_output::WlOutput,
        layer_surface: LayerSurface,
        max_fps: Option<u32>,
    ) -> Self {
        Self {
            name,
            wl_output,
            layer_surface,
            wgpu_surface: None,
            surface_config: None,
            renderer: None,
            width: 0,
            height: 0,
            configured: false,
            manual_paused: false,
            fullscreen_paused: false,
            frame_pending: false,
            max_fps,
            start_time: Instant::now(),
            last_frame_time: None,
            last_rendered_frame_time: None,
            cursor_position: None,
            audio_handle: None,
            audio_track: None,
            audio_muted: true,
            current_wallpaper: None,
        }
    }

    /// Returns true if this output is paused either manually or by a fullscreen window.
    pub fn is_paused(&self) -> bool {
        self.manual_paused || self.fullscreen_paused
    }

    /// Handles compositor configure event.
    /// Strictly defers wgpu::Surface creation until the first configure is received.
    pub fn handle_configure(
        &mut self,
        new_size: (u32, u32),
        gpu: &GpuContext<'_>,
        conn: &Connection,
        qh: &QueueHandle<EngineState>,
        renderer_factory: &dyn Fn() -> Box<dyn WallpaperRenderer>,
        compositor: &wl_compositor::WlCompositor,
    ) -> Result<(), OutputError> {
        let width = if new_size.0 > 0 { new_size.0 } else { 1920 };
        let height = if new_size.1 > 0 { new_size.1 } else { 1080 };

        if !self.configured {
            tracing::info!(
                output = ?self.name,
                width = width,
                height = height,
                "First configure received for output; creating wgpu surface"
            );

            let display_ptr = conn.backend().display_ptr() as *mut _;
            let surface_ptr = self.layer_surface.wl_surface().id().as_ptr() as *mut _;

            let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                NonNull::new(display_ptr)
                    .ok_or_else(|| OutputError::SurfaceCreation("Null display pointer".into()))?,
            ));
            let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
                NonNull::new(surface_ptr)
                    .ok_or_else(|| OutputError::SurfaceCreation("Null surface pointer".into()))?,
            ));

            let wgpu_surface = unsafe {
                gpu.instance
                    .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(raw_display_handle),
                        raw_window_handle,
                    })
                    .map_err(|e| OutputError::SurfaceCreation(e.to_string()))?
            };

            let mut config = wgpu_surface
                .get_default_config(gpu.adapter, width, height)
                .ok_or_else(|| {
                    OutputError::SurfaceCreation("No supported surface configuration".into())
                })?;

            config.present_mode = wgpu::PresentMode::Mailbox;
            wgpu_surface.configure(gpu.device, &config);

            let mut renderer = renderer_factory();
            renderer
                .init(gpu.device, gpu.queue, config.format)
                .map_err(|e| OutputError::RendererInit(e.to_string()))?;
            renderer.resize(width, height);

            self.wgpu_surface = Some(wgpu_surface);
            self.surface_config = Some(config);
            self.renderer = Some(renderer);
            self.width = width;
            self.height = height;
            self.configured = true;

            self.update_input_region(compositor, qh);

            // Immediately render the first frame to attach buffer, map surface in compositor, and kick off frame loop
            self.render_frame(gpu.device, gpu.queue, qh);
        } else if self.width != width || self.height != height {
            tracing::info!(
                output = ?self.name,
                old_width = self.width,
                old_height = self.height,
                new_width = width,
                new_height = height,
                "Output resized; reconfiguring wgpu surface"
            );

            self.width = width;
            self.height = height;

            if let (Some(surface), Some(config)) = (&self.wgpu_surface, &mut self.surface_config) {
                config.width = width;
                config.height = height;
                surface.configure(gpu.device, config);
            }

            if let Some(renderer) = &mut self.renderer {
                renderer.resize(width, height);
            }
        }

        Ok(())
    }

    /// Renders a single frame and requests the next callback from the compositor.
    pub fn render_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        qh: &QueueHandle<EngineState>,
    ) {
        self.frame_pending = false;

        if !self.configured {
            return;
        }

        let is_initial_frame = self.last_rendered_frame_time.is_none();
        if self.is_paused() && !is_initial_frame {
            return;
        }

        let (Some(surface), Some(renderer)) = (&self.wgpu_surface, &mut self.renderer) else {
            return;
        };

        let now = Instant::now();

        // Enforce effective FPS limit by combining user max_fps with renderer target_fps
        let effective_fps = match (self.max_fps.map(|f| f as f64), renderer.target_fps()) {
            (Some(max), Some(target)) => Some(max.min(target)),
            (Some(max), None) => Some(max),
            (None, Some(target)) => Some(target),
            (None, None) => None,
        };

        if let Some(fps) = effective_fps
            && fps > 0.0
            && let Some(last_render) = self.last_rendered_frame_time
        {
            let min_interval = Duration::from_secs_f64(1.0 / fps);
            if now.duration_since(last_render) < min_interval {
                // Skip render and re-register frame callback for next compositor vblank
                if !self.frame_pending {
                    self.layer_surface.wl_surface().frame(
                        qh,
                        FrameCallbackData(self.layer_surface.wl_surface().clone()),
                    );
                    self.layer_surface.commit();
                    self.frame_pending = true;
                }
                return;
            }
        }

        let elapsed = now.duration_since(self.start_time);
        let delta = self
            .last_frame_time
            .map_or(Duration::from_millis(16), |last| now.duration_since(last));
        self.last_frame_time = Some(now);

        let spectrum_arc = self.audio_handle.as_ref().map(|h| h.latest());
        let spectrum = spectrum_arc.as_deref().map(|v| v.as_slice());

        let ctx = FrameContext {
            elapsed,
            delta,
            output_size: (self.width, self.height),
            pointer: self.cursor_position,
            spectrum,
            device,
            queue,
        };

        // Update renderer state before acquiring swapchain texture
        let update_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.update(&ctx);
        }));

        if let Err(_panic_payload) = update_res {
            tracing::error!(
                output = ?self.name,
                "Panic occurred while updating output renderer. Pausing output."
            );
            self.manual_paused = true;
            return;
        }

        // If the renderer has no new frame content (e.g. video decoder has not produced a new frame),
        // skip swapchain texture acquisition, render pass encoding, and queue presentation.
        if !is_initial_frame && !renderer.is_dirty() {
            let is_animated = renderer.is_animated();
            if is_animated && !self.is_paused() && !self.frame_pending {
                self.layer_surface.wl_surface().frame(
                    qh,
                    FrameCallbackData(self.layer_surface.wl_surface().clone()),
                );
                self.frame_pending = true;
            }
            self.layer_surface.commit();
            return;
        }

        let surface_texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                if let Some(config) = &self.surface_config {
                    surface.configure(device, config);
                }
                if !self.frame_pending {
                    self.layer_surface.wl_surface().frame(
                        qh,
                        FrameCallbackData(self.layer_surface.wl_surface().clone()),
                    );
                    self.layer_surface.commit();
                    self.frame_pending = true;
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                if !self.frame_pending {
                    self.layer_surface.wl_surface().frame(
                        qh,
                        FrameCallbackData(self.layer_surface.wl_surface().clone()),
                    );
                    self.layer_surface.commit();
                    self.frame_pending = true;
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!(output = ?self.name, "Surface validation error; pausing output");
                self.manual_paused = true;
                return;
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("output_frame_encoder"),
        });

        // Enforce output fault isolation with catch_unwind
        let render_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.render(&mut encoder, &view);
        }));

        if let Err(_panic_payload) = render_res {
            tracing::error!(
                output = ?self.name,
                "Panic occurred while rendering output. Pausing this output to preserve daemon stability."
            );
            self.manual_paused = true;
            return;
        }

        queue.submit(Some(encoder.finish()));
        queue.present(surface_texture);
        self.last_rendered_frame_time = Some(now);

        // Only request next frame callback if not currently paused and renderer is animated.
        // Static wallpapers (solid colors, static images) render their initial frame once
        // and stop requesting callbacks, dropping idle CPU and GPU usage to 0.0%.
        let is_animated = renderer.is_animated();
        if is_animated && !self.is_paused() && !self.frame_pending {
            self.layer_surface.wl_surface().frame(
                qh,
                FrameCallbackData(self.layer_surface.wl_surface().clone()),
            );
            self.frame_pending = true;
        }
        self.layer_surface.commit();
    }

    /// Requests a single frame callback from the compositor if configured and not paused.
    pub fn request_frame(&mut self, qh: &QueueHandle<EngineState>) {
        if self.configured && !self.is_paused() && !self.frame_pending {
            self.layer_surface.wl_surface().frame(
                qh,
                FrameCallbackData(self.layer_surface.wl_surface().clone()),
            );
            self.layer_surface.commit();
            self.frame_pending = true;
        }
    }

    /// Replaces the active wallpaper renderer on this output.
    pub fn set_renderer(
        &mut self,
        mut renderer: Box<dyn WallpaperRenderer>,
        gpu: &GpuContext<'_>,
        qh: &QueueHandle<EngineState>,
        compositor: &wl_compositor::WlCompositor,
    ) -> Result<(), OutputError> {
        if let Some(config) = &self.surface_config {
            renderer
                .init(gpu.device, gpu.queue, config.format)
                .map_err(|e| OutputError::RendererInit(e.to_string()))?;
            renderer.resize(self.width, self.height);
        }
        if let Some(mut old) = self.renderer.take() {
            old.teardown();
        }
        self.renderer = Some(renderer);
        self.start_time = Instant::now();
        self.last_frame_time = None;
        self.last_rendered_frame_time = None;
        self.update_input_region(compositor, qh);
        if self.configured {
            self.render_frame(gpu.device, gpu.queue, qh);
        }
        if self.is_paused()
            && let Some(r) = &mut self.renderer
        {
            let _ = r.set_property("pause", wallrs_proto::PropertyValue::Bool(true));
        }
        Ok(())
    }

    /// Updates the Wayland surface input region based on whether the active wallpaper requires pointer interaction.
    ///
    /// When `wants_pointer` is false, an empty `WlRegion` is applied so all pointer events and clicks
    /// pass through to the underlying desktop (e.g., KDE Plasma's desktop containment, desktop icons,
    /// and popup grab dismissal for application launchers like Kickoff).
    pub fn update_input_region(
        &mut self,
        compositor: &wl_compositor::WlCompositor,
        qh: &QueueHandle<EngineState>,
    ) {
        let wants_pointer = self
            .renderer
            .as_ref()
            .map(|r| r.wants_pointer())
            .unwrap_or(false);

        if wants_pointer {
            self.layer_surface.wl_surface().set_input_region(None);
        } else {
            let region = compositor.create_region(qh, ());
            self.layer_surface
                .wl_surface()
                .set_input_region(Some(&region));
            region.destroy();
        }
        self.layer_surface.commit();
    }

    /// Sets a dynamic property on the active wallpaper renderer.
    pub fn set_property(
        &mut self,
        key: &str,
        value: wallrs_proto::PropertyValue,
        qh: &QueueHandle<EngineState>,
    ) -> Result<(), OutputError> {
        if key == "mute"
            && let wallrs_proto::PropertyValue::Bool(b) = value
        {
            self.set_muted(b);
            return Ok(());
        }

        if key == "volume" {
            let mut handled = false;
            if let Some(renderer) = &mut self.renderer
                && renderer.set_property(key, value.clone()).is_ok()
            {
                handled = true;
            }
            if let Some(player) = &mut self.audio_track
                && player.set_property(key, value.clone()).is_ok()
            {
                handled = true;
            }
            if handled {
                return Ok(());
            }
        }

        if let Some(renderer) = &mut self.renderer {
            renderer
                .set_property(key, value.clone())
                .map_err(|e| OutputError::Renderer(e.to_string()))?;
        }
        if let Some(player) = &mut self.audio_track {
            let _ = player.set_property(key, value);
        }
        self.request_frame(qh);
        Ok(())
    }

    /// Sets the muted state for this output's renderer and background audio track.
    pub fn set_muted(&mut self, muted: bool) {
        self.audio_muted = muted;
        if let Some(renderer) = &mut self.renderer {
            let _ = renderer.set_property("mute", wallrs_proto::PropertyValue::Bool(muted));
        }
        if let Some(player) = &mut self.audio_track {
            let _ = player.set_property("mute", wallrs_proto::PropertyValue::Bool(muted));
        }
    }

    /// Toggles the muted state of this output and returns the new state.
    pub fn toggle_mute(&mut self) -> bool {
        let new_state = !self.audio_muted;
        self.set_muted(new_state);
        new_state
    }

    /// Sets the manual paused state of this output (e.g., from `wallctl pause`).
    /// If resuming from a paused state, commits a new frame callback to wake up rendering.
    pub fn set_manual_paused(&mut self, paused: bool, qh: &QueueHandle<EngineState>) {
        let was_paused = self.is_paused();
        self.manual_paused = paused;
        let is_paused = self.is_paused();
        if was_paused != is_paused {
            if let Some(renderer) = &mut self.renderer {
                let _ =
                    renderer.set_property("pause", wallrs_proto::PropertyValue::Bool(is_paused));
            }
            if let Some(player) = &mut self.audio_track {
                player.set_paused(is_paused);
            }
        }
        if was_paused && !is_paused {
            self.request_frame(qh);
        }
    }

    /// Sets the fullscreen paused state of this output (triggered automatically when a window becomes fullscreen).
    pub fn set_fullscreen_paused(&mut self, paused: bool, qh: &QueueHandle<EngineState>) {
        let was_paused = self.is_paused();
        self.fullscreen_paused = paused;
        let is_paused = self.is_paused();
        if was_paused != is_paused {
            if let Some(renderer) = &mut self.renderer {
                let _ =
                    renderer.set_property("pause", wallrs_proto::PropertyValue::Bool(is_paused));
            }
            if let Some(player) = &mut self.audio_track {
                player.set_paused(is_paused);
            }
        }
        if was_paused && !is_paused {
            self.request_frame(qh);
        }
    }

    /// Convenience wrapper for manual pause.
    pub fn set_paused(&mut self, paused: bool, qh: &QueueHandle<EngineState>) {
        self.set_manual_paused(paused, qh);
    }

    /// Teardown when output is unplugged/destroyed.
    pub fn teardown(&mut self) {
        if let Some(mut renderer) = self.renderer.take() {
            renderer.teardown();
        }
        self.audio_track = None;
        self.audio_handle = None;
        self.wgpu_surface = None;
        self.surface_config = None;
    }
}

#[cfg(test)]
mod tests {
    use wallrs_render::{FrameContext, RendererError, WallpaperRenderer};

    struct MockTestRenderer {
        target_fps: Option<f64>,
        dirty: bool,
        animated: bool,
    }

    impl WallpaperRenderer for MockTestRenderer {
        fn init(
            &mut self,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _target_format: wgpu::TextureFormat,
        ) -> Result<(), RendererError> {
            Ok(())
        }
        fn resize(&mut self, _width: u32, _height: u32) {}
        fn update(&mut self, _ctx: &FrameContext) {}
        fn render(&mut self, _encoder: &mut wgpu::CommandEncoder, _view: &wgpu::TextureView) {
            self.dirty = false;
        }
        fn target_fps(&self) -> Option<f64> {
            self.target_fps
        }
        fn is_dirty(&self) -> bool {
            self.dirty
        }
        fn is_animated(&self) -> bool {
            self.animated
        }
    }

    #[test]
    fn test_renderer_target_fps_and_dirty_contract() {
        let mut r = MockTestRenderer {
            target_fps: Some(30.0),
            dirty: true,
            animated: true,
        };

        assert_eq!(r.target_fps(), Some(30.0));
        assert!(r.is_dirty());
        assert!(r.is_animated());

        r.dirty = false;
        assert!(!r.is_dirty());
    }

    #[test]
    fn test_effective_fps_computation() {
        let max_fps = Some(60u32);
        let target_fps = Some(30.0);

        let effective = match (max_fps.map(|f| f as f64), target_fps) {
            (Some(max), Some(target)) => Some(max.min(target)),
            (Some(max), None) => Some(max),
            (None, Some(target)) => Some(target),
            (None, None) => None,
        };
        assert_eq!(effective, Some(30.0));

        let max_fps = Some(24u32);
        let target_fps = Some(60.0);
        let effective = match (max_fps.map(|f| f as f64), target_fps) {
            (Some(max), Some(target)) => Some(max.min(target)),
            (Some(max), None) => Some(max),
            (None, Some(target)) => Some(target),
            (None, None) => None,
        };
        assert_eq!(effective, Some(24.0));
    }
}
