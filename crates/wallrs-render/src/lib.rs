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

    /// Tears down any allocated resources.
    fn teardown(&mut self) {}
}

/// A simple renderer that fills the surface with a solid RGBA color.
#[derive(Debug, Clone)]
pub struct SolidColorRenderer {
    pub color: [f32; 4],
    pub width: u32,
    pub height: u32,
}

impl SolidColorRenderer {
    pub fn new(color: [f32; 4]) -> Self {
        Self {
            color,
            width: 0,
            height: 0,
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
        _target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError> {
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn update(&mut self, _ctx: &FrameContext) {}

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("solid_color_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: self.color[0] as f64,
                        g: self.color[1] as f64,
                        b: self.color[2] as f64,
                        a: self.color[3] as f64,
                    }),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solid_color_renderer_properties() {
        let mut renderer = SolidColorRenderer::default();
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
}
