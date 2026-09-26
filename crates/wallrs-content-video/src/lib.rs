use gst::prelude::*;
use gst_video::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::path::{Path, PathBuf};
use std::str::FromStr;
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
var t_y: texture_2d<f32>;

@group(0) @binding(1)
var t_uv: texture_2d<f32>;

@group(0) @binding(2)
var s_video: sampler;

struct ColorParams {
    y_uv_params: vec4<f32>,   // x: y_offset, y: y_scale, z: uv_offset, w: r_coeff_v
    matrix_params: vec4<f32>, // x: g_coeff_u, y: g_coeff_v, z: b_coeff_u, w: to_linear
};

@group(0) @binding(3)
var<uniform> u_color: ColorParams;

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
    let y_raw = textureSample(t_y, s_video, in.uv).r;
    let uv_raw = textureSample(t_uv, s_video, in.uv).rg;

    var y_offset = u_color.y_uv_params.x;
    var y_scale = u_color.y_uv_params.y;
    var uv_offset = u_color.y_uv_params.z;
    var r_v = u_color.y_uv_params.w;
    var g_u = u_color.matrix_params.x;
    var g_v = u_color.matrix_params.y;
    var b_u = u_color.matrix_params.z;
    var to_linear = u_color.matrix_params.w;

    // Fail-safe: if uniform is uninitialized or 0, fallback to BT.709 studio range + linear
    if (y_scale <= 0.001) {
        y_offset = 0.06274510;
        y_scale = 1.16438356;
        uv_offset = 0.50196078;
        r_v = 1.79274107;
        g_u = 0.21324861;
        g_v = 0.53290933;
        b_u = 2.11240179;
        to_linear = 1.0;
    }

    let y = clamp((y_raw - y_offset) * y_scale, 0.0, 1.0);
    let u = uv_raw.x - uv_offset;
    let v = uv_raw.y - uv_offset;

    var r = clamp(y + r_v * v, 0.0, 1.0);
    var g = clamp(y - g_u * u - g_v * v, 0.0, 1.0);
    var b = clamp(y + b_u * u, 0.0, 1.0);

    // If writing to an sRGB render target, the GPU ROP automatically applies
    // the sRGB gamma transfer function on write. Convert the gamma-encoded video
    // RGB to linear space to prevent double-gamma (washed out / milky look).
    if (to_linear > 0.5) {
        if (r > 0.04045) {
            r = pow((r + 0.055) / 1.055, 2.4);
        } else {
            r = r / 12.92;
        }
        if (g > 0.04045) {
            g = pow((g + 0.055) / 1.055, 2.4);
        } else {
            g = g / 12.92;
        }
        if (b > 0.04045) {
            b = pow((b + 0.055) / 1.055, 2.4);
        } else {
            b = b / 12.92;
        }
    }

    return vec4<f32>(r, g, b, 1.0);
}
"#;

/// Returns the default ITU-R BT.709 limited-range color conversion parameters.
pub(crate) fn default_color_params(is_srgb: bool) -> [f32; 8] {
    [
        16.0 / 255.0,                    // y_offset (Limited range)
        255.0 / 219.0,                   // y_scale
        128.0 / 255.0,                   // uv_offset
        1.7927411,                       // r_coeff_v (BT.709)
        0.2132486,                       // g_coeff_u
        0.5329093,                       // g_coeff_v
        2.1124018,                       // b_coeff_u
        if is_srgb { 1.0 } else { 0.0 }, // to_linear
    ]
}

/// Dynamically calculates YUV to RGB color conversion parameters based on
/// the video stream's negotiated color range (full vs limited/studio) and color matrix.
pub(crate) fn color_params_from_video_info(info: &gst_video::VideoInfo, is_srgb: bool) -> [f32; 8] {
    let colorimetry = info.colorimetry();
    let is_full_range = colorimetry.range() == gst_video::VideoColorRange::Range0_255;
    let is_bt601 = colorimetry.matrix() == gst_video::VideoColorMatrix::Bt601;

    let (y_offset, y_scale, uv_offset) = if is_full_range {
        (0.0, 1.0, 128.0 / 255.0)
    } else {
        (16.0 / 255.0, 255.0 / 219.0, 128.0 / 255.0)
    };

    let (r_v, g_u, g_v, b_u) = match (is_bt601, is_full_range) {
        (true, true) => (1.4020, 0.344136, 0.714136, 1.7720),
        (true, false) => (1.5960268, 0.3917623, 0.8129676, 2.0172321),
        (false, true) => (1.57480, 0.187324, 0.468124, 1.85560),
        (false, false) => (1.7927411, 0.2132486, 0.5329093, 2.1124018),
    };

    let to_linear = if is_srgb { 1.0 } else { 0.0 };

    [y_offset, y_scale, uv_offset, r_v, g_u, g_v, b_u, to_linear]
}

/// Determines whether a GStreamer element is a genuine VA-API hardware video decoder.
///
/// Used to dynamically decide whether `vapostproc` should be injected as `video-filter`.
/// On non-VA decoders (such as NVIDIA `nvdec`, software `avdec`, or `openh264`), injecting
/// `vapostproc` causes pipeline negotiation errors, severe CPU/GPU copy overhead, or crashes.
pub(crate) fn is_vaapi_decoder(elem: &gst::Element) -> bool {
    let Some(factory) = elem.factory() else {
        return false;
    };
    let klass = factory.klass();
    if !klass.contains("Decoder") || !klass.contains("Video") {
        return false;
    }
    let name = factory.name();
    let is_va_name = name.starts_with("va") || name.starts_with("vaapi");
    let is_va_plugin = factory
        .plugin()
        .map(|p| p.name() == "va" || p.name() == "vaapi")
        .unwrap_or(false);

    is_va_name || is_va_plugin
}

/// Checks whether `vapostproc` is available on the system and capable of acquiring
/// a valid VA-API hardware display context (e.g. via `/dev/dri/renderD128`).
pub(crate) fn is_vapostproc_usable() -> bool {
    if let Ok(filter) = gst::ElementFactory::make("vapostproc").build()
        && filter.set_state(gst::State::Ready).is_ok()
    {
        let _ = filter.set_state(gst::State::Null);
        return true;
    }
    false
}

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
    texture_y: Option<wgpu::Texture>,
    texture_y_view: Option<wgpu::TextureView>,
    texture_uv: Option<wgpu::Texture>,
    texture_uv_view: Option<wgpu::TextureView>,
    color_buffer: Option<wgpu::Buffer>,
    cached_color_params: Option<[f32; 8]>,
    target_is_srgb: bool,
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
            texture_y: None,
            texture_y_view: None,
            texture_uv: None,
            texture_uv_view: None,
            color_buffer: None,
            cached_color_params: None,
            target_is_srgb: true,
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

        // Configure video appsink with native hardware NV12 pixel caps
        let video_caps = gst_video::VideoCapsBuilder::for_encoding("video/x-raw")
            .format(gst_video::VideoFormat::Nv12)
            .build();

        let appsink = gst_app::AppSink::builder()
            .caps(&video_caps)
            .drop(true)
            .max_buffers(1)
            .sync(true)
            .build();

        // Attempt playbin (classic, lean thread model), falling back to playbin3
        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin3").build())
            .map_err(|e| VideoError::PipelineCreate(e.to_string()))?;

        playbin.set_property("uri", uri.as_str());

        // Connect appsink directly as video-sink without intermediate bins or redundant queue threads.
        playbin.set_property("video-sink", &appsink);

        let clamped_volume = volume.clamp(0.0, 100.0);
        let has_audio = clamped_volume > 0.0;

        // Configure element-setup to:
        // 1. Prune unused audio parsing/decoding streams when muted/silent.
        // 2. Intelligently inject vapostproc (Task 5.1) ONLY when a genuine VA-API hardware decoder
        //    is active and capable of acquiring the hardware display context.
        // On NVIDIA (nvdec), software (avdec), or systems without VA-API, video-filter remains unset,
        // avoiding cross-device conflicts, upload/download roundtrips, and pipeline negotiation failures.
        playbin.connect("element-setup", false, move |args| {
            let playbin = args.first().and_then(|v| v.get::<gst::Element>().ok())?;
            let elem = args.get(1).and_then(|v| v.get::<gst::Element>().ok())?;

            let fac = elem.factory();
            let fac_name = fac
                .as_ref()
                .map(|f| f.name().to_string())
                .unwrap_or_default();

            if !has_audio
                && (fac_name == "uridecodebin" || fac_name == "decodebin")
                && let Ok(video_any) = gst::Caps::from_str("video/x-raw(ANY)")
            {
                elem.set_property("caps", &video_any);
                elem.set_property("expose-all-streams", false);
            }

            if is_vaapi_decoder(&elem)
                && playbin
                    .property::<Option<gst::Element>>("video-filter")
                    .is_none()
                && is_vapostproc_usable()
                && let Ok(vapostproc) = gst::ElementFactory::make("vapostproc").build()
            {
                tracing::info!(
                    decoder = %fac_name,
                    "Intelligently injected vapostproc video-filter for VA-API hardware pipeline"
                );
                playbin.set_property("video-filter", &vapostproc);
            }

            None
        });

        if has_audio {
            playbin.set_property_from_str("flags", "video+audio+soft-volume");
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
            // Highly optimized wallpaper mode: video-only decoding,
            // no audio pipeline, no subtitle processing, no software deinterlacing/colorbalance.
            // Buffering flag omitted to prevent redundant queue threads for local files.
            playbin.set_property_from_str("flags", "video");
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
            texture_y: None,
            texture_y_view: None,
            texture_uv: None,
            texture_uv_view: None,
            color_buffer: None,
            cached_color_params: None,
            target_is_srgb: true,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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

        self.target_is_srgb = target_format.is_srgb();

        let color_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video_color_params_buffer"),
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let default_params = default_color_params(self.target_is_srgb);
        let mut bytes = [0u8; 256];
        for (i, f) in default_params.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&f.to_ne_bytes());
        }
        _queue.write_buffer(&color_buffer, 0, &bytes);

        self.pipeline_gpu = Some(pipeline);
        self.bind_group_layout = Some(bind_group_layout);
        self.sampler = Some(sampler);
        self.color_buffer = Some(color_buffer);
        self.cached_color_params = None;

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
                    .field("format", "NV12")
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

        // Retrieve the latest video frame (pull sample or preroll buffer if paused)
        let pull_timeout = if self.texture_y.is_none() {
            // First frame initialization or paused preroll: wait briefly (up to 500ms)
            // for the decoder to produce the first frame so initial presentation
            // and screenshot capture don't render a blank/black frame.
            gst::ClockTime::from_mseconds(500)
        } else {
            gst::ClockTime::ZERO
        };

        let sample_opt = appsink
            .try_pull_sample(pull_timeout)
            .or_else(|| appsink.try_pull_preroll(pull_timeout));

        if let Some(sample) = sample_opt
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

            // Dynamically update color parameters uniform if colorimetry or format changed
            let color_params = color_params_from_video_info(&video_info, self.target_is_srgb);
            if self.cached_color_params != Some(color_params) {
                self.cached_color_params = Some(color_params);
                if let Some(buf) = &self.color_buffer {
                    let mut bytes = [0u8; 256];
                    for (i, f) in color_params.iter().enumerate() {
                        bytes[i * 4..(i + 1) * 4].copy_from_slice(&f.to_ne_bytes());
                    }
                    ctx.queue.write_buffer(buf, 0, &bytes);
                }
            }

            if let Ok(video_frame) =
                gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &video_info)
                && let Ok(data_y) = video_frame.plane_data(0)
                && let Ok(data_uv) = video_frame.plane_data(1)
            {
                let w = video_info.width();
                let h = video_info.height();
                let stride_y = video_frame.plane_stride()[0] as u32;
                let stride_uv = video_frame.plane_stride()[1] as u32;

                if w > 0 && h > 0 {
                    let uv_w = w.div_ceil(2);
                    let uv_h = h.div_ceil(2);

                    // Recreate GPU textures if dimensions changed or first allocation
                    if self.video_width != w || self.video_height != h || self.texture_y.is_none() {
                        self.video_width = w;
                        self.video_height = h;

                        let Some(bgl) = &self.bind_group_layout else {
                            return;
                        };
                        let Some(sampler) = &self.sampler else {
                            return;
                        };
                        let Some(color_buffer) = &self.color_buffer else {
                            return;
                        };

                        let texture_y = ctx.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("video_frame_texture_y"),
                            size: wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::R8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });

                        let texture_uv = ctx.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("video_frame_texture_uv"),
                            size: wgpu::Extent3d {
                                width: uv_w,
                                height: uv_h,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rg8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });

                        let view_y = texture_y.create_view(&wgpu::TextureViewDescriptor::default());
                        let view_uv =
                            texture_uv.create_view(&wgpu::TextureViewDescriptor::default());

                        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("video_bind_group"),
                            layout: bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&view_y),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&view_uv),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: color_buffer.as_entire_binding(),
                                },
                            ],
                        });

                        self.texture_y = Some(texture_y);
                        self.texture_y_view = Some(view_y);
                        self.texture_uv = Some(texture_uv);
                        self.texture_uv_view = Some(view_uv);
                        self.bind_group = Some(bind_group);
                    }

                    // Upload pixel data directly from GStreamer buffer into WGPU textures
                    if let (Some(tex_y), Some(tex_uv)) = (&self.texture_y, &self.texture_uv) {
                        ctx.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: tex_y,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            data_y,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(stride_y),
                                rows_per_image: Some(h),
                            },
                            wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: 1,
                            },
                        );

                        ctx.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: tex_uv,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            data_uv,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(stride_uv),
                                rows_per_image: Some(uv_h),
                            },
                            wgpu::Extent3d {
                                width: uv_w,
                                height: uv_h,
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
                        pipeline.set_property_from_str("flags", "video+audio+soft-volume");
                        if let Ok(asink) = gst::ElementFactory::make("pipewiresink")
                            .build()
                            .or_else(|_| gst::ElementFactory::make("autoaudiosink").build())
                        {
                            pipeline.set_property("audio-sink", &asink);
                        }
                    } else if old_vol > 0.0 && self.volume <= 0.0 {
                        pipeline.set_property_from_str("flags", "video");
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

    fn teardown(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
        self.appsink = None;
        self.bus = None;
        self.texture_y = None;
        self.texture_y_view = None;
        self.texture_uv = None;
        self.texture_uv_view = None;
        self.bind_group = None;
        self.color_buffer = None;
        self.pipeline_gpu = None;
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
    fn test_color_ranges() {
        let _ = gst_video::VideoColorRange::Range0_255;
        let _ = gst_video::VideoColorRange::Range16_235;
        let _ = gst_video::VideoColorMatrix::Bt709;
        let _ = gst_video::VideoColorMatrix::Bt601;
    }

    #[test]
    fn test_video_info_colorimetry() {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "NV12")
            .field("width", 1920i32)
            .field("height", 1080i32)
            .field("framerate", gst::Fraction::new(30, 1))
            .field("colorimetry", "bt709")
            .build();
        let info = gst_video::VideoInfo::from_caps(&caps).unwrap();
        let colorimetry = info.colorimetry();
        assert_eq!(colorimetry.range(), gst_video::VideoColorRange::Range16_235);
        assert_eq!(colorimetry.matrix(), gst_video::VideoColorMatrix::Bt709);
    }

    #[test]
    fn test_color_params_calculation() {
        gst::init().unwrap();
        let default_p = default_color_params(false);
        assert!((default_p[0] - 16.0 / 255.0).abs() < 1e-6);
        assert!((default_p[1] - 255.0 / 219.0).abs() < 1e-6);
        assert!((default_p[2] - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(default_p[7], 0.0);

        let default_p_srgb = default_color_params(true);
        assert_eq!(default_p_srgb[7], 1.0);

        // Test BT.709 limited-range caps
        let caps_709_lim = gst::Caps::builder("video/x-raw")
            .field("format", "NV12")
            .field("width", 1920i32)
            .field("height", 1080i32)
            .field("framerate", gst::Fraction::new(30, 1))
            .field("colorimetry", "bt709")
            .build();
        let info_709_lim = gst_video::VideoInfo::from_caps(&caps_709_lim).unwrap();
        let p_709_lim = color_params_from_video_info(&info_709_lim, false);
        assert_eq!(p_709_lim, default_p);

        let p_709_lim_srgb = color_params_from_video_info(&info_709_lim, true);
        assert_eq!(p_709_lim_srgb[7], 1.0);

        // Verify black level Y=16, U=128, V=128 maps to RGB (0, 0, 0)
        let y_raw = 16.0 / 255.0;
        let u_raw = 128.0 / 255.0;
        let v_raw = 128.0 / 255.0;
        let y = ((y_raw - p_709_lim[0]) * p_709_lim[1]).clamp(0.0, 1.0);
        let u = u_raw - p_709_lim[2];
        let v = v_raw - p_709_lim[2];
        let r = (y + p_709_lim[3] * v).clamp(0.0, 1.0);
        let g = (y - p_709_lim[4] * u - p_709_lim[5] * v).clamp(0.0, 1.0);
        let b = (y + p_709_lim[6] * u).clamp(0.0, 1.0);
        assert!(r.abs() < 1e-5);
        assert!(g.abs() < 1e-5);
        assert!(b.abs() < 1e-5);

        // Verify white level Y=235, U=128, V=128 maps to RGB (1, 1, 1)
        let y_white = 235.0 / 255.0;
        let y_w = ((y_white - p_709_lim[0]) * p_709_lim[1]).clamp(0.0, 1.0);
        let r_w = (y_w + p_709_lim[3] * v).clamp(0.0, 1.0);
        let g_w = (y_w - p_709_lim[4] * u - p_709_lim[5] * v).clamp(0.0, 1.0);
        let b_w = (y_w + p_709_lim[6] * u).clamp(0.0, 1.0);
        assert!((r_w - 1.0).abs() < 1e-5);
        assert!((g_w - 1.0).abs() < 1e-5);
        assert!((b_w - 1.0).abs() < 1e-5);
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

    #[test]
    fn test_is_vaapi_decoder_classification() {
        gst::init().unwrap();

        // Non-decoder elements must never be classified as VA-API decoders
        if let Ok(elem) = gst::ElementFactory::make("videotestsrc").build() {
            assert!(!is_vaapi_decoder(&elem));
        }
        if let Ok(elem) = gst::ElementFactory::make("capsfilter").build() {
            assert!(!is_vaapi_decoder(&elem));
        }
        if let Ok(elem) = gst::ElementFactory::make("vapostproc").build() {
            assert!(!is_vaapi_decoder(&elem));
        }
        if let Ok(elem) = gst::ElementFactory::make("appsink").build() {
            assert!(!is_vaapi_decoder(&elem));
        }

        // Software decoders must NOT be classified as VA-API decoders
        if let Ok(elem) = gst::ElementFactory::make("avdec_h264").build() {
            assert!(!is_vaapi_decoder(&elem));
        }
        if let Ok(elem) = gst::ElementFactory::make("openh264dec").build() {
            assert!(!is_vaapi_decoder(&elem));
        }

        // VA-API decoders (if present in the GStreamer registry) must be classified as true
        for va_name in &["vah264dec", "vaapih264dec", "vahevcdec", "vavp9dec"] {
            if let Ok(elem) = gst::ElementFactory::make(va_name).build() {
                assert!(
                    is_vaapi_decoder(&elem),
                    "Expected {} to be classified as VA-API decoder",
                    va_name
                );
            }
        }
    }

    #[test]
    fn test_vapostproc_usability_check() {
        gst::init().unwrap();
        // Verifies that is_vapostproc_usable runs cleanly without crashing or panicking
        let _ = is_vapostproc_usable();
    }
}
