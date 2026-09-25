use gst::prelude::*;
use gst_video::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
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

    #[error("Failed to initialize GStreamer: {0}")]
    GstInit(String),

    #[error("Failed to create GStreamer pipeline: {0}")]
    PipelineCreate(String),

    #[error("Failed to configure appsink: {0}")]
    AppSinkConfig(String),
}

/// Factory plugin for constructing and validating video wallpapers.
#[derive(Debug, Clone, Copy, Default)]
pub struct VideoRendererFactory;

impl VideoRendererFactory {
    pub fn new() -> Self {
        Self
    }
}

impl wallrs_render::RendererFactory for VideoRendererFactory {
    fn create_renderer(
        &self,
        manifest: &WallpaperManifest,
        base_dir: &Path,
    ) -> Result<Box<dyn WallpaperRenderer>, RendererError> {
        let renderer = VideoRenderer::from_manifest(manifest, base_dir)
            .map_err(|e| RendererError::InitFailed(e.to_string()))?;
        Ok(Box::new(renderer))
    }

    fn supports_type(&self, type_name: &str) -> bool {
        type_name == "video"
    }

    fn validate(&self, manifest: &WallpaperManifest, base_dir: &Path) -> Result<(), RendererError> {
        let Some(video_config) = &manifest.video else {
            return Err(RendererError::ValidationFailed(
                "Manifest declares type 'video' but is missing [video] configuration block".into(),
            ));
        };
        let full_path = if video_config.path.is_absolute() {
            video_config.path.clone()
        } else {
            base_dir.join(&video_config.path)
        };
        if !full_path.exists() {
            return Err(RendererError::ValidationFailed(format!(
                "Video source file does not exist: {full_path:?}"
            )));
        }
        Ok(())
    }

    fn capabilities(&self, _manifest: &WallpaperManifest) -> wallrs_render::RendererCapabilities {
        wallrs_render::RendererCapabilities {
            needs_audio_spectrum: false,
            produces_audio: true,
        }
    }
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

/// Video wallpaper renderer powered by GStreamer (appsink hardware/software decoding).
pub struct VideoRenderer {
    pipeline: Option<gst::Element>,
    appsink: Option<gst_app::AppSink>,
    bus: Option<gst::Bus>,
    video_path: Option<PathBuf>,
    loop_file: bool,
    volume: f64,
    width: u32,
    height: u32,
    video_width: u32,
    video_height: u32,
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    sampler: Option<wgpu::Sampler>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline_gpu: Option<wgpu::RenderPipeline>,
    dirty: bool,
    cached_fps: Option<f64>,
    cached_caps: Option<gst::Caps>,
    cached_video_info: Option<gst_video::VideoInfo>,
    is_paused: bool,
}

/// Builds an optimized video sink element.
///
/// On systems with VA-API hardware acceleration (Intel Gen Graphics / AMD),
/// `vapostproc` converts decoded NV12/YUV video frames directly into RGBA
/// on the GPU, avoiding CPU-intensive software color conversion and scaling.
///
/// If VA-API is unavailable, it gracefully falls back to a multithreaded CPU
/// `videoscale` + `videoconvert` element (`n-threads=0`) feeding `appsink`.
fn create_video_sink(appsink: &gst_app::AppSink) -> Result<gst::Element, VideoError> {
    if gst::ElementFactory::find("vapostproc").is_some() {
        let bin = gst::Bin::new();
        let vapostproc = gst::ElementFactory::make("vapostproc")
            .build()
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        bin.add_many([&vapostproc, appsink.upcast_ref()])
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        gst::Element::link_many([&vapostproc, appsink.upcast_ref()])
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        let sink_pad = vapostproc
            .static_pad("sink")
            .ok_or_else(|| VideoError::PipelineCreate("vapostproc missing sink pad".into()))?;
        let ghost_pad = gst::GhostPad::with_target(&sink_pad)
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
        ghost_pad
            .set_active(true)
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
        bin.add_pad(&ghost_pad)
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        return Ok(bin.upcast());
    }

    // Fallback path: multithreaded CPU videoscale + videoconvert directly feeding appsink
    let bin = gst::Bin::new();
    let videoscale = gst::ElementFactory::make("videoscale")
        .property("n-threads", 0u32)
        .build()
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
    let videoconvert = gst::ElementFactory::make("videoconvert")
        .property("n-threads", 0u32)
        .build()
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

    bin.add_many([&videoscale, &videoconvert, appsink.upcast_ref()])
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
    gst::Element::link_many([&videoscale, &videoconvert, appsink.upcast_ref()])
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

    let sink_pad = videoscale
        .static_pad("sink")
        .ok_or_else(|| VideoError::PipelineCreate("videoscale missing sink pad".into()))?;
    let ghost_pad = gst::GhostPad::with_target(&sink_pad)
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
    ghost_pad
        .set_active(true)
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;
    bin.add_pad(&ghost_pad)
        .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

    Ok(bin.upcast())
}

// Safety: VideoRenderer owns the GStreamer elements, which are thread-safe and
// accessed exclusively by the rendering thread in the calloop event loop.
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
            pipeline: None,
            appsink: None,
            bus: None,
            video_path: None,
            loop_file: true,
            volume: 0.0,
            width: 0,
            height: 0,
            video_width: 0,
            video_height: 0,
            texture: None,
            texture_view: None,
            sampler: None,
            bind_group_layout: None,
            bind_group: None,
            pipeline_gpu: None,
            dirty: true,
            cached_fps: None,
            cached_caps: None,
            cached_video_info: None,
            is_paused: false,
        }
    }

    /// Loads and initializes a GStreamer video renderer from a wallpaper manifest.
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

        gst::init().map_err(|e| VideoError::GstInit(e.to_string()))?;

        let canonical = std::fs::canonicalize(&full_path).unwrap_or_else(|_| full_path.clone());
        let uri = format!("file://{}", canonical.display());

        // Configure video appsink with RGBA pixel caps
        let video_caps = gst_video::VideoCapsBuilder::for_encoding("video/x-raw")
            .format(gst_video::VideoFormat::Rgba)
            .build();

        let appsink = gst_app::AppSink::builder()
            .caps(&video_caps)
            .drop(true)
            .max_buffers(1)
            .sync(true)
            .build();

        let video_sink = create_video_sink(&appsink)?;

        // Attempt playbin3, falling back to playbin
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin").build())
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        playbin.set_property("uri", uri.as_str());
        playbin.set_property("video-sink", &video_sink);

        let clamped_volume = volume.clamp(0.0, 100.0);
        let has_audio = clamped_volume > 0.0;

        if has_audio {
            playbin.set_property_from_str("flags", "video+audio+soft-volume+buffering");
            let audio_sink = gst::ElementFactory::make("pipewiresink")
                .build()
                .or_else(|_| gst::ElementFactory::make("autoaudiosink").build())
                .ok();
            if let Some(asink) = audio_sink {
                playbin.set_property("audio-sink", &asink);
            }
            playbin.set_property("volume", clamped_volume / 100.0);
            playbin.set_property("mute", false);
        } else {
            // Highly optimized wallpaper mode: video-only decoding, buffering,
            // no audio pipeline, no subtitle processing, no software deinterlacing/colorbalance.
            playbin.set_property_from_str("flags", "video+buffering");
            if let Ok(fakesink) = gst::ElementFactory::make("fakesink").build() {
                playbin.set_property("audio-sink", &fakesink);
            }
            playbin.set_property("volume", 0.0);
            playbin.set_property("mute", true);
        }

        let _ = playbin.set_state(gst::State::Playing);
        let bus = playbin.bus();

        Ok(Self {
            pipeline: Some(playbin),
            appsink: Some(appsink),
            bus,
            video_path: Some(full_path),
            loop_file,
            volume: clamped_volume,
            width: 0,
            height: 0,
            video_width: 0,
            video_height: 0,
            texture: None,
            texture_view: None,
            sampler: None,
            bind_group_layout: None,
            bind_group: None,
            pipeline_gpu: None,
            dirty: true,
            cached_fps: None,
            cached_caps: None,
            cached_video_info: None,
            is_paused: false,
        })
    }

    pub fn video_path(&self) -> Option<&Path> {
        self.video_path.as_deref()
    }

    pub fn is_looping(&self) -> bool {
        self.loop_file
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
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

        self.pipeline_gpu = Some(pipeline);
        self.bind_group_layout = Some(bind_group_layout);
        self.sampler = Some(sampler);

        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.dirty = true;

            // Cap the maximum appsink resolution to the output's physical dimensions.
            // When a 4K video is played on a 1080p display, this informs the hardware post-processor
            // (vapostproc) to downscale on the GPU before memory download, reducing PCIe bandwidth
            // from 1 GB/s (33.2 MB/frame) down to 250 MB/s (8.3 MB/frame) and slashing CPU usage.
            if width > 0
                && height > 0
                && let Some(appsink) = &self.appsink
            {
                let video_caps = gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .field("width", gst::IntRange::new(1, width as i32))
                    .field("height", gst::IntRange::new(1, height as i32))
                    .build();
                appsink.set_caps(Some(&video_caps));
            }
        }
    }

    fn update(&mut self, ctx: &FrameContext) {
        // Drain bus messages (looping on EOS or handling runtime errors)
        if let Some(bus) = &self.bus {
            while let Some(msg) = bus.pop() {
                match msg.view() {
                    gst::MessageView::Eos(..) => {
                        if self.loop_file
                            && let Some(pipeline) = &self.pipeline
                        {
                            let _ = pipeline.seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                gst::ClockTime::ZERO,
                            );
                            self.dirty = true;
                        }
                    }
                    gst::MessageView::Error(err) => {
                        tracing::warn!(
                            error = %err.error(),
                            debug = ?err.debug(),
                            "GStreamer video playback error"
                        );
                    }
                    _ => {}
                }
            }
        }

        let Some(appsink) = &self.appsink else {
            return;
        };

        // Non-blocking attempt to retrieve the latest video frame
        if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::ZERO)
            && let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps())
        {
            let video_info = if let Some(info) = &self.cached_video_info
                && self.cached_caps.as_deref() == Some(caps)
            {
                info.clone()
            } else if let Ok(info) = gst_video::VideoInfo::from_caps(caps) {
                self.cached_caps = Some(caps.to_owned());
                self.cached_video_info = Some(info.clone());
                if self.cached_fps.is_none() {
                    let fps_frac = info.fps();
                    if fps_frac.numer() > 0 && fps_frac.denom() > 0 {
                        let rate = fps_frac.numer() as f64 / fps_frac.denom() as f64;
                        if rate.is_finite() && (1.0..=240.0).contains(&rate) {
                            self.cached_fps = Some(rate);
                        }
                    }
                }
                info
            } else {
                return;
            };

            if let Ok(video_frame) =
                gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &video_info)
                && let Ok(data) = video_frame.plane_data(0)
            {
                let w = video_info.width();
                let h = video_info.height();
                let stride = video_frame.plane_stride()[0] as u32;

                if w > 0 && h > 0 {
                    // Recreate GPU texture if dimensions changed or first allocation
                    if self.video_width != w || self.video_height != h || self.texture.is_none() {
                        self.video_width = w;
                        self.video_height = h;

                        let Some(bgl) = &self.bind_group_layout else {
                            return;
                        };
                        let Some(sampler) = &self.sampler else {
                            return;
                        };

                        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("video_frame_texture"),
                            size: wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8UnormSrgb,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
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

                    // Upload pixel data directly from GStreamer buffer into WGPU texture
                    if let Some(texture) = &self.texture {
                        ctx.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(stride),
                                rows_per_image: Some(h),
                            },
                            wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: 1,
                            },
                        );
                        self.dirty = true;
                    }
                }
            }
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        self.dirty = false;
        let (Some(pipeline), Some(bind_group)) = (&self.pipeline_gpu, &self.bind_group) else {
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
        match (key, value) {
            ("pause", PropertyValue::Bool(b)) => {
                self.is_paused = b;
                if let Some(pipeline) = &self.pipeline {
                    let state = if b {
                        gst::State::Paused
                    } else {
                        gst::State::Playing
                    };
                    let _ = pipeline.set_state(state);
                }
                if !b {
                    self.dirty = true;
                }
            }
            ("play", PropertyValue::Bool(b)) => {
                self.is_paused = !b;
                if let Some(pipeline) = &self.pipeline {
                    let state = if !b {
                        gst::State::Paused
                    } else {
                        gst::State::Playing
                    };
                    let _ = pipeline.set_state(state);
                }
                if b {
                    self.dirty = true;
                }
            }
            ("mute", PropertyValue::Bool(b)) => {
                if let Some(pipeline) = &self.pipeline {
                    pipeline.set_property("mute", b);
                }
            }
            ("volume", PropertyValue::Number(n)) => {
                let old_vol = self.volume;
                self.volume = (n as f64).clamp(0.0, 100.0);
                if let Some(pipeline) = &self.pipeline {
                    if old_vol <= 0.0 && self.volume > 0.0 {
                        pipeline
                            .set_property_from_str("flags", "video+audio+soft-volume+buffering");
                        if let Ok(asink) = gst::ElementFactory::make("pipewiresink")
                            .build()
                            .or_else(|_| gst::ElementFactory::make("autoaudiosink").build())
                        {
                            pipeline.set_property("audio-sink", &asink);
                        }
                    } else if old_vol > 0.0 && self.volume <= 0.0 {
                        pipeline.set_property_from_str("flags", "video+buffering");
                        if let Ok(fakesink) = gst::ElementFactory::make("fakesink").build() {
                            pipeline.set_property("audio-sink", &fakesink);
                        }
                    }
                    pipeline.set_property("volume", self.volume / 100.0);
                    pipeline.set_property("mute", self.volume <= 0.0);
                }
            }
            ("seek", PropertyValue::Number(n)) => {
                self.dirty = true;
                if let Some(pipeline) = &self.pipeline {
                    let target_ns = (n as f64 * 1_000_000_000.0).max(0.0) as u64;
                    let _ = pipeline.seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                        gst::ClockTime::from_nseconds(target_ns),
                    );
                }
            }
            ("loop", PropertyValue::Bool(b)) => {
                self.loop_file = b;
            }
            ("speed", PropertyValue::Number(n)) => {
                let rate = (n as f64).clamp(0.1, 10.0);
                if let Some(pipeline) = &self.pipeline {
                    let _ = pipeline.seek(
                        rate,
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::SeekType::None,
                        gst::ClockTime::NONE,
                        gst::SeekType::None,
                        gst::ClockTime::NONE,
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn target_fps(&self) -> Option<f64> {
        self.cached_fps
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wallrs_render::RendererFactory;

    #[test]
    fn test_video_renderer_factory() {
        let factory = VideoRendererFactory::new();
        assert!(factory.supports_type("video"));
        assert!(!factory.supports_type("shader"));
        assert!(!factory.supports_type("image"));

        let manifest = WallpaperManifest::from_toml_str(
            r#"
            [wallpaper]
            name = "test-video"
            type = "video"

            [video]
            path = "nonexistent.mp4"
            "#,
        )
        .unwrap();

        let caps = factory.capabilities(&manifest);
        assert!(!caps.needs_audio_spectrum);
        assert!(caps.produces_audio);

        let validation = factory.validate(&manifest, Path::new("/tmp"));
        assert!(validation.is_err());
    }

    #[test]
    fn test_video_renderer_property_controls() {
        let mut renderer = VideoRenderer::new();
        assert_eq!(renderer.volume(), 0.0);
        assert!(!renderer.is_paused());

        renderer
            .set_property("volume", PropertyValue::Number(65.0))
            .unwrap();
        assert_eq!(renderer.volume(), 65.0);

        renderer
            .set_property("pause", PropertyValue::Bool(true))
            .unwrap();
        assert!(renderer.is_paused());

        renderer
            .set_property("play", PropertyValue::Bool(true))
            .unwrap();
        assert!(!renderer.is_paused());

        renderer
            .set_property("loop", PropertyValue::Bool(false))
            .unwrap();
        assert!(!renderer.is_looping());
    }

    #[test]
    fn test_video_renderer_optimization_states() {
        let mut renderer = VideoRenderer::new();
        assert!(renderer.is_dirty());

        renderer.resize(1920, 1080);
        assert_eq!(renderer.width, 1920);
        assert_eq!(renderer.height, 1080);
        assert!(renderer.is_dirty());
    }

    #[test]
    fn test_video_renderer_lifecycle() {
        let renderer = VideoRenderer::new();
        assert!(renderer.video_path().is_none());
        assert!(renderer.is_looping());
        assert_eq!(renderer.volume(), 0.0);
    }

    #[test]
    fn test_video_renderer_from_manifest() {
        let temp_dir = std::env::temp_dir();
        let dummy_video = temp_dir.join("wallrs_test_video.mp4");
        std::fs::write(&dummy_video, b"dummy video content").unwrap();

        let toml_str = format!(
            r#"
            [wallpaper]
            name = "test"
            type = "video"

            [video]
            path = "{}"
            loop = false
            volume = 40.0
            "#,
            dummy_video.display()
        );

        let manifest = WallpaperManifest::from_toml_str(&toml_str).unwrap();
        let renderer = VideoRenderer::from_manifest(&manifest, &temp_dir).unwrap();

        assert_eq!(renderer.volume(), 40.0);
        assert!(!renderer.is_looping());
        assert_eq!(renderer.video_path(), Some(dummy_video.as_path()));

        let _ = std::fs::remove_file(dummy_video);
    }
}
