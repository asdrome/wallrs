use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{PropertyValue, WallpaperManifest};
use wallrs_render::{FrameContext, RendererError, WallpaperRenderer};

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("Video manifest missing video configuration")]
    NoConfig,

    #[error("Video file not found at {0:?}")]
    FileNotFound(PathBuf),

    #[error("Failed to initialize MPV: {0}")]
    MpvInit(String),

    #[error("Failed to create MPV render context: error code {0}")]
    RenderContextCreate(i32),
}

const VIDEO_SHADER_SRC: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0)
var t_video: texture_2d<f32>;

@group(0) @binding(1)
var s_video: sampler;

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
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, -p.y * 0.5 + 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_video, s_video, in.uv);
}
"#;

/// Video wallpaper renderer powered by libmpv (software rendering API mode).
pub struct VideoRenderer {
    mpv: Option<libmpv2::Mpv>,
    render_ctx: *mut libmpv2_sys::mpv_render_context,
    video_path: Option<PathBuf>,
    loop_file: bool,
    volume: f64,
    width: u32,
    height: u32,
    cpu_buffer: Vec<u8>,
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    sampler: Option<wgpu::Sampler>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline: Option<wgpu::RenderPipeline>,
    needs_initial_load: bool,
}

// Safety: VideoRenderer owns the mpv instance and render context, which are accessed
// exclusively by the rendering thread in the calloop event loop.
unsafe impl Send for VideoRenderer {}

impl Default for VideoRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoRenderer {
    /// Creates an uninitialized video renderer (useful for testing or fallback).
    pub fn new() -> Self {
        Self {
            mpv: None,
            render_ctx: std::ptr::null_mut(),
            video_path: None,
            loop_file: true,
            volume: 0.0,
            width: 0,
            height: 0,
            cpu_buffer: Vec::new(),
            texture: None,
            texture_view: None,
            sampler: None,
            bind_group_layout: None,
            bind_group: None,
            pipeline: None,
            needs_initial_load: true,
        }
    }

    /// Loads and initializes an MPV video renderer from a wallpaper manifest.
    pub fn from_manifest(
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Self, VideoError> {
        let Some(video_config) = &manifest.video else {
            return Err(VideoError::NoConfig);
        };

        let full_path = if video_config.path.is_absolute() {
            video_config.path.clone()
        } else {
            base_dir.join(&video_config.path)
        };

        if !full_path.exists() {
            return Err(VideoError::FileNotFound(full_path));
        }

        let loop_file = video_config.r#loop.unwrap_or(true);
        let volume = video_config.volume.unwrap_or(0.0) as f64;

        let mpv = libmpv2::Mpv::new().map_err(|e| VideoError::MpvInit(format!("{e:?}")))?;

        // Configure MPV options for live wallpaper embedding
        let _ = mpv.set_property("vo", "libmpv");
        let _ = mpv.set_property("hwdec", "auto-safe");
        if loop_file {
            let _ = mpv.set_property("loop-file", "inf");
        }
        let _ = mpv.set_property("volume", volume);
        if volume <= 0.0 {
            let _ = mpv.set_property("mute", true);
        }

        // Initialize MPV software render context
        let api_type = CString::new("sw").unwrap();
        let mut params = [
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];

        let mut render_ctx: *mut libmpv2_sys::mpv_render_context = std::ptr::null_mut();
        let err = unsafe {
            libmpv2_sys::mpv_render_context_create(
                &mut render_ctx,
                mpv.ctx.as_ptr(),
                params.as_mut_ptr(),
            )
        };

        if err != 0 {
            return Err(VideoError::RenderContextCreate(err));
        }

        // Enqueue video playback
        let path_str = full_path.to_string_lossy();
        if let Err(e) = mpv.command("loadfile", &[&path_str, "replace"]) {
            tracing::warn!(error = ?e, path = ?full_path, "MPV failed initial loadfile command");
        }

        Ok(Self {
            mpv: Some(mpv),
            render_ctx,
            video_path: Some(full_path),
            loop_file,
            volume,
            width: 0,
            height: 0,
            cpu_buffer: Vec::new(),
            texture: None,
            texture_view: None,
            sampler: None,
            bind_group_layout: None,
            bind_group: None,
            pipeline: None,
            needs_initial_load: true,
        })
    }

    pub fn video_path(&self) -> Option<&Path> {
        self.video_path.as_deref()
    }

    pub fn is_looping(&self) -> bool {
        self.loop_file
    }
}

impl WallpaperRenderer for VideoRenderer {
    fn init(
        &mut self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Result<(), RendererError> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video_quad_shader"),
            source: wgpu::ShaderSource::Wgsl(VIDEO_SHADER_SRC.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video_render_pipeline"),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        self.pipeline = Some(pipeline);
        self.bind_group_layout = Some(bind_group_layout);
        self.sampler = Some(sampler);

        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            let needed_bytes = (width as usize) * (height as usize) * 4;
            self.cpu_buffer.resize(needed_bytes, 0);
            // Invalidate GPU texture so update() rebuilds it with new dimensions
            self.texture = None;
            self.texture_view = None;
            self.bind_group = None;
            self.needs_initial_load = true;
        }
    }

    fn update(&mut self, ctx: &FrameContext) {
        // Drain any pending MPV events without blocking
        if let Some(mpv) = &self.mpv {
            unsafe {
                loop {
                    let event = libmpv2_sys::mpv_wait_event(mpv.ctx.as_ptr(), 0.0);
                    if (*event).event_id == libmpv2_sys::mpv_event_id_MPV_EVENT_NONE {
                        break;
                    }
                }
            }
        }

        if self.width == 0 || self.height == 0 {
            return;
        }

        // Recreate WGPU texture & bind group if invalidated by resize
        if self.texture.is_none() {
            let Some(bgl) = &self.bind_group_layout else {
                return;
            };
            let Some(sampler) = &self.sampler else {
                return;
            };

            let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video_frame_texture"),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video_bind_group"),
                layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });

            self.texture = Some(texture);
            self.texture_view = Some(view);
            self.bind_group = Some(bind_group);
        }

        // Query MPV software renderer for frame availability
        if !self.render_ctx.is_null() {
            let flags = unsafe { libmpv2_sys::mpv_render_context_update(self.render_ctx) };
            let has_new_frame =
                (flags & (libmpv2_sys::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64) != 0)
                    || self.needs_initial_load;

            if has_new_frame {
                self.needs_initial_load = false;

                let mut size: [std::os::raw::c_int; 2] = [self.width as _, self.height as _];
                let fmt = CString::new("rgba").unwrap();
                let mut stride: usize = self.width as usize * 4;

                let mut render_params = [
                    libmpv2_sys::mpv_render_param {
                        type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                        data: size.as_mut_ptr() as *mut c_void,
                    },
                    libmpv2_sys::mpv_render_param {
                        type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                        data: fmt.as_ptr() as *mut c_void,
                    },
                    libmpv2_sys::mpv_render_param {
                        type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                        data: &mut stride as *mut usize as *mut c_void,
                    },
                    libmpv2_sys::mpv_render_param {
                        type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                        data: self.cpu_buffer.as_mut_ptr() as *mut c_void,
                    },
                    libmpv2_sys::mpv_render_param {
                        type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                        data: std::ptr::null_mut(),
                    },
                ];

                let res = unsafe {
                    libmpv2_sys::mpv_render_context_render(
                        self.render_ctx,
                        render_params.as_mut_ptr(),
                    )
                };

                if res == 0
                    && let Some(texture) = &self.texture
                {
                    ctx.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &self.cpu_buffer,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(self.width * 4),
                            rows_per_image: Some(self.height),
                        },
                        wgpu::Extent3d {
                            width: self.width,
                            height: self.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let (Some(pipeline), Some(bind_group)) = (&self.pipeline, &self.bind_group) else {
            return;
        };

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video_wallpaper_render_pass"),
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
        rpass.draw(0..6, 0..1);
    }

    fn set_property(&mut self, key: &str, value: PropertyValue) -> Result<(), RendererError> {
        let Some(mpv) = &self.mpv else {
            return Ok(());
        };

        match (key, value) {
            ("pause", PropertyValue::Bool(b)) => {
                let _ = mpv.set_property("pause", b);
            }
            ("play", PropertyValue::Bool(b)) => {
                let _ = mpv.set_property("pause", !b);
            }
            ("mute", PropertyValue::Bool(b)) => {
                let _ = mpv.set_property("mute", b);
            }
            ("volume", PropertyValue::Number(n)) => {
                self.volume = n as f64;
                let _ = mpv.set_property("volume", self.volume);
                let _ = mpv.set_property("mute", self.volume <= 0.0);
            }
            ("seek", PropertyValue::Number(n)) => {
                let _ = mpv.command("seek", &[&n.to_string(), "relative"]);
            }
            ("loop", PropertyValue::Bool(b)) => {
                self.loop_file = b;
                let _ = mpv.set_property("loop-file", if b { "inf" } else { "no" });
            }
            (k, _) => {
                return Err(RendererError::PropertyNotFound(k.to_string()));
            }
        }

        Ok(())
    }

    fn teardown(&mut self) {
        if !self.render_ctx.is_null() {
            unsafe {
                libmpv2_sys::mpv_render_context_free(self.render_ctx);
            }
            self.render_ctx = std::ptr::null_mut();
        }
        self.mpv = None;
    }
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        self.teardown();
    }
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
        assert_eq!(renderer.cpu_buffer.len(), 3840 * 2160 * 4);
    }

    #[test]
    fn test_mpv_instance() {
        let mpv = libmpv2::Mpv::new();
        assert!(mpv.is_ok());
    }

    #[test]
    fn test_mpv_sw_render_context() {
        let mpv = libmpv2::Mpv::new().expect("create mpv");
        let mpv_handle = mpv.ctx.as_ptr();

        let api_type = CString::new("sw").unwrap();
        let mut params = [
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];

        let mut render_ctx: *mut libmpv2_sys::mpv_render_context = std::ptr::null_mut();
        let err = unsafe {
            libmpv2_sys::mpv_render_context_create(&mut render_ctx, mpv_handle, params.as_mut_ptr())
        };
        assert_eq!(err, 0, "mpv_render_context_create failed with code {err}");
        assert!(!render_ctx.is_null());

        let mut buffer = vec![0u8; 64 * 64 * 4];
        let mut size: [std::os::raw::c_int; 2] = [64, 64];
        let fmt = CString::new("rgba").unwrap();
        let mut stride: usize = 64 * 4;

        let mut render_params = [
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                data: fmt.as_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                data: &mut stride as *mut usize as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                data: buffer.as_mut_ptr() as *mut c_void,
            },
            libmpv2_sys::mpv_render_param {
                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];

        let render_err = unsafe {
            libmpv2_sys::mpv_render_context_render(render_ctx, render_params.as_mut_ptr())
        };
        assert_eq!(
            render_err, 0,
            "mpv_render_context_render failed: {render_err}"
        );

        unsafe {
            libmpv2_sys::mpv_render_context_free(render_ctx);
        }
    }

    #[test]
    fn test_video_renderer_property_controls() {
        let mut renderer = VideoRenderer::new();
        let mpv = libmpv2::Mpv::new().expect("create mpv");
        renderer.mpv = Some(mpv);

        assert!(
            renderer
                .set_property("volume", PropertyValue::Number(75.0))
                .is_ok()
        );
        assert_eq!(renderer.volume, 75.0);

        assert!(
            renderer
                .set_property("pause", PropertyValue::Bool(true))
                .is_ok()
        );
        assert!(
            renderer
                .set_property("play", PropertyValue::Bool(true))
                .is_ok()
        );
        assert!(
            renderer
                .set_property("mute", PropertyValue::Bool(false))
                .is_ok()
        );
        assert!(
            renderer
                .set_property("loop", PropertyValue::Bool(false))
                .is_ok()
        );
        assert!(!renderer.is_looping());
        assert!(
            renderer
                .set_property("invalid_prop", PropertyValue::Bool(true))
                .is_err()
        );
    }

    #[test]
    fn test_video_renderer_from_manifest() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/video-wallpaper/wallpaper.toml");

        let manifest =
            wallrs_proto::WallpaperManifest::from_file(&manifest_path).expect("parse manifest");
        let base_dir = manifest_path.parent().unwrap();

        let renderer = VideoRenderer::from_manifest(&manifest, base_dir)
            .expect("create video renderer from manifest");

        assert!(renderer.video_path().is_some());
        assert!(renderer.is_looping());
        assert_eq!(renderer.volume, 0.0);
    }
}
