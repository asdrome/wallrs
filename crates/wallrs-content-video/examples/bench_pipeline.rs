use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
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
        .field("format", "RGBA")
        .field("width", gst::IntRange::new(1, 1920i32))
        .field("height", gst::IntRange::new(1, 1080i32))
        .build();

    let appsink = gst_app::AppSink::builder()
        .caps(&video_caps)
        .drop(true)
        .max_buffers(1)
        .sync(true)
        .build();

    let playbin = gst::ElementFactory::make("playbin3")
        .build()
        .or_else(|_| gst::ElementFactory::make("playbin").build())
        .expect("Failed to create playbin element");

    playbin.set_property("uri", uri.as_str());

    // Build hardware-accelerated video sink bin
    let bin = gst::Bin::new();
    let vapostproc = gst::ElementFactory::make("vapostproc")
        .build()
        .expect("Failed to create vapostproc");
    let caps = gst_video::VideoCapsBuilder::for_encoding("video/x-raw")
        .format(gst_video::VideoFormat::Rgba)
        .build();
    let capsfilter = gst::ElementFactory::make("capsfilter")
        .property("caps", &caps)
        .build()
        .expect("Failed to create capsfilter");
    let videoconvert = gst::ElementFactory::make("videoconvert")
        .property("n-threads", 0u32)
        .build()
        .expect("Failed to create videoconvert");

    bin.add_many([
        &vapostproc,
        &capsfilter,
        &videoconvert,
        appsink.upcast_ref(),
    ])
    .expect("Failed to add elements to sink bin");

    gst::Element::link_many([
        &vapostproc,
        &capsfilter,
        &videoconvert,
        appsink.upcast_ref(),
    ])
    .expect("Failed to link elements in sink bin");

    let sink_pad = vapostproc
        .static_pad("sink")
        .expect("vapostproc missing sink pad");
    let ghost_pad = gst::GhostPad::with_target(&sink_pad).expect("Failed to create ghost pad");
    ghost_pad
        .set_active(true)
        .expect("Failed to activate ghost pad");
    bin.add_pad(&ghost_pad)
        .expect("Failed to add ghost pad to bin");

    playbin.set_property("video-sink", &bin);
    playbin.set_property_from_str("flags", "video+buffering");
    if let Ok(fakesink) = gst::ElementFactory::make("fakesink").build() {
        playbin.set_property("audio-sink", &fakesink);
    }

    playbin
        .set_state(gst::State::Playing)
        .expect("Failed to transition playbin to Playing");
    let bus = playbin.bus().expect("Failed to acquire bus");

    let start_wall = Instant::now();
    let start_cpu = get_cpu_time();

    let mut samples = 0;
    let mut loops = 0;
    let mut detected_fps = None;

    // Simulate 10 seconds at 60 Hz event loop rate (600 frames)
    for _ in 0..600 {
        std::thread::sleep(Duration::from_millis(16));

        while let Some(msg) = bus.pop() {
            if let gst::MessageView::Eos(..) = msg.view() {
                let _ = playbin.seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                    gst::ClockTime::ZERO,
                );
                loops += 1;
            }
        }

        if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::ZERO) {
            if detected_fps.is_none()
                && let Some(caps) = sample.caps()
                && let Ok(vi) = gst_video::VideoInfo::from_caps(caps)
            {
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
