use clap::{Args, Parser, Subcommand};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use wallrs_proto::{
    Command, OutputInfoProto, OutputSelector, PropertyValue, Response, default_socket_path,
};

#[derive(Parser, Debug)]
#[command(
    name = "wallctl",
    author,
    version,
    about = "Control and query the wallrs live wallpaper daemon",
    long_about = "wallctl communicates with wallrsd over a Unix domain socket to manage active outputs, switch wallpapers, set runtime properties, pause/resume, or terminate the daemon."
)]
struct Cli {
    /// Path to the wallrsd control Unix domain socket
    #[arg(short, long, global = true, value_name = "SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Subcommands,
}

#[derive(Subcommand, Debug)]
enum Subcommands {
    /// List all connected display outputs and their current status
    #[command(alias = "list")]
    ListOutputs(ListOutputsArgs),

    /// Set solid background color on an output or all outputs
    SetColor(SetColorArgs),

    /// Set a dynamic runtime property on an output
    SetProperty(SetPropertyArgs),

    /// Pause wallpaper rendering (saves GPU/CPU when windows are full screen or idle)
    Pause(TargetOutputArgs),

    /// Resume wallpaper rendering
    Resume(TargetOutputArgs),

    /// Toggle wallpaper rendering pause/resume state
    #[command(alias = "toggle")]
    TogglePause(TargetOutputArgs),

    /// Mute wallpaper audio playback
    Mute(TargetOutputArgs),

    /// Unmute wallpaper audio playback
    Unmute(TargetOutputArgs),

    /// Toggle wallpaper audio mute state
    ToggleMute(TargetOutputArgs),

    /// Load and display a wallpaper from a manifest folder or wallpaper.toml
    SetWallpaper(SetWallpaperArgs),

    /// Take a screenshot of the current wallpaper on an output and save it to an image file
    Screenshot(ScreenshotArgs),

    /// Get the representative preview image path for an output (resolves thumbnail or captures live snapshot)
    Preview(PreviewArgs),

    /// Validate and lint a wallpaper folder or wallpaper.toml manifest
    Validate(ValidateArgs),

    /// Gracefully terminate the wallrsd daemon
    Kill,
}

#[derive(Args, Debug)]
struct PreviewArgs {
    /// Target output name (defaults to first active output)
    #[arg(short, long)]
    output: Option<String>,

    /// Force capturing a live GPU snapshot instead of returning static thumbnail/image
    #[arg(long)]
    snapshot: bool,

    /// Output path for snapshot fallback (defaults to $XDG_RUNTIME_DIR/wallrs/preview-<output>.png)
    #[arg(long)]
    out_path: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ValidateArgs {
    /// Path to wallpaper.toml or directory containing wallpaper.toml
    path: PathBuf,
}

#[derive(Args, Debug)]
struct ScreenshotArgs {
    /// Target output name (e.g. "eDP-1")
    output: String,

    /// Destination file path (e.g. "screenshot.png")
    path: PathBuf,
}

#[derive(Args, Debug)]
struct SetWallpaperArgs {
    /// Path to wallpaper.toml or a directory containing wallpaper.toml
    path: PathBuf,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,

    /// Unmute wallpaper audio playback upon loading (audio defaults to muted)
    #[arg(long)]
    unmute: bool,
}

#[derive(Args, Debug)]
struct ListOutputsArgs {
    /// Output the list in JSON format
    #[arg(short, long)]
    json: bool,
}

#[derive(Args, Debug)]
struct SetColorArgs {
    /// Color in hex format (e.g., "#0f172a", "ff5500", "#1e1e2eff")
    color: String,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Args, Debug)]
struct SetPropertyArgs {
    /// Property name/key
    key: String,

    /// Property value (number, bool, hex color, or text)
    value: String,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Args, Debug)]
struct TargetOutputArgs {
    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

/// Parses a hex color string (3, 4, 6, or 8 hex digits, optional leading '#') into RGBA f32 [0.0..1.0].
pub fn parse_hex_color(s: &str) -> Result<[f32; 4], String> {
    let clean = s.trim().trim_start_matches('#');
    let (r, g, b, a) = match clean.len() {
        3 => {
            let r = u8::from_str_radix(&clean[0..1], 16).map_err(|e| e.to_string())? * 17;
            let g = u8::from_str_radix(&clean[1..2], 16).map_err(|e| e.to_string())? * 17;
            let b = u8::from_str_radix(&clean[2..3], 16).map_err(|e| e.to_string())? * 17;
            (r, g, b, 255)
        }
        4 => {
            let r = u8::from_str_radix(&clean[0..1], 16).map_err(|e| e.to_string())? * 17;
            let g = u8::from_str_radix(&clean[1..2], 16).map_err(|e| e.to_string())? * 17;
            let b = u8::from_str_radix(&clean[2..3], 16).map_err(|e| e.to_string())? * 17;
            let a = u8::from_str_radix(&clean[3..4], 16).map_err(|e| e.to_string())? * 17;
            (r, g, b, a)
        }
        6 => {
            let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
            (r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
            let a = u8::from_str_radix(&clean[6..8], 16).map_err(|e| e.to_string())?;
            (r, g, b, a)
        }
        _ => {
            return Err(format!(
                "Invalid hex color '{s}': expected 3, 4, 6, or 8 hex digits"
            ));
        }
    };

    Ok([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

/// Infers the `PropertyValue` type from a string representation.
pub fn parse_property_value(val: &str) -> PropertyValue {
    if val.eq_ignore_ascii_case("true") {
        return PropertyValue::Bool(true);
    }
    if val.eq_ignore_ascii_case("false") {
        return PropertyValue::Bool(false);
    }
    if let Ok(num) = val.parse::<f32>() {
        return PropertyValue::Number(num);
    }
    if val.starts_with('#')
        && let Ok(color) = parse_hex_color(val)
    {
        return PropertyValue::Color(color);
    }
    PropertyValue::Text(val.to_string())
}

fn print_outputs_table(outputs: &[OutputInfoProto]) {
    if outputs.is_empty() {
        println!("No active outputs detected by wallrsd.");
        return;
    }

    println!(
        "{:<15} {:<15} {:<10} {:<10} {:<25}",
        "OUTPUT", "RESOLUTION", "STATUS", "AUDIO", "WALLPAPER"
    );
    println!(
        "{:-<15} {:-<15} {:-<10} {:-<10} {:-<25}",
        "", "", "", "", ""
    );
    for out in outputs {
        let res = format!("{}x{}", out.width, out.height);
        let status = if out.paused { "Paused" } else { "Active" };
        let audio = if out.muted { "Muted" } else { "Unmuted" };
        let wall = out
            .wallpaper
            .as_ref()
            .map(|p| {
                p.file_name()
                    .or_else(|| p.parent().and_then(|parent| parent.file_name()))
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "(solid color)".into());
        println!(
            "{:<15} {:<15} {:<10} {:<10} {:<25}",
            out.name, res, status, audio, wall
        );
    }
}

fn validate_wallpaper(path: &Path) -> Result<(), String> {
    let manifest_path = if path.is_dir() {
        path.join("wallpaper.toml")
    } else {
        path.to_path_buf()
    };

    if !manifest_path.exists() {
        return Err(format!(
            "Wallpaper manifest not found at {:?}",
            manifest_path
        ));
    }

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {:?}: {}", manifest_path, e))?;

    let manifest = wallrs_proto::WallpaperManifest::from_toml_str(&content)
        .map_err(|e| format!("Syntax error in wallpaper.toml: {}", e))?;

    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    println!("Validating wallpaper: {:?}", manifest_path);
    println!("  • Name: \"{}\"", manifest.wallpaper.name);
    println!("  • Type: \"{}\"", manifest.wallpaper.r#type);

    match manifest.wallpaper.r#type.as_str() {
        "image" => {
            let Some(img) = &manifest.image else {
                return Err(
                    "Manifest declares type 'image' but is missing [image] configuration block"
                        .into(),
                );
            };
            if img.layers.is_empty() {
                return Err("Image wallpaper has 0 layers defined in [[image.layers]]".into());
            }
            println!("  • Layers ({}):", img.layers.len());
            for (i, layer) in img.layers.iter().enumerate() {
                let layer_file = if layer.path.is_absolute() {
                    layer.path.clone()
                } else {
                    base_dir.join(&layer.path)
                };
                if !layer_file.exists() {
                    return Err(format!("Layer {i} file does not exist: {:?}", layer_file));
                }
                match image::image_dimensions(&layer_file) {
                    Ok((w, h)) => {
                        let parallax_str =
                            layer.parallax.map_or("none".into(), |p| format!("{p:.2}"));
                        let pan_str = layer.pan.as_ref().map_or("none".into(), |p| {
                            format!("speed: {}, axis: {}", p.speed, p.axis)
                        });
                        println!(
                            "    ✓ Layer {i}: {:?} ({}x{}) [parallax: {}, pan: {}]",
                            layer.path, w, h, parallax_str, pan_str
                        );
                    }
                    Err(e) => {
                        return Err(format!(
                            "Layer {i} file {:?} is not a valid image: {}",
                            layer_file, e
                        ));
                    }
                }
            }
        }
        "shader" => {
            let Some(sh) = &manifest.shader else {
                return Err(
                    "Manifest declares type 'shader' but is missing [shader] configuration block"
                        .into(),
                );
            };
            let shader_file = if sh.entry.is_absolute() {
                sh.entry.clone()
            } else {
                base_dir.join(&sh.entry)
            };
            if !shader_file.exists() {
                return Err(format!(
                    "Shader entry file does not exist: {:?}",
                    shader_file
                ));
            }
            let shader_code = std::fs::read_to_string(&shader_file)
                .map_err(|e| format!("Failed to read shader file {:?}: {}", shader_file, e))?;

            let is_glsl = shader_file.extension().and_then(|ext| ext.to_str()) == Some("glsl")
                || shader_code.contains("void mainImage");

            if is_glsl {
                match wallrs_content_shader::translate_shadertoy_glsl_to_wgsl(&shader_code) {
                    Ok(wgsl) => {
                        if let Err(e) = naga::front::wgsl::parse_str(&wgsl) {
                            return Err(format!(
                                "Translated Shadertoy WGSL failed validation:\n{}",
                                e.emit_to_string(&wgsl)
                            ));
                        }
                        println!(
                            "    ✓ Shadertoy GLSL shader validated successfully: {:?}",
                            sh.entry
                        );
                    }
                    Err(e) => {
                        return Err(format!(
                            "Shadertoy GLSL translation error in {:?}: {}",
                            shader_file, e
                        ));
                    }
                }
            } else {
                let prepared = wallrs_content_shader::prepare_wgsl(&shader_code);
                if let Err(e) = naga::front::wgsl::parse_str(&prepared) {
                    return Err(format!(
                        "WGSL shader syntax error in {:?}:\n{}",
                        shader_file,
                        e.emit_to_string(&prepared)
                    ));
                }
                println!(
                    "    ✓ Native WGSL shader validated successfully: {:?}",
                    sh.entry
                );
            }
        }
        "video" => {
            let Some(vid) = &manifest.video else {
                return Err(
                    "Manifest declares type 'video' but is missing [video] configuration block"
                        .into(),
                );
            };
            let video_file = if vid.path.is_absolute() {
                vid.path.clone()
            } else {
                base_dir.join(&vid.path)
            };
            if !video_file.exists() {
                return Err(format!("Video file does not exist: {:?}", video_file));
            }
            let vol = vid.volume.unwrap_or(50.0);
            let lp = vid.r#loop.unwrap_or(true);
            println!(
                "    ✓ Video file exists: {:?} [volume: {}%, loop: {}]",
                vid.path, vol, lp
            );
        }
        other => {
            return Err(format!(
                "Unknown wallpaper type: '{other}'. Expected 'image', 'shader', or 'video'"
            ));
        }
    }

    if let Some(audio) = &manifest.audio {
        let audio_file = if audio.path.is_absolute() {
            audio.path.clone()
        } else {
            base_dir.join(&audio.path)
        };
        if !audio_file.exists() {
            return Err(format!(
                "Background audio track file does not exist: {:?}",
                audio_file
            ));
        }
        let vol = audio.volume.unwrap_or(50.0);
        let lp = audio.r#loop.unwrap_or(true);
        println!(
            "  • Background audio track: {:?} [volume: {}%, loop: {}]",
            audio.path, vol, lp
        );
    }

    println!(
        "✓ Wallpaper '{}' is valid and ready to use!",
        manifest.wallpaper.name
    );
    Ok(())
}

fn send_command(socket_path: &Path, cmd: &Command) -> Result<Response, String> {
    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        format!(
            "Failed to connect to wallrsd at {:?}: {}\nIs the daemon running? (Try running `wallrsd &`)",
            socket_path, e
        )
    })?;

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(3)));

    let mut payload =
        serde_json::to_string(cmd).map_err(|e| format!("Failed to serialize command: {e}"))?;
    payload.push('\n');

    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("Failed to send command to wallrsd: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush socket: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read response from wallrsd: {e}"))?;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("Daemon closed connection without returning a response".to_string());
    }

    serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse response from wallrsd: {e} (raw: {trimmed})"))
}

fn handle_preview(socket_path: &Path, args: PreviewArgs) -> Result<(), String> {
    let resp = send_command(socket_path, &Command::ListOutputs)?;
    let outputs = match resp {
        Response::Outputs(outs) => outs,
        Response::Error(e) => return Err(format!("Daemon error: {e}")),
        Response::Ok => return Err("Unexpected response from daemon".into()),
    };

    if outputs.is_empty() {
        return Err("No active Wayland display outputs found".into());
    }

    let target = if let Some(name) = &args.output {
        outputs.iter().find(|o| o.name == *name).ok_or_else(|| {
            format!(
                "Output '{name}' not found. Available outputs: {}",
                outputs
                    .iter()
                    .map(|o| o.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?
    } else {
        &outputs[0]
    };

    let fallback_path = || -> PathBuf {
        if let Some(path) = &args.out_path {
            return path.clone();
        }
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            })
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("wallrs");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("preview-{}.png", target.name))
    };

    if args.snapshot {
        let out_img = fallback_path();
        let shot_cmd = Command::Screenshot {
            output: target.name.clone(),
            path: out_img.clone(),
        };
        match send_command(socket_path, &shot_cmd)? {
            Response::Ok => {
                println!("{}", out_img.display());
                return Ok(());
            }
            Response::Error(e) => return Err(format!("Failed to capture screenshot: {e}")),
            _ => return Err("Unexpected response taking screenshot".into()),
        }
    }

    if let Some(wall_path) = &target.wallpaper {
        let is_image_ext = wall_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp"
                )
            });
        if is_image_ext && wall_path.exists() {
            println!(
                "{}",
                wall_path
                    .canonicalize()
                    .unwrap_or_else(|_| wall_path.clone())
                    .display()
            );
            return Ok(());
        }

        let manifest_file = if wall_path.is_dir() {
            wall_path.join("wallpaper.toml")
        } else {
            wall_path.clone()
        };

        if manifest_file.exists() {
            let base_dir = manifest_file.parent().unwrap_or_else(|| Path::new("."));
            if let Ok(manifest) = wallrs_proto::WallpaperManifest::from_file(&manifest_file) {
                if let Some(thumb) = &manifest.wallpaper.thumbnail {
                    let thumb_path = if thumb.is_absolute() {
                        thumb.clone()
                    } else {
                        base_dir.join(thumb)
                    };
                    if thumb_path.exists() {
                        println!(
                            "{}",
                            thumb_path.canonicalize().unwrap_or(thumb_path).display()
                        );
                        return Ok(());
                    }
                }

                if let Some(img_cfg) = &manifest.image
                    && let Some(first) = img_cfg.layers.first()
                {
                    let layer_path = if first.path.is_absolute() {
                        first.path.clone()
                    } else {
                        base_dir.join(&first.path)
                    };
                    if layer_path.exists() {
                        println!(
                            "{}",
                            layer_path.canonicalize().unwrap_or(layer_path).display()
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    let out_img = fallback_path();
    let shot_cmd = Command::Screenshot {
        output: target.name.clone(),
        path: out_img.clone(),
    };
    match send_command(socket_path, &shot_cmd)? {
        Response::Ok => {
            println!("{}", out_img.display());
            Ok(())
        }
        Response::Error(e) => Err(format!("Failed to capture fallback preview snapshot: {e}")),
        _ => Err("Unexpected response taking screenshot".into()),
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    if let Subcommands::Validate(args) = cli.command {
        return validate_wallpaper(&args.path);
    }
    let socket_path = cli.socket.unwrap_or_else(default_socket_path);

    if let Subcommands::Preview(args) = cli.command {
        return handle_preview(&socket_path, args);
    }

    let mut unmute_after = false;
    let mut unmute_selector = OutputSelector::All;

    let (cmd, expect_json) = match cli.command {
        Subcommands::ListOutputs(args) => (Command::ListOutputs, args.json),
        Subcommands::SetColor(args) => {
            let color = parse_hex_color(&args.color)?;
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: "color".into(),
                    value: PropertyValue::Color(color),
                },
                false,
            )
        }
        Subcommands::SetProperty(args) => {
            let value = parse_property_value(&args.value);
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: args.key,
                    value,
                },
                false,
            )
        }
        Subcommands::Pause(args) => (
            Command::Pause {
                output: args.output,
            },
            false,
        ),
        Subcommands::Resume(args) => (
            Command::Resume {
                output: args.output,
            },
            false,
        ),
        Subcommands::TogglePause(args) => (
            Command::TogglePause {
                output: args.output,
            },
            false,
        ),
        Subcommands::Mute(args) => {
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: "mute".into(),
                    value: PropertyValue::Bool(true),
                },
                false,
            )
        }
        Subcommands::Unmute(args) => {
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: "mute".into(),
                    value: PropertyValue::Bool(false),
                },
                false,
            )
        }
        Subcommands::ToggleMute(args) => (
            Command::ToggleMute {
                output: args.output,
            },
            false,
        ),
        Subcommands::SetWallpaper(args) => {
            let manifest_path = if args.path.is_dir() {
                args.path.join("wallpaper.toml")
            } else {
                args.path
            };
            let canonical = manifest_path.canonicalize().map_err(|e| {
                format!("Failed to find wallpaper manifest at {manifest_path:?}: {e}")
            })?;
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            if args.unmute {
                unmute_after = true;
                unmute_selector = selector.clone();
            }
            (
                Command::SetWallpaper {
                    output: selector,
                    manifest_path: canonical,
                },
                false,
            )
        }
        Subcommands::Screenshot(args) => (
            Command::Screenshot {
                output: args.output,
                path: args.path,
            },
            false,
        ),
        Subcommands::Kill => (Command::Kill, false),
        Subcommands::Validate(_) | Subcommands::Preview(_) => unreachable!(),
    };

    let resp = send_command(&socket_path, &cmd)?;

    match resp {
        Response::Ok => {
            if unmute_after {
                let unmute_cmd = Command::SetProperty {
                    output: unmute_selector,
                    key: "mute".into(),
                    value: PropertyValue::Bool(false),
                };
                let resp2 = send_command(&socket_path, &unmute_cmd)?;
                if let Response::Error(err) = resp2 {
                    return Err(format!("Wallpaper loaded, but failed to unmute: {err}"));
                }
            }
            println!("OK");
            Ok(())
        }
        Response::Outputs(outputs) => {
            if expect_json {
                let json = serde_json::to_string_pretty(&outputs)
                    .map_err(|e| format!("Failed to serialize outputs to JSON: {e}"))?;
                println!("{json}");
            } else {
                print_outputs_table(&outputs);
            }
            Ok(())
        }
        Response::Error(err) => Err(format!("Daemon error: {err}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_valid() {
        // #RGB
        let c = parse_hex_color("#fff").unwrap();
        assert_eq!(c, [1.0, 1.0, 1.0, 1.0]);

        // #RGBA
        let c = parse_hex_color("#f008").unwrap();
        assert!((c[0] - 1.0).abs() < 1e-3);
        assert!((c[1] - 0.0).abs() < 1e-3);
        assert!((c[2] - 0.0).abs() < 1e-3);
        assert!((c[3] - 0.533).abs() < 1e-2);

        // #RRGGBB
        let c = parse_hex_color("#00ff00").unwrap();
        assert_eq!(c, [0.0, 1.0, 0.0, 1.0]);

        // without '#'
        let c = parse_hex_color("0000ff").unwrap();
        assert_eq!(c, [0.0, 0.0, 1.0, 1.0]);

        // #RRGGBBAA
        let c = parse_hex_color("#00000080").unwrap();
        assert_eq!(c[0], 0.0);
        assert_eq!(c[1], 0.0);
        assert_eq!(c[2], 0.0);
        assert!((c[3] - (128.0 / 255.0)).abs() < 1e-3);
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert!(parse_hex_color("#12").is_err());
        assert!(parse_hex_color("#12345").is_err());
        assert!(parse_hex_color("#zzzzzz").is_err());
    }

    #[test]
    fn test_parse_property_value() {
        assert_eq!(parse_property_value("true"), PropertyValue::Bool(true));
        assert_eq!(parse_property_value("FALSE"), PropertyValue::Bool(false));
        assert_eq!(parse_property_value("42.5"), PropertyValue::Number(42.5));
        assert_eq!(
            parse_property_value("#ffffff"),
            PropertyValue::Color([1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(
            parse_property_value("hello world"),
            PropertyValue::Text("hello world".into())
        );
    }

    #[test]
    fn test_cli_parse_screenshot() {
        let cli = Cli::try_parse_from(["wallctl", "screenshot", "eDP-1", "test.png"]).unwrap();
        match cli.command {
            Subcommands::Screenshot(args) => {
                assert_eq!(args.output, "eDP-1");
                assert_eq!(args.path, PathBuf::from("test.png"));
            }
            _ => panic!("Expected Subcommands::Screenshot"),
        }
    }

    #[test]
    fn test_cli_parse_toggle_pause() {
        let cli = Cli::try_parse_from(["wallctl", "toggle-pause", "--output", "eDP-1"]).unwrap();
        match cli.command {
            Subcommands::TogglePause(args) => {
                assert_eq!(args.output, Some("eDP-1".into()));
            }
            _ => panic!("Expected Subcommands::TogglePause"),
        }

        let cli_alias = Cli::try_parse_from(["wallctl", "toggle"]).unwrap();
        match cli_alias.command {
            Subcommands::TogglePause(args) => {
                assert_eq!(args.output, None);
            }
            _ => panic!("Expected Subcommands::TogglePause"),
        }
    }

    #[test]
    fn test_cli_parse_validate() {
        let cli = Cli::try_parse_from(["wallctl", "validate", "examples/aurora-shader"]).unwrap();
        match cli.command {
            Subcommands::Validate(args) => {
                assert_eq!(args.path, PathBuf::from("examples/aurora-shader"));
            }
            _ => panic!("Expected Subcommands::Validate"),
        }
    }

    #[test]
    fn test_validate_sample_wallpapers() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

        let aurora = workspace_root.join("examples/aurora-shader");
        if aurora.exists() {
            assert!(validate_wallpaper(&aurora).is_ok());
        }

        let non_existent = workspace_root.join("examples/non_existent_wallpaper_123");
        assert!(validate_wallpaper(&non_existent).is_err());
    }

    #[test]
    fn test_cli_parse_mute_commands() {
        let cli_mute = Cli::try_parse_from(["wallctl", "mute", "--output", "HDMI-A-1"]).unwrap();
        match cli_mute.command {
            Subcommands::Mute(args) => {
                assert_eq!(args.output, Some("HDMI-A-1".into()));
            }
            _ => panic!("Expected Subcommands::Mute"),
        }

        let cli_unmute = Cli::try_parse_from(["wallctl", "unmute"]).unwrap();
        match cli_unmute.command {
            Subcommands::Unmute(args) => {
                assert_eq!(args.output, None);
            }
            _ => panic!("Expected Subcommands::Unmute"),
        }

        let cli_toggle_mute =
            Cli::try_parse_from(["wallctl", "toggle-mute", "-o", "eDP-1"]).unwrap();
        match cli_toggle_mute.command {
            Subcommands::ToggleMute(args) => {
                assert_eq!(args.output, Some("eDP-1".into()));
            }
            _ => panic!("Expected Subcommands::ToggleMute"),
        }
    }

    #[test]
    fn test_cli_parse_set_wallpaper_unmute() {
        let cli = Cli::try_parse_from([
            "wallctl",
            "set-wallpaper",
            "examples/video-sunset",
            "--unmute",
        ])
        .unwrap();
        match cli.command {
            Subcommands::SetWallpaper(args) => {
                assert!(args.unmute);
                assert_eq!(args.output, None);
                assert_eq!(args.path, PathBuf::from("examples/video-sunset"));
            }
            _ => panic!("Expected Subcommands::SetWallpaper"),
        }
    }

    #[test]
    fn test_cli_parse_preview() {
        let cli1 = Cli::try_parse_from(["wallctl", "preview"]).unwrap();
        match cli1.command {
            Subcommands::Preview(args) => {
                assert_eq!(args.output, None);
                assert!(!args.snapshot);
                assert_eq!(args.out_path, None);
            }
            _ => panic!("Expected Subcommands::Preview"),
        }

        let cli2 = Cli::try_parse_from([
            "wallctl",
            "preview",
            "-o",
            "HDMI-A-1",
            "--snapshot",
            "--out-path",
            "/tmp/custom-preview.png",
        ])
        .unwrap();
        match cli2.command {
            Subcommands::Preview(args) => {
                assert_eq!(args.output, Some("HDMI-A-1".into()));
                assert!(args.snapshot);
                assert_eq!(
                    args.out_path,
                    Some(PathBuf::from("/tmp/custom-preview.png"))
                );
            }
            _ => panic!("Expected Subcommands::Preview"),
        }
    }
}
