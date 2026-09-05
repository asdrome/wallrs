use wallrs_render::{FrameContext, RendererError, WallpaperRenderer};

/// Video wallpaper renderer powered by libmpv (software rendering API mode).
#[derive(Debug, Default)]
pub struct VideoRenderer {
    width: u32,
    height: u32,
}

impl VideoRenderer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl WallpaperRenderer for VideoRenderer {
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
    fn test_video_renderer_lifecycle() {
        let mut renderer = VideoRenderer::new();
        renderer.resize(3840, 2160);
        assert_eq!(renderer.width, 3840);
        assert_eq!(renderer.height, 2160);
    }
}
