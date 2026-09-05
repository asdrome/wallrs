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
};

@group(0) @binding(0)
var<uniform> u_params: ShaderUniforms;
"#;

fn uniform_bytes(
    resolution: [f32; 2],
    time: f32,
    time_delta: f32,
    mouse: [f32; 4],
    frame: u32,
    custom: [f32; 3],
) -> [u8; 48] {
    let mut bytes = [0u8; 48];
    bytes[0..4].copy_from_slice(&resolution[0].to_ne_bytes());
    bytes[4..8].copy_from_slice(&resolution[1].to_ne_bytes());
    bytes[8..12].copy_from_slice(&time.to_ne_bytes());
    bytes[12..16].copy_from_slice(&time_delta.to_ne_bytes());
    bytes[16..20].copy_from_slice(&mouse[0].to_ne_bytes());
    bytes[20..24].copy_from_slice(&mouse[1].to_ne_bytes());
    bytes[24..28].copy_from_slice(&mouse[2].to_ne_bytes());
    bytes[28..32].copy_from_slice(&mouse[3].to_ne_bytes());
    bytes[32..36].copy_from_slice(&frame.to_ne_bytes());
    bytes[36..40].copy_from_slice(&custom[0].to_ne_bytes());
    bytes[40..44].copy_from_slice(&custom[1].to_ne_bytes());
    bytes[44..48].copy_from_slice(&custom[2].to_ne_bytes());
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
};

#define iResolution vec3(iResolution2D, 1.0)

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
    if !source.contains("ShaderUniforms") && !source.contains("var<uniform>") {
        format!("{STANDARD_UNIFORM_WGSL}\n{source}")
    } else {
        source.to_string()
    }
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

        Ok(Self {
            entry_path: Some(full_path),
            raw_source: Some(content),
            is_glsl,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            custom_uniforms: shader_config.uniforms.clone(),
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
            let prepared = prepare_wgsl(source);
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

        let init_raw = uniform_bytes([1920.0, 1080.0], 0.0, 0.016, [0.0; 4], 0, [0.0, 0.0, 0.0]);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader_uniform_buffer"),
            size: 48,
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

        let c0 = self.custom_uniforms.get("custom0").copied().unwrap_or(0.0);
        let c1 = self.custom_uniforms.get("custom1").copied().unwrap_or(0.0);
        let c2 = self.custom_uniforms.get("custom2").copied().unwrap_or(0.0);

        let raw = uniform_bytes(res, time, dt, mouse, self.frame_count, [c0, c1, c2]);
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
        if let PropertyValue::Number(num) = value {
            self.custom_uniforms.insert(key.to_string(), num);
            return Ok(());
        }
        Err(RendererError::InvalidPropertyValue(
            "Shader custom properties must be floating point numbers".into(),
        ))
    }
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

    #[test]
    fn test_uniform_bytes_layout() {
        let bytes = uniform_bytes(
            [1920.0, 1080.0],
            10.5,
            0.016,
            [100.0, 200.0, 0.0, 0.0],
            42,
            [1.23, 4.56, 7.89],
        );
        assert_eq!(bytes.len(), 48);

        let w = f32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let h = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let time = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        let frame = u32::from_ne_bytes(bytes[32..36].try_into().unwrap());
        let c0 = f32::from_ne_bytes(bytes[36..40].try_into().unwrap());

        assert_eq!(w, 1920.0);
        assert_eq!(h, 1080.0);
        assert_eq!(time, 10.5);
        assert_eq!(frame, 42);
        assert!((c0 - 1.23).abs() < 1e-5);
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
    }
}
