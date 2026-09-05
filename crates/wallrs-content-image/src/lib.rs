use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{PanConfig, WallpaperManifest};
use wallrs_render::{FrameContext, PropertyValue, RendererError, WallpaperRenderer};

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("Failed to decode image from {0:?}: {1}")]
    Decode(PathBuf, image::ImageError),

    #[error("Failed to read image file at {0:?}: {1}")]
    Io(PathBuf, std::io::Error),

    #[error("No layers defined in image configuration")]
    NoLayers,
}

const SHADER_SRC: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct LayerUniform {
    offset: vec2<f32>,
    scale: vec2<f32>,
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0)
var<uniform> u_layer: LayerUniform;

@group(0) @binding(1)
var t_texture: texture_2d<f32>;

@group(0) @binding(2)
var s_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
    );

    let p = pos[in_vertex_index];
    out.position = vec4<f32>(p, 0.0, 1.0);

    let base_uv = vec2<f32>(p.x * 0.5 + 0.5, -p.y * 0.5 + 0.5);
    out.uv = (base_uv - vec2<f32>(0.5, 0.5)) * u_layer.scale + vec2<f32>(0.5, 0.5) + u_layer.offset;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_texture, s_sampler, in.uv);
    return vec4<f32>(color.rgb, color.a * u_layer.opacity);
}
"#;

fn uniform_bytes(offset: [f32; 2], scale: [f32; 2], opacity: f32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&offset[0].to_ne_bytes());
    bytes[4..8].copy_from_slice(&offset[1].to_ne_bytes());
    bytes[8..12].copy_from_slice(&scale[0].to_ne_bytes());
    bytes[12..16].copy_from_slice(&scale[1].to_ne_bytes());
    bytes[16..20].copy_from_slice(&opacity.to_ne_bytes());
    bytes
}

/// Definition of a single wallpaper image layer before GPU allocation.
#[derive(Debug, Clone)]
pub struct LayerDef {
    pub image_path: PathBuf,
    pub parallax: Option<f32>,
    pub pan: Option<PanConfig>,
    pub opacity: f32,
}

struct LoadedLayer {
    _texture: wgpu::Texture,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    img_width: u32,
    img_height: u32,
    parallax: f32,
    pan: Option<PanConfig>,
    current_offset: [f32; 2],
    target_offset: [f32; 2],
    opacity: f32,
}

/// Image wallpaper renderer supporting layered parallax and pan loops.
pub struct ImageRenderer {
    layer_defs: Vec<LayerDef>,
    raw_memory_layers: Vec<(u32, u32, Vec<u8>)>,
    loaded_layers: Vec<LoadedLayer>,
    pipeline: Option<wgpu::RenderPipeline>,
    width: u32,
    height: u32,
}

impl Default for ImageRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory image representation: (width, height, raw_rgba, parallax, pan).
pub type MemoryLayer = (u32, u32, Vec<u8>, Option<f32>, Option<PanConfig>);

impl ImageRenderer {
    pub fn new() -> Self {
        Self {
            layer_defs: Vec::new(),
            raw_memory_layers: Vec::new(),
            loaded_layers: Vec::new(),
            pipeline: None,
            width: 0,
            height: 0,
        }
    }

    pub fn from_layers(layers: Vec<LayerDef>) -> Self {
        Self {
            layer_defs: layers,
            raw_memory_layers: Vec::new(),
            loaded_layers: Vec::new(),
            pipeline: None,
            width: 0,
            height: 0,
        }
    }

    pub fn from_manifest(
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Self, ImageError> {
        let Some(image_config) = &manifest.image else {
            return Err(ImageError::NoLayers);
        };
        if image_config.layers.is_empty() {
            return Err(ImageError::NoLayers);
        }

        let layers = image_config
            .layers
            .iter()
            .map(|l| {
                let full_path = if l.path.is_absolute() {
                    l.path.clone()
                } else {
                    base_dir.join(&l.path)
                };
                LayerDef {
                    image_path: full_path,
                    parallax: l.parallax,
                    pan: l.pan.clone(),
                    opacity: 1.0,
                }
            })
            .collect();

        Ok(Self::from_layers(layers))
    }

    /// Convenience builder for in-memory RGBA images (useful for tests and synthetic textures).
    pub fn from_memory_layers(memory_layers: Vec<MemoryLayer>) -> Self {
        let mut defs = Vec::new();
        let mut raw = Vec::new();

        for (w, h, bytes, parallax, pan) in memory_layers {
            defs.push(LayerDef {
                image_path: PathBuf::new(),
                parallax,
                pan,
                opacity: 1.0,
            });
            raw.push((w, h, bytes));
        }

        Self {
            layer_defs: defs,
            raw_memory_layers: raw,
            loaded_layers: Vec::new(),
            pipeline: None,
            width: 0,
            height: 0,
        }
    }

    pub fn layer_count(&self) -> usize {
        self.layer_defs.len()
    }
}

impl WallpaperRenderer for ImageRenderer {
    fn init(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image_wallpaper_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image_layer_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        self.loaded_layers.clear();

        for (i, def) in self.layer_defs.iter().enumerate() {
            let (img_w, img_h, raw_rgba) = if let Some(mem) = self.raw_memory_layers.get(i) {
                (mem.0, mem.1, mem.2.clone())
            } else {
                let img = image::open(&def.image_path)
                    .map_err(|e| {
                        RendererError::InitFailed(format!(
                            "Failed to open image {:?}: {e}",
                            def.image_path
                        ))
                    })?
                    .to_rgba8();
                (img.width(), img.height(), img.into_raw())
            };

            let size = wgpu::Extent3d {
                width: img_w,
                height: img_h,
                depth_or_array_layers: 1,
            };

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("image_texture_{i}")),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &raw_rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * img_w),
                    rows_per_image: Some(img_h),
                },
                size,
            );

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let uniform_init = uniform_bytes([0.0, 0.0], [1.0, 1.0], def.opacity);
            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("layer_{i}_uniform_buffer")),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&uniform_buffer, 0, &uniform_init);

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("layer_{i}_bind_group")),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

            self.loaded_layers.push(LoadedLayer {
                _texture: texture,
                uniform_buffer,
                bind_group,
                img_width: img_w,
                img_height: img_h,
                parallax: def.parallax.unwrap_or(0.0),
                pan: def.pan.clone(),
                current_offset: [0.0, 0.0],
                target_offset: [0.0, 0.0],
                opacity: def.opacity,
            });
        }

        self.pipeline = Some(pipeline);
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn update(&mut self, ctx: &FrameContext) {
        let dt = ctx.delta.as_secs_f32();
        let pointer = ctx.pointer.unwrap_or((0.0, 0.0));

        for layer in &mut self.loaded_layers {
            // 1. Continuous Pan Animation
            if let Some(pan) = &layer.pan {
                let shift = pan.speed * dt;
                if pan.axis.eq_ignore_ascii_case("y") {
                    layer.current_offset[1] = (layer.current_offset[1] + shift) % 1.0;
                } else {
                    layer.current_offset[0] = (layer.current_offset[0] + shift) % 1.0;
                }
            }

            // 2. Interactive Parallax via Pointer Position
            if layer.parallax.abs() > 1e-4 {
                let target_x = -pointer.0 * layer.parallax * 0.04;
                let target_y = pointer.1 * layer.parallax * 0.04;
                layer.target_offset = [target_x, target_y];

                let lerp = (dt * 8.0).min(1.0);
                layer.current_offset[0] +=
                    (layer.target_offset[0] - layer.current_offset[0]) * lerp;
                layer.current_offset[1] +=
                    (layer.target_offset[1] - layer.current_offset[1]) * lerp;
            }

            // 3. Aspect Ratio Cover Fit
            let mut scale = [1.0f32, 1.0f32];
            if self.width > 0 && self.height > 0 && layer.img_width > 0 && layer.img_height > 0 {
                let screen_aspect = self.width as f32 / self.height as f32;
                let img_aspect = layer.img_width as f32 / layer.img_height as f32;

                if screen_aspect > img_aspect {
                    scale[0] = 1.0;
                    scale[1] = img_aspect / screen_aspect;
                } else {
                    scale[0] = screen_aspect / img_aspect;
                    scale[1] = 1.0;
                }

                // Add margin for parallax motion so edges stay hidden
                if layer.parallax.abs() > 1e-4 {
                    let margin = 0.90;
                    scale[0] *= margin;
                    scale[1] *= margin;
                }
            }

            // 4. Update GPU uniform buffer
            let raw = uniform_bytes(layer.current_offset, scale, layer.opacity);
            ctx.queue.write_buffer(&layer.uniform_buffer, 0, &raw);
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("image_wallpaper_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
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

        rpass.set_pipeline(pipeline);

        for layer in &self.loaded_layers {
            rpass.set_bind_group(0, &layer.bind_group, &[]);
            rpass.draw(0..6, 0..1);
        }
    }

    fn set_property(&mut self, key: &str, value: PropertyValue) -> Result<(), RendererError> {
        if key == "opacity" {
            if let PropertyValue::Number(num) = value {
                for layer in &mut self.loaded_layers {
                    layer.opacity = num.clamp(0.0, 1.0);
                }
                return Ok(());
            }
            return Err(RendererError::InvalidPropertyValue(
                "opacity must be a number".into(),
            ));
        }
        Err(RendererError::PropertyNotFound(key.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_renderer_lifecycle() {
        let mut renderer = ImageRenderer::new();
        renderer.resize(2560, 1440);
        assert_eq!(renderer.width, 2560);
        assert_eq!(renderer.height, 1440);
        assert_eq!(renderer.layer_count(), 0);
    }

    #[test]
    fn test_image_renderer_from_memory() {
        // Create 2 2x2 RGBA layers
        let layer1 = (
            2,
            2,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
            None,
            None,
        );
        let layer2 = (
            2,
            2,
            vec![
                0, 255, 0, 128, 0, 255, 0, 128, 0, 255, 0, 128, 0, 255, 0, 128,
            ],
            Some(0.5),
            Some(PanConfig {
                speed: 0.01,
                axis: "x".into(),
            }),
        );

        let renderer = ImageRenderer::from_memory_layers(vec![layer1, layer2]);
        assert_eq!(renderer.layer_count(), 2);
    }

    #[test]
    fn test_uniform_bytes_layout() {
        let bytes = uniform_bytes([0.1, 0.2], [1.5, 2.0], 0.8);
        assert_eq!(bytes.len(), 32);

        let offset_x = f32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let offset_y = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let scale_x = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        let scale_y = f32::from_ne_bytes(bytes[12..16].try_into().unwrap());
        let opacity = f32::from_ne_bytes(bytes[16..20].try_into().unwrap());

        assert!((offset_x - 0.1).abs() < 1e-5);
        assert!((offset_y - 0.2).abs() < 1e-5);
        assert!((scale_x - 1.5).abs() < 1e-5);
        assert!((scale_y - 2.0).abs() < 1e-5);
        assert!((opacity - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_load_sample_manifest_and_images() {
        let manifest_path = Path::new("../../examples/parallax-landscape/wallpaper.toml");
        if manifest_path.exists() {
            let manifest = WallpaperManifest::from_file(manifest_path)
                .expect("failed to load sample wallpaper.toml");
            let base_dir = manifest_path.parent().unwrap();
            let renderer = ImageRenderer::from_manifest(&manifest, base_dir)
                .expect("failed to build ImageRenderer");
            assert_eq!(renderer.layer_count(), 2);
            assert!(renderer.layer_defs[0].image_path.exists());
            assert!(renderer.layer_defs[1].image_path.exists());
        }
    }
}
