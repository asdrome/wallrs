use std::time::Duration;
use thiserror::Error;
pub use wallrs_proto::PropertyValue;

pub use wgpu;

/// Error returned by renderer operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RendererError {
    #[error("Initialization failed: {0}")]
    InitFailed(String),

    #[error("Render failed: {0}")]
    RenderFailed(String),

    #[error("Property not found: {0}")]
    PropertyNotFound(String),

    #[error("Invalid property value: {0}")]
    InvalidPropertyValue(String),
}

/// Context provided to the renderer on every frame.
#[derive(Debug, Clone)]
pub struct FrameContext<'a> {
    pub elapsed: Duration,
    pub delta: Duration,
    pub output_size: (u32, u32),
    pub pointer: Option<(f32, f32)>,
    pub spectrum: Option<&'a [f32]>,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// Core trait implemented by all wallpaper content types.
pub trait WallpaperRenderer: Send {
    /// Called when the output size changes or at startup.
    /// Called once when the wallpaper is loaded onto an output.
    fn init(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError>;

    /// Called when the output size changes (or at startup after configure).
    fn resize(&mut self, width: u32, height: u32);

    /// Called once per frame prior to rendering.
    /// Called once per frame, before render().
    fn update(&mut self, ctx: &FrameContext);

    /// Emits render commands targeting the output surface view.
    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView);

    /// Sets a dynamic property value at runtime.
    fn set_property(&mut self, _key: &str, _value: PropertyValue) -> Result<(), RendererError> {
        Ok(())
    }

    /// Returns whether this wallpaper is continuously animated.
    ///
    /// Static wallpapers (solid colors, static images without pan or parallax) return `false`.
    /// When `false`, the engine renders an initial frame and stops requesting frame callbacks,
    /// dropping idle CPU and GPU usage to 0.0%.
    fn is_animated(&self) -> bool {
        true
    }

    /// Returns the suggested target FPS for this renderer, if any.
    ///
    /// For example, video wallpapers can report their container or estimated video framerate (e.g. 24.0, 30.0, 60.0),
    /// preventing redundant render passes and GPU swapchain presentations on high refresh rate displays (144Hz, 240Hz).
    /// Defaults to `None` (rendering at display refresh rate or global max_fps ceiling).
    fn target_fps(&self) -> Option<f64> {
        None
    }

    /// Returns whether the renderer has new visual content that requires presenting a new frame.
    ///
    /// When `false`, the display engine can skip swapchain acquisition, command buffer submission,
    /// and presentation for the current frame callback, saving GPU and CPU cycles.
    /// Defaults to `true` (e.g., continuous procedural shaders or animated layers).
    fn is_dirty(&self) -> bool {
        true
    }

    /// Returns whether this wallpaper requires interactive pointer input (e.g., cursor parallax or mouse uniforms).
    ///
    /// When `false` (the default), the Wayland surface configures an empty input region (`WlRegion`),
    /// granting complete click-through pass-through to the underlying desktop.
    fn wants_pointer(&self) -> bool {
        false
    }

    /// Tears down any allocated resources.
    fn teardown(&mut self) {}
}

fn srgb_to_linear(c: f32) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        (c / 12.92) as f64
    } else {
        (((c + 0.055) / 1.055).powf(2.4)) as f64
    }
}

/// A simple renderer that fills the surface with a solid RGBA color.
#[derive(Debug, Clone)]
pub struct SolidColorRenderer {
    pub color: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub target_format: Option<wgpu::TextureFormat>,
}

impl SolidColorRenderer {
    pub fn new(color: [f32; 4]) -> Self {
        Self {
            color,
            width: 0,
            height: 0,
            target_format: None,
        }
    }
}

impl Default for SolidColorRenderer {
    fn default() -> Self {
        // Default to a modern dark slate tone (#0f172a)
        Self::new([0.059, 0.090, 0.165, 1.0])
    }
}

impl WallpaperRenderer for SolidColorRenderer {
    fn init(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError> {
        self.target_format = Some(target_format);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn update(&mut self, _ctx: &FrameContext) {}

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let is_srgb = self.target_format.is_none_or(|f| f.is_srgb());
        let clear_color = if is_srgb {
            wgpu::Color {
                r: srgb_to_linear(self.color[0]),
                g: srgb_to_linear(self.color[1]),
                b: srgb_to_linear(self.color[2]),
                a: self.color[3] as f64,
            }
        } else {
            wgpu::Color {
                r: self.color[0] as f64,
                g: self.color[1] as f64,
                b: self.color[2] as f64,
                a: self.color[3] as f64,
            }
        };

        let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("solid_color_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    fn set_property(&mut self, key: &str, value: PropertyValue) -> Result<(), RendererError> {
        if key == "color" {
            if let PropertyValue::Color(rgba) = value {
                self.color = rgba;
                return Ok(());
            }
            return Err(RendererError::InvalidPropertyValue(
                "expected PropertyValue::Color".to_string(),
            ));
        }
        Err(RendererError::PropertyNotFound(key.to_string()))
    }

    fn is_animated(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solid_color_renderer_properties() {
        let mut renderer = SolidColorRenderer::default();
        assert!(!renderer.is_animated());
        assert!(!renderer.wants_pointer());
        assert_eq!(renderer.color, [0.059, 0.090, 0.165, 1.0]);

        renderer.resize(1920, 1080);
        assert_eq!(renderer.width, 1920);
        assert_eq!(renderer.height, 1080);

        let new_color = [1.0, 0.0, 0.0, 1.0];
        renderer
            .set_property("color", PropertyValue::Color(new_color))
            .expect("should set color");
        assert_eq!(renderer.color, new_color);

        let err = renderer.set_property("unknown", PropertyValue::Bool(true));
        assert!(err.is_err());
    }

    #[test]
    fn test_srgb_to_linear() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert_eq!(srgb_to_linear(1.0), 1.0);

        // #121212: 18 / 255 = 0.070588
        let lin_12 = srgb_to_linear(18.0 / 255.0);
        assert!((lin_12 - 0.006048).abs() < 0.001);

        // #aaaaaa: 170 / 255 = 0.666667
        let lin_aa = srgb_to_linear(170.0 / 255.0);
        assert!((lin_aa - 0.401977).abs() < 0.001);
    }
}
