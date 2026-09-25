use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// Reads total user + system CPU time in seconds for the current process from `/proc/self/stat`.
fn get_cpu_time() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let parts: Vec<&str> = stat.split_whitespace().collect();
    if parts.len() > 15 {
        let utime: f64 = parts[13].parse().unwrap_or(0.0);
        let stime: f64 = parts[14].parse().unwrap_or(0.0);
        let ticks_per_sec = 100.0; // Standard Linux USER_HZ
        (utime + stime) / ticks_per_sec
    } else {
        0.0
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = if args.len() > 1 {
        args[1].clone()
    } else {
        "examples/video-wallpaper/sample.mp4".into()
    };

    println!("============================================================");
    println!("  wallrs Video Pipeline Diagnostic Benchmark");
    println!("  Target: {path}");
    println!("============================================================");

    gst::init().expect("Failed to initialize GStreamer");

    let full_path = std::path::PathBuf::from(&path);
    let canonical = std::fs::canonicalize(&full_path).unwrap_or(full_path);
    let uri = format!("file://{}", canonical.display());

    let video_caps = gst::Caps::builder("video/x-raw")
        .field("format", "NV12")
        .field("width", gst::IntRange::new(1, 1920i32))
        .field("height", gst::IntRange::new(1, 1080i32))
        .build();

    let appsink = gst_app::AppSink::builder()
        .caps(&video_caps)
        .drop(true)
        .max_buffers(1)
        .sync(true)
        .build();

    let playbin = gst::ElementFactory::make("playbin")
        .build()
        .or_else(|_| gst::ElementFactory::make("playbin3").build())
        .expect("Failed to create playbin element");

    playbin.set_property("uri", uri.as_str());

    playbin.set_property("video-sink", &appsink);
    if let Ok(filter) = gst::ElementFactory::make("vapostproc").build() {
        playbin.set_property("video-filter", &filter);
    }
    playbin.set_property_from_str("flags", "video");
    if let Ok(fakesink) = gst::ElementFactory::make("fakesink").build() {
        playbin.set_property("audio-sink", &fakesink);
    }

    playbin.connect("element-setup", false, |args| {
        if let Some(elem) = args.get(1).and_then(|v| v.get::<gst::Element>().ok()) {
            let fac_name = elem
                .factory()
                .map(|f| f.name().to_string())
                .unwrap_or_default();
            if (fac_name == "uridecodebin" || fac_name == "decodebin")
                && let Ok(video_any) = gst::Caps::from_str("video/x-raw(ANY)")
            {
                elem.set_property("caps", &video_any);
                elem.set_property("expose-all-streams", false);
            }
        }
        None
    });

    let bus = playbin.bus().expect("Failed to acquire bus");
    if let Err(e) = playbin.set_state(gst::State::Playing) {
        eprintln!("set_state(Playing) returned error: {e:?}");
        while let Some(msg) = bus.pop() {
            if let gst::MessageView::Error(err) = msg.view() {
                eprintln!("Bus Error: {} (debug: {:?})", err.error(), err.debug());
            }
        }
        panic!("Failed to transition playbin to Playing: {e:?}");
    }

    let start_wall = Instant::now();
    let start_cpu = get_cpu_time();

    let mut samples = 0;
    let mut loops = 0;
    let mut detected_fps = None;

    // Simulate 10 seconds at 60 Hz event loop rate (600 frames)
    for _ in 0..600 {
        std::thread::sleep(Duration::from_millis(16));

        while let Some(msg) = bus.pop() {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    let _ = playbin.seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                        gst::ClockTime::ZERO,
                    );
                    loops += 1;
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Pipeline Bus Error: {} (debug: {:?})",
                        err.error(),
                        err.debug()
                    );
                }
                gst::MessageView::Warning(warn) => {
                    eprintln!(
                        "Pipeline Bus Warning: {} (debug: {:?})",
                        warn.error(),
                        warn.debug()
                    );
                }
                _ => {}
            }
        }

        if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::ZERO) {
            if detected_fps.is_none()
                && let Some(caps) = sample.caps()
                && let Ok(vi) = gst_video::VideoInfo::from_caps(caps)
            {
                println!("Negotiated caps: {caps}");
                println!(
                    "Negotiated sample resolution: {}x{}",
                    vi.width(),
                    vi.height()
                );
                let frac = vi.fps();
                if frac.numer() > 0 && frac.denom() > 0 {
                    detected_fps = Some(frac.numer() as f64 / frac.denom() as f64);
                }
            }
            samples += 1;
            if let (Some(buf), Some(caps)) = (sample.buffer(), sample.caps())
                && let Ok(vi) = gst_video::VideoInfo::from_caps(caps)
                && let Ok(vf) = gst_video::VideoFrameRef::from_buffer_ref_readable(buf, &vi)
                && let Ok(data) = vf.plane_data(0)
            {
                std::hint::black_box(data[0]);
            }
        }
    }

    let elapsed_wall = start_wall.elapsed().as_secs_f64();
    let elapsed_cpu = get_cpu_time() - start_cpu;
    let cpu_core_percent = (elapsed_cpu / elapsed_wall) * 100.0;
    // Assuming standard 8-thread topology for btop comparison:
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8) as f64;
    let btop_system_percent = cpu_core_percent / num_cpus;

    println!("\nBenchmark Results (5s continuous playback):");
    println!("  * Video FPS detected    : {:?}", detected_fps);
    println!("  * Video samples pulled  : {samples}");
    println!("  * Video loops completed : {loops}");
    println!("  * Wall-clock duration   : {elapsed_wall:.2}s");
    println!("  * Process CPU time spent: {elapsed_cpu:.2}s");
    println!("  * CPU Usage (htop scale, 1 core = 100%): {cpu_core_percent:.1}%");
    println!("  * CPU Usage (btop scale, total CPU = 100%): {btop_system_percent:.1}%");
    println!("============================================================");

    // Clean up pipeline gracefully to avoid GStreamer disposal warnings
    let _ = playbin.set_state(gst::State::Null);
}
