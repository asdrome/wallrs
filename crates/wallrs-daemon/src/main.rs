use clap::Parser;
use wallrs_core::{Engine, EngineError};
use wallrs_render::SolidColorRenderer;

#[derive(Parser, Debug)]
#[command(
    name = "wallrsd",
    author,
    version,
    about = "High-performance live wallpaper daemon for Wayland (wgpu/Vulkan)"
)]
struct Args {
    /// Solid background color in hex format (e.g. '#0f172a', '#1e293b') or comma-separated RGBA (0.0-1.0)
    #[arg(short, long, default_value = "#0f172a")]
    color: String,

    /// Maximum frames-per-second render ceiling (default: unlimited, synced to display refresh rate)
    #[arg(long)]
    fps: Option<u32>,

    /// Disable automatic pausing of wallpaper rendering when a window is in fullscreen
    #[arg(long)]
    no_fullscreen_pause: bool,
}

fn parse_color(s: &str) -> Result<[f32; 4], String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#').or(Some(s)) {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            return Ok([r, g, b, 1.0]);
        } else if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            let a = u8::from_str_radix(&hex[6..8], 16).map_err(|e| e.to_string())? as f32 / 255.0;
            return Ok([r, g, b, a]);
        }
    }

    // Try comma-separated floats
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() == 3 || parts.len() == 4 {
        let r: f32 = parts[0]
            .parse()
            .map_err(|e: std::num::ParseFloatError| e.to_string())?;
        let g: f32 = parts[1]
            .parse()
            .map_err(|e: std::num::ParseFloatError| e.to_string())?;
        let b: f32 = parts[2]
            .parse()
            .map_err(|e: std::num::ParseFloatError| e.to_string())?;
        let a: f32 = if parts.len() == 4 {
            parts[3]
                .parse()
                .map_err(|e: std::num::ParseFloatError| e.to_string())?
        } else {
            1.0
        };
        return Ok([r, g, b, a]);
    }

    Err(format!(
        "Invalid color format: '{s}'. Expected #RRGGBB or #RRGGBBAA"
    ))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let initial_color = match parse_color(&args.color) {
        Ok(c) => c,
        Err(err) => {
            tracing::error!("{err}");
            std::process::exit(1);
        }
    };

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        color = ?initial_color,
        fps = ?args.fps,
        fullscreen_pause = !args.no_fullscreen_pause,
        "Initializing wallrsd live wallpaper daemon"
    );

    let config = wallrs_core::EngineConfig {
        socket_path: None,
        max_fps: args.fps,
        fullscreen_pause: !args.no_fullscreen_pause,
    };

    let mut engine = match Engine::with_config(
        move || Box::new(SolidColorRenderer::new(initial_color)),
        config,
    ) {
        Ok(engine) => engine,
        Err(EngineError::LayerShellNotSupported) => {
            tracing::error!(
                "wlr-layer-shell protocol is not supported by the current Wayland compositor."
            );
            tracing::error!(
                "wallrs requires a compositor with wlr-layer-shell support (such as Sway, Hyprland, river, labwc, etc.)."
            );
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!("Failed to initialize wallrs engine: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = engine.run() {
        tracing::error!("Engine runtime error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_smoke() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn test_parse_color() {
        let c6 = parse_color("#ff0000").unwrap();
        assert!((c6[0] - 1.0).abs() < 1e-4);
        assert!((c6[1] - 0.0).abs() < 1e-4);
        assert!((c6[2] - 0.0).abs() < 1e-4);
        assert!((c6[3] - 1.0).abs() < 1e-4);

        let c8 = parse_color("#00ff0080").unwrap();
        assert!((c8[1] - 1.0).abs() < 1e-4);
        assert!((c8[3] - 0.5019).abs() < 1e-3);
    }

    #[test]
    fn test_daemon_args_parse() {
        let args = Args::try_parse_from([
            "wallrsd",
            "--color",
            "#123456",
            "--fps",
            "60",
            "--no-fullscreen-pause",
        ])
        .unwrap();

        assert_eq!(args.color, "#123456");
        assert_eq!(args.fps, Some(60));
        assert!(args.no_fullscreen_pause);
    }
}
