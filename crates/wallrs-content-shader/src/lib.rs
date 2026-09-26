use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{PropertyValue, WallpaperManifest};
use wallrs_render::{FrameContext, RendererError, WallpaperRenderer};

#[derive(Debug, Error)]
pub enum ShaderError {
    #[error("Failed to read shader file at {0:?}: {1}")]
    Io(PathBuf, std::io::Error),

    #[error("GLSL parse error: {0}")]
    Glsl(String),

    #[error("Naga validation error: {0}")]
    Validation(String),

    #[error("WGSL translation error: {0}")]
    Wgsl(String),

    #[error("No entry file defined in shader configuration")]
    NoEntry,
}

/// Factory plugin for constructing and validating shader wallpapers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShaderRendererFactory;

impl ShaderRendererFactory {
    pub fn new() -> Self {
        Self
    }
}

impl wallrs_render::RendererFactory for ShaderRendererFactory {
    fn create_renderer(
        &self,
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Box<dyn WallpaperRenderer>, RendererError> {
        let renderer = ShaderRenderer::from_manifest(manifest, base_dir)
            .map_err(|e| RendererError::InitFailed(e.to_string()))?;
        Ok(Box::new(renderer))
    }

    fn supports_type(&self, type_name: &str) -> bool {
        type_name == "shader"
    }

    fn validate(&self, manifest: &WallpaperManifest, base_dir: &Path) -> Result<(), RendererError> {
        let Some(sh) = &manifest.shader else {
            return Err(RendererError::ValidationFailed(
                "Manifest declares type 'shader' but is missing [shader] configuration block"
                    .into(),
            ));
        };
        let shader_file = if sh.entry.is_absolute() {
            sh.entry.clone()
        } else {
            base_dir.join(&sh.entry)
        };
        if !shader_file.exists() {
            return Err(RendererError::ValidationFailed(format!(
                "Shader entry file does not exist: {shader_file:?}"
            )));
        }
        let shader_code = std::fs::read_to_string(&shader_file).map_err(|e| {
            RendererError::ValidationFailed(format!(
                "Failed to read shader file {shader_file:?}: {e}"
            ))
        })?;

        let is_glsl = shader_file.extension().and_then(|ext| ext.to_str()) == Some("glsl")
            || shader_code.contains("void mainImage");

        let named_uniforms = sh
            .uniform_mapping
            .clone()
            .unwrap_or_else(|| sh.uniforms.keys().cloned().collect());

        crate::validate_shader_source(&shader_code, is_glsl, &named_uniforms).map_err(|e| {
            RendererError::ValidationFailed(format!("Shader validation error: {e}"))
        })?;

        Ok(())
    }

    fn capabilities(&self, _manifest: &WallpaperManifest) -> wallrs_render::RendererCapabilities {
        wallrs_render::RendererCapabilities {
            needs_audio_spectrum: true,
            produces_audio: false,
        }
    }
}

pub const STANDARD_VS_WGSL: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0)
    );
    let p = pos[in_vertex_index];
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 1.0 - (p.y * 0.5 + 0.5));
    return out;
}
"#;

pub const STANDARD_UNIFORM_WGSL: &str = r#"
struct ShaderUniforms {
    resolution: vec2<f32>,
    time: f32,
    time_delta: f32,
    mouse: vec4<f32>,
    frame: u32,
    custom0: f32,
    custom1: f32,
    custom2: f32,
    audio_bass: f32,
    audio_mid: f32,
    audio_treble: f32,
    audio_volume: f32,
    audio_spectrum: array<vec4<f32>, 8>,
    custom_extra: array<vec4<f32>, 2>,
};

@group(0) @binding(0)
var<uniform> u_params: ShaderUniforms;

fn get_audio_band(idx: u32) -> f32 {
    let clamped_idx = min(idx, 31u);
    let vec_idx = clamped_idx / 4u;
    let comp_idx = clamped_idx % 4u;
    return u_params.audio_spectrum[vec_idx][comp_idx];
}

fn get_custom(slot: u32) -> f32 {
    if (slot == 0u) { return u_params.custom0; }
    if (slot == 1u) { return u_params.custom1; }
    if (slot == 2u) { return u_params.custom2; }
    let extra_idx = slot - 3u;
    if (extra_idx < 8u) {
        return u_params.custom_extra[extra_idx / 4u][extra_idx % 4u];
    }
    return 0.0;
}
"#;

#[allow(clippy::too_many_arguments)]
fn uniform_bytes(
    resolution: [f32; 2],
    time: f32,
    time_delta: f32,
    mouse: [f32; 4],
    frame: u32,
    custom: &[f32],
    audio: &wallrs_audio::AudioMetrics,
    spectrum: Option<&[f32]>,
) -> [u8; 224] {
    let mut bytes = [0u8; 224];
    bytes[0..4].copy_from_slice(&resolution[0].to_ne_bytes());
    bytes[4..8].copy_from_slice(&resolution[1].to_ne_bytes());
    bytes[8..12].copy_from_slice(&time.to_ne_bytes());
    bytes[12..16].copy_from_slice(&time_delta.to_ne_bytes());
    bytes[16..20].copy_from_slice(&mouse[0].to_ne_bytes());
    bytes[20..24].copy_from_slice(&mouse[1].to_ne_bytes());
    bytes[24..28].copy_from_slice(&mouse[2].to_ne_bytes());
    bytes[28..32].copy_from_slice(&mouse[3].to_ne_bytes());
    bytes[32..36].copy_from_slice(&frame.to_ne_bytes());

    let c0 = custom.first().copied().unwrap_or(0.0);
    let c1 = custom.get(1).copied().unwrap_or(0.0);
    let c2 = custom.get(2).copied().unwrap_or(0.0);
    bytes[36..40].copy_from_slice(&c0.to_ne_bytes());
    bytes[40..44].copy_from_slice(&c1.to_ne_bytes());
    bytes[44..48].copy_from_slice(&c2.to_ne_bytes());

    bytes[48..52].copy_from_slice(&audio.bass.to_ne_bytes());
    bytes[52..56].copy_from_slice(&audio.mid.to_ne_bytes());
    bytes[56..60].copy_from_slice(&audio.treble.to_ne_bytes());
    bytes[60..64].copy_from_slice(&audio.volume.to_ne_bytes());

    if let Some(spec) = spectrum {
        let count = spec.len().min(32);
        for (i, &band) in spec.iter().enumerate().take(count) {
            let offset = 64 + i * 4;
            bytes[offset..offset + 4].copy_from_slice(&band.to_ne_bytes());
        }
    }

    for i in 0..8 {
        if let Some(&val) = custom.get(3 + i) {
            let offset = 192 + i * 4;
            bytes[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
        }
    }

    bytes
}

/// Translates Shadertoy-style GLSL fragment code into standard WGSL using Naga.
pub fn translate_shadertoy_glsl_to_wgsl(glsl_source: &str) -> Result<String, ShaderError> {
    let preamble = r#"#version 450
precision highp float;

layout(std140, set = 0, binding = 0) uniform UniformBlock {
    vec2 iResolution2D;
    float iTime;
    float iTimeDelta;
    vec4 iMouse;
    uint iFrame;
    float u_custom0;
    float u_custom1;
    float u_custom2;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_volume;
    vec4 audio_spectrum[8];
    vec4 custom_extra[2];
};

#define iResolution vec3(iResolution2D, 1.0)
#define iBass audio_bass
#define iMid audio_mid
#define iTreble audio_treble
#define iVolume audio_volume

float get_audio_band(uint idx) {
    uint clamped = min(idx, 31u);
    uint vec_idx = clamped / 4u;
    uint comp_idx = clamped % 4u;
    return audio_spectrum[vec_idx][comp_idx];
}

float get_custom(uint slot) {
    if (slot == 0u) return u_custom0;
    if (slot == 1u) return u_custom1;
    if (slot == 2u) return u_custom2;
    uint extra_idx = slot - 3u;
    if (extra_idx < 8u) {
        return custom_extra[extra_idx / 4u][extra_idx % 4u];
    }
    return 0.0;
}

layout(location = 0) out vec4 _outColor;
"#;

    let epilogue = r#"
void main() {
    mainImage(_outColor, gl_FragCoord.xy);
}
"#;

    let full_glsl = if glsl_source.contains("mainImage") && !glsl_source.contains("void main(") {
        format!("{preamble}\n{glsl_source}\n{epilogue}")
    } else {
        glsl_source.to_string()
    };

    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let mut frontend = naga::front::glsl::Frontend::default();
    let module = frontend
        .parse(&options, &full_glsl)
        .map_err(|e| ShaderError::Glsl(format!("{e:?}")))?;

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    let info = validator
        .validate(&module)
        .map_err(|e| ShaderError::Validation(format!("{e:?}")))?;

    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| ShaderError::Wgsl(format!("{e:?}")))
}

/// Prepares WGSL source code by inlining uniforms if needed.
pub fn prepare_wgsl(source: &str) -> String {
    prepare_wgsl_with_uniforms(source, &[])
}

/// Prepares WGSL source code by inlining uniforms and named alias helper functions if needed.
pub fn prepare_wgsl_with_uniforms(source: &str, named_uniforms: &[String]) -> String {
    let base = if !source.contains("ShaderUniforms") && !source.contains("var<uniform>") {
        format!("{STANDARD_UNIFORM_WGSL}\n{source}")
    } else {
        source.to_string()
    };

    if named_uniforms.is_empty() {
        return base;
    }

    let mut helpers = String::new();
    for (idx, name) in named_uniforms.iter().enumerate().take(11) {
        if is_valid_wgsl_identifier(name)
            && !matches!(
                name.as_str(),
                "custom0" | "custom1" | "custom2" | "time" | "resolution" | "mouse" | "frame"
            )
            && !base.contains(&format!("fn {name}("))
        {
            helpers.push_str(&format!(
                "\nfn {name}() -> f32 {{\n    return get_custom({idx}u);\n}}\n"
            ));
        }
    }

    if helpers.is_empty() {
        base
    } else {
        format!("{helpers}\n{base}")
    }
}

fn is_valid_wgsl_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Validates shader source code (GLSL or WGSL) using Naga without initializing a WGPU device.
pub fn validate_shader_source(
    source: &str,
    is_glsl: bool,
    named_uniforms: &[String],
) -> Result<(), ShaderError> {
    let wgsl = if is_glsl {
        translate_shadertoy_glsl_to_wgsl(source)?
    } else {
        prepare_wgsl_with_uniforms(source, named_uniforms)
    };

    let module = naga::front::wgsl::parse_str(&wgsl)
        .map_err(|e| ShaderError::Validation(e.emit_to_string(&wgsl)))?;

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .map_err(|e| ShaderError::Validation(format!("{e:?}")))?;

    Ok(())
}

/// Shader wallpaper renderer supporting native WGSL and Shadertoy GLSL via Naga.
pub struct ShaderRenderer {
    entry_path: Option<PathBuf>,
    raw_source: Option<String>,
    is_glsl: bool,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    custom_uniforms: HashMap<String, f32>,
    uniform_slots: Vec<String>,
    target_fps: Option<f64>,
    width: u32,
    height: u32,
    frame_count: u32,
}

impl Default for ShaderRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderRenderer {
    pub fn new() -> Self {
        Self {
            entry_path: None,
            raw_source: None,
            is_glsl: false,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            custom_uniforms: HashMap::new(),
            uniform_slots: Vec::new(),
            target_fps: Some(60.0),
            width: 0,
            height: 0,
            frame_count: 0,
        }
    }

    pub fn from_wgsl_source(source: &str) -> Self {
        Self {
            entry_path: None,
            raw_source: Some(source.to_string()),
            is_glsl: false,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            custom_uniforms: HashMap::new(),
            uniform_slots: Vec::new(),
            target_fps: Some(60.0),
            width: 0,
            height: 0,
            frame_count: 0,
        }
    }

    pub fn from_glsl_source(source: &str) -> Self {
        Self {
            entry_path: None,
            raw_source: Some(source.to_string()),
            is_glsl: true,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            custom_uniforms: HashMap::new(),
            uniform_slots: Vec::new(),
            target_fps: Some(60.0),
            width: 0,
            height: 0,
            frame_count: 0,
        }
    }

    pub fn from_manifest(
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Self, ShaderError> {
        let Some(shader_config) = &manifest.shader else {
            return Err(ShaderError::NoEntry);
        };

        let full_path = if shader_config.entry.is_absolute() {
            shader_config.entry.clone()
        } else {
            base_dir.join(&shader_config.entry)
        };

        let ext = full_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_glsl = ext == "glsl" || ext == "frag";

        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| ShaderError::Io(full_path.clone(), e))?;

        let mut uniform_slots = Vec::new();
        if let Some(mapping) = &shader_config.uniform_mapping {
            uniform_slots.extend(mapping.iter().take(11).cloned());
        } else {
            let mut named: Vec<String> = shader_config.uniforms.keys().cloned().collect();
            named.sort();
            for k in named {
                if !uniform_slots.contains(&k) && uniform_slots.len() < 11 {
                    uniform_slots.push(k);
                }
            }
        }

        let target_fps = match shader_config.fps {
            Some(0) => None,
            Some(f) => Some(f as f64),
            None => Some(60.0),
        };

        Ok(Self {
            entry_path: Some(full_path),
            raw_source: Some(content),
            is_glsl,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            custom_uniforms: shader_config.uniforms.clone(),
            uniform_slots,
            target_fps,
            width: 0,
            height: 0,
            frame_count: 0,
        })
    }

    pub fn entry_path(&self) -> Option<&Path> {
        self.entry_path.as_deref()
    }
}

impl WallpaperRenderer for ShaderRenderer {
    fn init(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError> {
        let source = self.raw_source.as_deref().unwrap_or("");
        if source.is_empty() {
            return Err(RendererError::InitFailed("Shader source is empty".into()));
        }

        let (final_fs_wgsl, entry_point) = if self.is_glsl {
            let wgsl = translate_shadertoy_glsl_to_wgsl(source)
                .map_err(|e| RendererError::InitFailed(format!("GLSL translation failed: {e}")))?;
            (wgsl, "main".to_string())
        } else {
            let prepared = prepare_wgsl_with_uniforms(source, &self.uniform_slots);
            let ep = if prepared.contains("fn fs_main") {
                "fs_main"
            } else if prepared.contains("fn main") {
                "main"
            } else {
                "fs_main"
            };
            (prepared, ep.to_string())
        };

        let vs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("standard_vs_module"),
            source: wgpu::ShaderSource::Wgsl(STANDARD_VS_WGSL.into()),
        });

        let fs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("user_fs_module"),
            source: wgpu::ShaderSource::Wgsl(final_fs_wgsl.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader_uniform_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs_module,
                entry_point: Some(&entry_point),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
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

        let default_audio = wallrs_audio::AudioMetrics::default();
        let init_raw = uniform_bytes(
            [1920.0, 1080.0],
            0.0,
            0.016,
            [0.0; 4],
            0,
            &[0.0; 11],
            &default_audio,
            None,
        );
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader_uniform_buffer"),
            size: 224,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, &init_raw);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);

        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn update(&mut self, ctx: &FrameContext) {
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };

        self.frame_count = self.frame_count.wrapping_add(1);
        let time = ctx.elapsed.as_secs_f32();
        let dt = ctx.delta.as_secs_f32();
        let res = [self.width as f32, self.height as f32];

        let (mouse_x, mouse_y) = if let Some(p) = ctx.pointer {
            let px = ((p.0 + 1.0) * 0.5 * res[0]).clamp(0.0, res[0]);
            let py = ((1.0 - p.1) * 0.5 * res[1]).clamp(0.0, res[1]);
            (px, py)
        } else {
            (res[0] * 0.5, res[1] * 0.5)
        };
        let mouse = [mouse_x, mouse_y, 0.0, 0.0];

        let mut custom_values = [0.0f32; 11];
        if let Some(&val) = self.custom_uniforms.get("custom0") {
            custom_values[0] = val;
        }
        if let Some(&val) = self.custom_uniforms.get("custom1") {
            custom_values[1] = val;
        }
        if let Some(&val) = self.custom_uniforms.get("custom2") {
            custom_values[2] = val;
        }
        for (idx, name) in self.uniform_slots.iter().enumerate().take(11) {
            if let Some(&val) = self.custom_uniforms.get(name) {
                custom_values[idx] = val;
            }
        }

        let audio_metrics = ctx
            .spectrum
            .map(wallrs_audio::AudioMetrics::from_spectrum)
            .unwrap_or_default();
        let raw = uniform_bytes(
            res,
            time,
            dt,
            mouse,
            self.frame_count,
            &custom_values,
            &audio_metrics,
            ctx.spectrum,
        );
        ctx.queue.write_buffer(uniform_buffer, 0, &raw);
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let (Some(pipeline), Some(bind_group)) = (&self.pipeline, &self.bind_group) else {
            return;
        };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shader_wallpaper_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }

    fn set_property(&mut self, key: &str, value: PropertyValue) -> Result<(), RendererError> {
        if key == "fps"
            && let PropertyValue::Number(num) = value
        {
            self.target_fps = if num <= 0.0 { None } else { Some(num as f64) };
            return Ok(());
        }
        if let PropertyValue::Number(num) = value {
            if !self.uniform_slots.contains(&key.to_string()) && self.uniform_slots.len() < 11 {
                self.uniform_slots.push(key.to_string());
            }
            self.custom_uniforms.insert(key.to_string(), num);
            return Ok(());
        }
        Err(RendererError::InvalidPropertyValue(
            "Shader custom properties must be floating point numbers".into(),
        ))
    }

    fn target_fps(&self) -> Option<f64> {
        self.target_fps
    }

    fn wants_pointer(&self) -> bool {
        if let Some(src) = &self.raw_source {
            src.contains("mouse") || src.contains("iMouse") || src.contains("u_mouse")
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_renderer_lifecycle() {
        let mut renderer = ShaderRenderer::new();
        assert!(!renderer.wants_pointer());
        renderer.resize(1920, 1080);
        assert_eq!(renderer.width, 1920);
        assert_eq!(renderer.height, 1080);

        renderer.raw_source = Some("void mainImage(...) { vec2 m = iMouse.xy; }".into());
        assert!(renderer.wants_pointer());
    }

    #[test]
    fn test_uniform_bytes_layout() {
        let metrics = wallrs_audio::AudioMetrics {
            bass: 0.95,
            mid: 0.55,
            treble: 0.25,
            volume: 0.60,
        };
        let mut bands = [0.0f32; 32];
        bands[0] = 0.95;
        bands[15] = 0.55;
        bands[31] = 0.25;

        let custom_vals = [
            1.23, 4.56, 7.89, // custom0..2
            0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, // custom_extra
        ];

        let bytes = uniform_bytes(
            [1920.0, 1080.0],
            10.5,
            0.016,
            [100.0, 200.0, 0.0, 0.0],
            42,
            &custom_vals,
            &metrics,
            Some(&bands),
        );
        assert_eq!(bytes.len(), 224);

        let w = f32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let h = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let time = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        let frame = u32::from_ne_bytes(bytes[32..36].try_into().unwrap());
        let c0 = f32::from_ne_bytes(bytes[36..40].try_into().unwrap());
        let c1 = f32::from_ne_bytes(bytes[40..44].try_into().unwrap());
        let c2 = f32::from_ne_bytes(bytes[44..48].try_into().unwrap());
        let bass = f32::from_ne_bytes(bytes[48..52].try_into().unwrap());
        let mid = f32::from_ne_bytes(bytes[52..56].try_into().unwrap());
        let treble = f32::from_ne_bytes(bytes[56..60].try_into().unwrap());
        let vol = f32::from_ne_bytes(bytes[60..64].try_into().unwrap());
        let b0 = f32::from_ne_bytes(bytes[64..68].try_into().unwrap());
        let b15 = f32::from_ne_bytes(bytes[64 + 15 * 4..64 + 16 * 4].try_into().unwrap());
        let b31 = f32::from_ne_bytes(bytes[64 + 31 * 4..64 + 32 * 4].try_into().unwrap());

        // Check custom_extra at offset 192..224
        let extra0 = f32::from_ne_bytes(bytes[192..196].try_into().unwrap());
        let extra7 = f32::from_ne_bytes(bytes[192 + 7 * 4..192 + 8 * 4].try_into().unwrap());

        assert_eq!(w, 1920.0);
        assert_eq!(h, 1080.0);
        assert_eq!(time, 10.5);
        assert_eq!(frame, 42);
        assert!((c0 - 1.23).abs() < 1e-5);
        assert!((c1 - 4.56).abs() < 1e-5);
        assert!((c2 - 7.89).abs() < 1e-5);
        assert_eq!(bass, 0.95);
        assert_eq!(mid, 0.55);
        assert_eq!(treble, 0.25);
        assert_eq!(vol, 0.60);
        assert_eq!(b0, 0.95);
        assert_eq!(b15, 0.55);
        assert_eq!(b31, 0.25);
        assert!((extra0 - 0.1).abs() < 1e-5);
        assert!((extra7 - 0.8).abs() < 1e-5);
    }

    #[test]
    fn test_shader_target_fps_and_dynamic_uniforms() {
        let mut renderer = ShaderRenderer::new();
        // Default target_fps is 60.0
        assert_eq!(renderer.target_fps(), Some(60.0));

        // Setting fps property dynamically
        renderer
            .set_property("fps", PropertyValue::Number(30.0))
            .unwrap();
        assert_eq!(renderer.target_fps(), Some(30.0));

        renderer
            .set_property("fps", PropertyValue::Number(0.0))
            .unwrap();
        assert_eq!(renderer.target_fps(), None);

        // Setting named properties
        renderer
            .set_property("speed", PropertyValue::Number(2.5))
            .unwrap();
        renderer
            .set_property("glow", PropertyValue::Number(0.8))
            .unwrap();

        assert_eq!(renderer.custom_uniforms.get("speed"), Some(&2.5));
        assert_eq!(renderer.custom_uniforms.get("glow"), Some(&0.8));
        assert!(renderer.uniform_slots.contains(&"speed".to_string()));
        assert!(renderer.uniform_slots.contains(&"glow".to_string()));

        // Injected named helpers
        let wgsl_helpers = prepare_wgsl_with_uniforms(
            "@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(speed(), glow(), 0.0, 1.0); }",
            &renderer.uniform_slots,
        );
        assert!(wgsl_helpers.contains("fn speed() -> f32"));
        assert!(wgsl_helpers.contains("fn glow() -> f32"));
    }

    #[test]
    fn test_translate_shadertoy_glsl() {
        let glsl = r#"
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    fragColor = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0);
}
"#;
        let wgsl = translate_shadertoy_glsl_to_wgsl(glsl).expect("failed to translate GLSL");
        assert!(wgsl.contains("fn main("));
    }

    #[test]
    fn test_prepare_wgsl_auto_inject_uniforms() {
        let raw = r#"
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(u_params.time, 0.0, 0.0, 1.0);
}
"#;
        let prepared = prepare_wgsl(raw);
        assert!(prepared.contains("struct ShaderUniforms"));
        assert!(prepared.contains("var<uniform> u_params: ShaderUniforms;"));
    }

    #[test]
    fn test_load_sample_manifests() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let aurora_toml = manifest_dir.join("examples/aurora-shader/wallpaper.toml");
        let manifest_aurora =
            WallpaperManifest::from_file(&aurora_toml).expect("read aurora manifest");
        let renderer_aurora =
            ShaderRenderer::from_manifest(&manifest_aurora, aurora_toml.parent().unwrap())
                .expect("aurora renderer creation");
        assert!(!renderer_aurora.is_glsl);
        assert!(renderer_aurora.raw_source.is_some());

        let plasma_toml = manifest_dir.join("examples/shadertoy-plasma/wallpaper.toml");
        let manifest_plasma =
            WallpaperManifest::from_file(&plasma_toml).expect("read plasma manifest");
        let renderer_plasma =
            ShaderRenderer::from_manifest(&manifest_plasma, plasma_toml.parent().unwrap())
                .expect("plasma renderer creation");
        assert!(renderer_plasma.is_glsl);
        assert!(renderer_plasma.raw_source.is_some());

        let wgsl = translate_shadertoy_glsl_to_wgsl(renderer_plasma.raw_source.as_ref().unwrap())
            .expect("plasma GLSL translates cleanly to WGSL");
        assert!(wgsl.contains("fn main("));

        let tunnel_toml = manifest_dir.join("examples/shadertoy-tunnel/wallpaper.toml");
        let manifest_tunnel =
            WallpaperManifest::from_file(&tunnel_toml).expect("read tunnel manifest");
        let renderer_tunnel =
            ShaderRenderer::from_manifest(&manifest_tunnel, tunnel_toml.parent().unwrap())
                .expect("tunnel renderer creation");
        assert!(renderer_tunnel.is_glsl);
        assert!(renderer_tunnel.raw_source.is_some());

        let wgsl_tunnel =
            translate_shadertoy_glsl_to_wgsl(renderer_tunnel.raw_source.as_ref().unwrap())
                .expect("tunnel GLSL translates cleanly to WGSL");
        assert!(wgsl_tunnel.contains("fn main("));

        let visualizer_toml = manifest_dir.join("examples/audio-visualizer/wallpaper.toml");
        let manifest_vis =
            WallpaperManifest::from_file(&visualizer_toml).expect("read visualizer manifest");
        let renderer_vis =
            ShaderRenderer::from_manifest(&manifest_vis, visualizer_toml.parent().unwrap())
                .expect("visualizer renderer creation");
        assert!(!renderer_vis.is_glsl);
        assert!(renderer_vis.raw_source.is_some());

        let wgsl_source = renderer_vis.raw_source.as_ref().unwrap();
        let module =
            naga::front::wgsl::parse_str(wgsl_source).expect("visualizer WGSL parse failed");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        validator
            .validate(&module)
            .expect("visualizer Naga WGSL validation failed");
    }

    #[test]
    fn test_validate_shader_source() {
        let valid_wgsl = r#"
@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(uv.x, uv.y, 0.5, 1.0);
}
"#;
        assert!(validate_shader_source(valid_wgsl, false, &[]).is_ok());

        let invalid_wgsl = "this is not valid wgsl code syntax";
        assert!(validate_shader_source(invalid_wgsl, false, &[]).is_err());

        let valid_glsl = r#"
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord.xy / iResolution.xy;
    fragColor = vec4(uv, 0.5, 1.0);
}
"#;
        assert!(validate_shader_source(valid_glsl, true, &[]).is_ok());
    }

    #[test]
    fn test_shader_renderer_factory() {
        use wallrs_render::RendererFactory;
        let factory = ShaderRendererFactory;
        assert!(factory.supports_type("shader"));
        assert!(!factory.supports_type("image"));

        let manifest = WallpaperManifest::from_toml_str(
            r#"
            [wallpaper]
            name = "test-shader"
            type = "shader"

            [shader]
            entry = "does_not_exist.wgsl"
            "#,
        )
        .unwrap();

        assert!(factory.validate(&manifest, Path::new(".")).is_err());
        let caps = factory.capabilities(&manifest);
        assert!(caps.needs_audio_spectrum);
        assert!(!caps.produces_audio);
    }
}
