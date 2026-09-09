use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{DayNightMode, OscillationConfig, PanConfig, WallpaperManifest};
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
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
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
    let tint = vec3<f32>(u_layer.tint_r, u_layer.tint_g, u_layer.tint_b);
    return vec4<f32>(color.rgb * tint, color.a * u_layer.opacity);
}
"#;

fn uniform_bytes(offset: [f32; 2], scale: [f32; 2], opacity: f32, tint: [f32; 3]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&offset[0].to_ne_bytes());
    bytes[4..8].copy_from_slice(&offset[1].to_ne_bytes());
    bytes[8..12].copy_from_slice(&scale[0].to_ne_bytes());
    bytes[12..16].copy_from_slice(&scale[1].to_ne_bytes());
    bytes[16..20].copy_from_slice(&opacity.to_ne_bytes());
    bytes[20..24].copy_from_slice(&tint[0].to_ne_bytes());
    bytes[24..28].copy_from_slice(&tint[1].to_ne_bytes());
    bytes[28..32].copy_from_slice(&tint[2].to_ne_bytes());
    bytes
}

/// Returns the fractional local hour of the day in `[0.0, 24.0)`.
pub fn local_time_of_day() -> f32 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let tm = unsafe {
        libc::localtime_r(&now, tm.as_mut_ptr());
        tm.assume_init()
    };
    tm.tm_hour as f32 + tm.tm_min as f32 / 60.0 + tm.tm_sec as f32 / 3600.0
}

/// Returns the daylight factor in `[0.0, 1.0]`:
/// - `1.0` during full daylight (08:00 - 18:00)
/// - `0.0` during full night (21:00 - 05:30)
/// - Smooth transition during dawn and dusk.
pub fn daylight_factor_for_hour(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    if (8.0..=18.0).contains(&h) {
        1.0
    } else if (21.0..=24.0).contains(&h) || (0.0..=5.5).contains(&h) {
        0.0
    } else if (5.5..8.0).contains(&h) {
        let t = (h - 5.5) / 2.5;
        (t * std::f32::consts::PI * 0.5).sin().clamp(0.0, 1.0)
    } else {
        let t = (h - 18.0) / 3.0;
        (1.0 - t * std::f32::consts::PI * 0.5).sin().clamp(0.0, 1.0)
    }
}

/// Returns the ambient RGB tint factor based on fractional hour of the day.
pub fn ambient_tint_for_hour(hour: f32) -> [f32; 3] {
    let h = hour.rem_euclid(24.0);

    // Key time nodes: (hour, [R, G, B])
    let nodes: &[(f32, [f32; 3])] = &[
        (2.0, [0.40, 0.50, 0.75]),  // Deep night (cool moonlight blue)
        (5.0, [0.42, 0.52, 0.76]),  // Pre-dawn
        (6.5, [0.95, 0.78, 0.85]),  // Sunrise / dawn glow
        (8.5, [0.98, 0.98, 1.00]),  // Morning light
        (12.0, [1.00, 1.00, 1.00]), // Midday (neutral bright)
        (16.5, [1.00, 0.98, 0.95]), // Late afternoon
        (18.5, [1.05, 0.80, 0.58]), // Golden hour / sunset
        (20.0, [0.65, 0.50, 0.85]), // Twilight / dusk
        (21.5, [0.42, 0.50, 0.75]), // Nightfall
        (26.0, [0.40, 0.50, 0.75]), // Wraparound to 02:00 (2.0 + 24.0)
    ];

    let query_h = if h < 2.0 { h + 24.0 } else { h };

    for i in 0..nodes.len() - 1 {
        let (h0, rgb0) = nodes[i];
        let (h1, rgb1) = nodes[i + 1];
        if query_h >= h0 && query_h <= h1 {
            let t = (query_h - h0) / (h1 - h0);
            let s = t * t * (3.0 - 2.0 * t);
            return [
                rgb0[0] * (1.0 - s) + rgb1[0] * s,
                rgb0[1] * (1.0 - s) + rgb1[1] * s,
                rgb0[2] * (1.0 - s) + rgb1[2] * s,
            ];
        }
    }

    [1.0, 1.0, 1.0]
}

/// Definition of a single wallpaper image layer before GPU allocation.
#[derive(Debug, Clone)]
pub struct LayerDef {
    pub image_path: PathBuf,
    pub parallax: Option<f32>,
    pub pan: Option<PanConfig>,
    pub oscillation: Option<OscillationConfig>,
    pub day_night: Option<DayNightMode>,
    pub tint: Option<[f32; 3]>,
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
    oscillation: Option<OscillationConfig>,
    day_night: Option<DayNightMode>,
    tint: Option<[f32; 3]>,
    pan_offset: [f32; 2],
    parallax_offset: [f32; 2],
    target_offset: [f32; 2],
    opacity: f32,
}

/// Image wallpaper renderer supporting layered parallax, pan loops, sinusoidal oscillation, and day/night lighting.
pub struct ImageRenderer {
    layer_defs: Vec<LayerDef>,
    raw_memory_layers: Vec<(u32, u32, Vec<u8>)>,
    loaded_layers: Vec<LoadedLayer>,
    pipeline: Option<wgpu::RenderPipeline>,
    width: u32,
    height: u32,
    simulated_hour: Option<f32>,
}

impl Default for ImageRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory image representation: (width, height, raw_rgba, parallax, pan, oscillation, day_night, tint).
pub type MemoryLayer = (
    u32,
    u32,
    Vec<u8>,
    Option<f32>,
    Option<PanConfig>,
    Option<OscillationConfig>,
    Option<DayNightMode>,
    Option<[f32; 3]>,
);

impl ImageRenderer {
    pub fn new() -> Self {
        Self {
            layer_defs: Vec::new(),
            raw_memory_layers: Vec::new(),
            loaded_layers: Vec::new(),
            pipeline: None,
            width: 0,
            height: 0,
            simulated_hour: None,
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
            simulated_hour: None,
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
                    oscillation: l.oscillation.clone(),
                    day_night: l.day_night,
                    tint: l.tint,
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

        for (w, h, bytes, parallax, pan, oscillation, day_night, tint) in memory_layers {
            defs.push(LayerDef {
                image_path: PathBuf::new(),
                parallax,
                pan,
                oscillation,
                day_night,
                tint,
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
            simulated_hour: None,
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

            let initial_tint = def.tint.unwrap_or([1.0, 1.0, 1.0]);
            let uniform_init = uniform_bytes([0.0, 0.0], [1.0, 1.0], def.opacity, initial_tint);
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
                oscillation: def.oscillation.clone(),
                day_night: def.day_night,
                tint: def.tint,
                pan_offset: [0.0, 0.0],
                parallax_offset: [0.0, 0.0],
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

        let active_hour = self.simulated_hour.unwrap_or_else(local_time_of_day);
        let daylight = daylight_factor_for_hour(active_hour);
        let ambient_tint = ambient_tint_for_hour(active_hour);

        for layer in &mut self.loaded_layers {
            // 1. Continuous Pan Animation (independent accumulation)
            if let Some(pan) = &layer.pan {
                let shift = pan.speed * dt;
                if pan.axis.eq_ignore_ascii_case("y") {
                    layer.pan_offset[1] = (layer.pan_offset[1] + shift) % 1.0;
                } else {
                    layer.pan_offset[0] = (layer.pan_offset[0] + shift) % 1.0;
                }
            }

            // 2. Interactive Parallax via Pointer Position (independent smoothing)
            if layer.parallax.abs() > 1e-4 {
                let target_x = -pointer.0 * layer.parallax * 0.04;
                let target_y = pointer.1 * layer.parallax * 0.04;
                layer.target_offset = [target_x, target_y];

                let lerp = (dt * 8.0).min(1.0);
                layer.parallax_offset[0] +=
                    (layer.target_offset[0] - layer.parallax_offset[0]) * lerp;
                layer.parallax_offset[1] +=
                    (layer.target_offset[1] - layer.parallax_offset[1]) * lerp;
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

                // Add margin for parallax or oscillation motion so edges stay hidden
                let has_motion = layer.parallax.abs() > 1e-4 || layer.oscillation.is_some();
                if has_motion {
                    let margin = 0.90;
                    scale[0] *= margin;
                    scale[1] *= margin;
                }
            }

            // 4. Compute final offset (pan + parallax + periodic oscillation)
            let mut final_offset = [
                layer.pan_offset[0] + layer.parallax_offset[0],
                layer.pan_offset[1] + layer.parallax_offset[1],
            ];
            if let Some(osc) = &layer.oscillation {
                let phase = osc.phase.unwrap_or(0.0);
                let wave = (ctx.elapsed.as_secs_f32() * osc.speed + phase).sin() * osc.amplitude;
                if osc.axis.eq_ignore_ascii_case("x") {
                    final_offset[0] += wave;
                } else {
                    final_offset[1] += wave;
                }
            }

            // 5. Apply Day/Night lighting and tint modulation
            let mut effective_opacity = layer.opacity;
            let mut effective_tint = layer.tint.unwrap_or([1.0, 1.0, 1.0]);

            match layer.day_night {
                Some(DayNightMode::Night) => {
                    effective_opacity *= 1.0 - daylight;
                }
                Some(DayNightMode::Day) => {
                    effective_opacity *= daylight;
                }
                Some(DayNightMode::Tint) => {
                    effective_tint[0] *= ambient_tint[0];
                    effective_tint[1] *= ambient_tint[1];
                    effective_tint[2] *= ambient_tint[2];
                }
                None => {}
            }

            // 6. Update GPU uniform buffer
            let raw = uniform_bytes(final_offset, scale, effective_opacity, effective_tint);
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
        if key == "hour" || key == "time_of_day" {
            if let PropertyValue::Number(num) = value {
                if num < 0.0 {
                    self.simulated_hour = None;
                } else {
                    self.simulated_hour = Some(num.rem_euclid(24.0));
                }
                return Ok(());
            }
            return Err(RendererError::InvalidPropertyValue(
                "hour must be a number (0..24, or negative for system clock)".into(),
            ));
        }
        Err(RendererError::PropertyNotFound(key.into()))
    }

    fn is_animated(&self) -> bool {
        if !self.loaded_layers.is_empty() {
            self.loaded_layers.iter().any(|l| {
                l.pan.is_some()
                    || l.oscillation.is_some()
                    || l.day_night.is_some()
                    || l.parallax.abs() > 1e-4
            })
        } else {
            self.layer_defs.iter().any(|l| {
                l.pan.is_some()
                    || l.oscillation.is_some()
                    || l.day_night.is_some()
                    || l.parallax.unwrap_or(0.0).abs() > 1e-4
            })
        }
    }

    fn wants_pointer(&self) -> bool {
        if !self.loaded_layers.is_empty() {
            self.loaded_layers.iter().any(|l| l.parallax.abs() > 1e-4)
        } else {
            self.layer_defs
                .iter()
                .any(|l| l.parallax.unwrap_or(0.0).abs() > 1e-4)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_renderer_lifecycle() {
        let mut renderer = ImageRenderer::new();
        assert!(!renderer.is_animated());
        renderer.resize(2560, 1440);
        assert_eq!(renderer.width, 2560);
        assert_eq!(renderer.height, 1440);
        assert_eq!(renderer.layer_count(), 0);
    }

    #[test]
    fn test_image_renderer_from_memory() {
        // Create static 2x2 RGBA layer
        let layer_static = (
            2,
            2,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
            None,
            None,
            None,
            None,
            None,
        );
        let renderer_static = ImageRenderer::from_memory_layers(vec![layer_static.clone()]);
        assert!(!renderer_static.is_animated());
        assert!(!renderer_static.wants_pointer());

        // Create layer with pan and parallax
        let layer1 = layer_static;
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
            None,
            None,
            None,
        );

        let renderer = ImageRenderer::from_memory_layers(vec![layer1.clone(), layer2]);
        assert_eq!(renderer.layer_count(), 2);
        assert!(renderer.is_animated());
        assert!(renderer.wants_pointer());

        // Create layer with oscillation
        let layer3 = (
            2,
            2,
            vec![
                0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255,
            ],
            None,
            None,
            Some(OscillationConfig {
                speed: 1.5,
                amplitude: 0.03,
                axis: "y".into(),
                phase: Some(0.0),
            }),
            None,
            None,
        );
        let renderer_osc = ImageRenderer::from_memory_layers(vec![layer1.clone(), layer3]);
        assert_eq!(renderer_osc.layer_count(), 2);
        assert!(renderer_osc.is_animated());
        assert!(!renderer_osc.wants_pointer());

        // Create layer with day/night mode
        let layer_dn = (
            2,
            2,
            vec![
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            ],
            None,
            None,
            None,
            Some(DayNightMode::Night),
            Some([0.8, 0.9, 1.0]),
        );
        let renderer_dn = ImageRenderer::from_memory_layers(vec![layer1, layer_dn]);
        assert_eq!(renderer_dn.layer_count(), 2);
        assert!(renderer_dn.is_animated());
    }

    #[test]
    fn test_uniform_bytes_layout() {
        let bytes = uniform_bytes([0.1, 0.2], [1.5, 2.0], 0.8, [0.5, 0.6, 0.7]);
        assert_eq!(bytes.len(), 32);

        let offset_x = f32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let offset_y = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let scale_x = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        let scale_y = f32::from_ne_bytes(bytes[12..16].try_into().unwrap());
        let opacity = f32::from_ne_bytes(bytes[16..20].try_into().unwrap());
        let tint_r = f32::from_ne_bytes(bytes[20..24].try_into().unwrap());
        let tint_g = f32::from_ne_bytes(bytes[24..28].try_into().unwrap());
        let tint_b = f32::from_ne_bytes(bytes[28..32].try_into().unwrap());

        assert!((offset_x - 0.1).abs() < 1e-5);
        assert!((offset_y - 0.2).abs() < 1e-5);
        assert!((scale_x - 1.5).abs() < 1e-5);
        assert!((scale_y - 2.0).abs() < 1e-5);
        assert!((opacity - 0.8).abs() < 1e-5);
        assert!((tint_r - 0.5).abs() < 1e-5);
        assert!((tint_g - 0.6).abs() < 1e-5);
        assert!((tint_b - 0.7).abs() < 1e-5);
    }

    #[test]
    fn test_day_night_curves() {
        // Noon: full daylight factor 1.0, neutral tint [1.0, 1.0, 1.0]
        let noon_daylight = daylight_factor_for_hour(12.0);
        assert!((noon_daylight - 1.0).abs() < 1e-5);
        let noon_tint = ambient_tint_for_hour(12.0);
        assert!((noon_tint[0] - 1.0).abs() < 1e-3);
        assert!((noon_tint[1] - 1.0).abs() < 1e-3);
        assert!((noon_tint[2] - 1.0).abs() < 1e-3);

        // Midnight: 0.0 daylight factor, cool blue night tint
        let midnight_daylight = daylight_factor_for_hour(0.0);
        assert!((midnight_daylight - 0.0).abs() < 1e-5);
        let midnight_tint = ambient_tint_for_hour(2.0);
        assert!(midnight_tint[0] < 0.5);
        assert!(midnight_tint[2] > midnight_tint[0]); // blue dominant

        // Dawn transition
        let dawn_daylight = daylight_factor_for_hour(6.75);
        assert!(dawn_daylight > 0.0 && dawn_daylight < 1.0);
    }

    #[test]
    fn test_set_hour_property() {
        let mut renderer = ImageRenderer::new();
        assert!(renderer.simulated_hour.is_none());

        renderer
            .set_property("hour", PropertyValue::Number(14.5))
            .expect("should set hour");
        assert_eq!(renderer.simulated_hour, Some(14.5));

        // Negative resets to system time
        renderer
            .set_property("time_of_day", PropertyValue::Number(-1.0))
            .expect("should reset hour");
        assert!(renderer.simulated_hour.is_none());
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
            assert_eq!(renderer.layer_count(), 4);
            assert!(renderer.layer_defs[0].image_path.exists());
            assert!(renderer.layer_defs[1].image_path.exists());
            assert!(renderer.layer_defs[2].image_path.exists());
            assert!(renderer.layer_defs[3].image_path.exists());
        }
    }
}
