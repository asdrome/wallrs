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
use wayland_client::{Connection, Proxy, QueueHandle, protocol::wl_output};

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
    pub paused: bool,
    pub start_time: Instant,
    pub last_frame_time: Option<Instant>,
    pub cursor_position: Option<(f32, f32)>,
    pub audio_handle: Option<wallrs_audio::SpectrumHandle>,
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
            paused: false,
            start_time: Instant::now(),
            last_frame_time: None,
            cursor_position: None,
            audio_handle: None,
        }
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
        if self.paused || !self.configured {
            return;
        }

        let (Some(surface), Some(renderer)) = (&self.wgpu_surface, &mut self.renderer) else {
            return;
        };

        let surface_texture = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                if let Some(config) = &self.surface_config {
                    surface.configure(device, config);
                }
                self.layer_surface.wl_surface().frame(
                    qh,
                    FrameCallbackData(self.layer_surface.wl_surface().clone()),
                );
                self.layer_surface.commit();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.layer_surface.wl_surface().frame(
                    qh,
                    FrameCallbackData(self.layer_surface.wl_surface().clone()),
                );
                self.layer_surface.commit();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                tracing::error!(output = ?self.name, "Surface validation error; pausing output");
                self.paused = true;
                return;
            }
        };

        let now = Instant::now();
        let elapsed = now.duration_since(self.start_time);
        let delta = self
            .last_frame_time
            .map_or(Duration::from_millis(16), |last| now.duration_since(last));
        self.last_frame_time = Some(now);

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("output_frame_encoder"),
        });

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

        // Enforce output fault isolation with catch_unwind
        let render_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.update(&ctx);
            renderer.render(&mut encoder, &view);
        }));

        if let Err(_panic_payload) = render_res {
            tracing::error!(
                output = ?self.name,
                "Panic occurred while rendering output. Pausing this output to preserve daemon stability."
            );
            self.paused = true;
            return;
        }

        queue.submit(Some(encoder.finish()));
        queue.present(surface_texture);

        // Request next frame callback to adhere to display refresh rate
        self.layer_surface.wl_surface().frame(
            qh,
            FrameCallbackData(self.layer_surface.wl_surface().clone()),
        );
        self.layer_surface.commit();
    }

    /// Replaces the active wallpaper renderer on this output.
    pub fn set_renderer(
        &mut self,
        mut renderer: Box<dyn WallpaperRenderer>,
        gpu: &GpuContext<'_>,
        qh: &QueueHandle<EngineState>,
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
        if self.configured && !self.paused {
            self.render_frame(gpu.device, gpu.queue, qh);
        }
        Ok(())
    }

    /// Sets a dynamic property on the active wallpaper renderer.
    pub fn set_property(
        &mut self,
        key: &str,
        value: wallrs_proto::PropertyValue,
        qh: &QueueHandle<EngineState>,
    ) -> Result<(), OutputError> {
        if let Some(renderer) = &mut self.renderer {
            renderer
                .set_property(key, value)
                .map_err(|e| OutputError::Renderer(e.to_string()))?;
        }
        if self.configured && !self.paused {
            self.layer_surface.wl_surface().frame(
                qh,
                FrameCallbackData(self.layer_surface.wl_surface().clone()),
            );
            self.layer_surface.commit();
        }
        Ok(())
    }

    /// Sets the paused state of this output.
    /// If resuming from a paused state, commits a new frame callback to wake up rendering.
    pub fn set_paused(&mut self, paused: bool, qh: &QueueHandle<EngineState>) {
        let was_paused = self.paused;
        self.paused = paused;
        if was_paused && !paused && self.configured {
            self.layer_surface.wl_surface().frame(
                qh,
                FrameCallbackData(self.layer_surface.wl_surface().clone()),
            );
            self.layer_surface.commit();
        }
    }

    /// Teardown when output is unplugged/destroyed.
    pub fn teardown(&mut self) {
        if let Some(mut renderer) = self.renderer.take() {
            renderer.teardown();
        }
        self.wgpu_surface = None;
        self.surface_config = None;
    }
}
