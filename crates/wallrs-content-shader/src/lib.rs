use wallrs_render::{FrameContext, RendererError, WallpaperRenderer};

/// Shader wallpaper renderer for WGSL and Shadertoy GLSL compatible shaders.
#[derive(Debug, Default)]
pub struct ShaderRenderer {
    width: u32,
    height: u32,
}

impl ShaderRenderer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WallpaperRenderer for ShaderRenderer {
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

    fn render(&mut self, _encoder: &mut wgpu::CommandEncoder, _view: &wgpu::TextureView) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_renderer_lifecycle() {
        let mut renderer = ShaderRenderer::new();
        renderer.resize(1920, 1080);
        assert_eq!(renderer.width, 1920);
        assert_eq!(renderer.height, 1080);
    }
}
