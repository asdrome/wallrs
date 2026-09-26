use crate::xdg;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use wallrs_proto::{Command, Response, default_socket_path};

/// Validates and lints a wallpaper manifest using the default socket if available.
pub fn validate_wallpaper(path: &Path) -> Result<(), String> {
    validate_wallpaper_with_socket(path, None)
}

/// Validates and lints a wallpaper manifest and its associated assets with an optional custom socket.
///
/// Resolves paths directly or via standard XDG wallpaper directories.
pub fn validate_wallpaper_with_socket(path: &Path, socket: Option<&Path>) -> Result<(), String> {
    let manifest_path = xdg::resolve_wallpaper_path(path)?;

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read manifest at {:?}: {}", manifest_path, e))?;

    let manifest = wallrs_proto::WallpaperManifest::from_toml_str(&content)
        .map_err(|e| format!("Syntax error in wallpaper.toml: {}", e))?;

    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    println!("Validating wallpaper: {:?}", manifest_path);
    println!("  • Name: \"{}\"", manifest.wallpaper.name);
    println!("  • Type: \"{}\"", manifest.wallpaper.r#type);
    if let Some(author) = &manifest.wallpaper.author {
        println!("  • Author: \"{}\"", author);
    }
    if let Some(desc) = &manifest.wallpaper.description {
        println!("  • Description: \"{}\"", desc);
    }

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
                        let osc_str = layer.oscillation.as_ref().map_or("none".into(), |o| {
                            format!("speed: {}, amp: {}, axis: {}", o.speed, o.amplitude, o.axis)
                        });
                        let dn_str = match layer.day_night {
                            Some(wallrs_proto::DayNightMode::Tint) => "tint",
                            Some(wallrs_proto::DayNightMode::Night) => "night",
                            Some(wallrs_proto::DayNightMode::Day) => "day",
                            None => "none",
                        };
                        let tint_str = layer.tint.map_or("none".into(), |t| {
                            format!("[{:.2}, {:.2}, {:.2}]", t[0], t[1], t[2])
                        });
                        println!(
                            "    ✓ Layer {i}: {:?} ({}x{}) [parallax: {}, pan: {}, osc: {}, day_night: {}, tint: {}]",
                            layer.path, w, h, parallax_str, pan_str, osc_str, dn_str, tint_str
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

            if shader_code.trim().is_empty() {
                return Err(format!("Shader entry file {:?} is empty", shader_file));
            }

            let is_glsl = shader_file.extension().and_then(|ext| ext.to_str()) == Some("glsl")
                || shader_code.contains("void mainImage");

            // Attempt to query live daemon compiler via IPC socket if available
            let socket_path = socket
                .map(PathBuf::from)
                .unwrap_or_else(default_socket_path);
            let mut validated_by_daemon = false;

            if let Ok(mut stream) = UnixStream::connect(&socket_path) {
                let cmd = Command::ValidateWallpaper {
                    manifest_path: manifest_path.clone(),
                };
                if let Ok(json) = serde_json::to_string(&cmd) {
                    let _ = stream.write_all(json.as_bytes());
                    let _ = stream.write_all(b"\n");
                    let mut reader = BufReader::new(stream);
                    let mut response_line = String::new();
                    if reader.read_line(&mut response_line).is_ok()
                        && let Ok(resp) = serde_json::from_str::<Response>(&response_line)
                    {
                        match resp {
                            Response::Ok => {
                                validated_by_daemon = true;
                                println!(
                                    "    ✓ Daemon GPU pipeline validated shader AST successfully: {:?}",
                                    sh.entry
                                );
                            }
                            Response::Error(e) => {
                                return Err(format!(
                                    "Daemon shader validation error in {:?}: {}",
                                    shader_file, e
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !validated_by_daemon {
                // Structural offline validation without pulling Naga / WGPU into the client CLI
                if is_glsl {
                    if !shader_code.contains("mainImage") && !shader_code.contains("main(") {
                        return Err(format!(
                            "GLSL shader {:?} is missing entry point (expected 'void mainImage' or 'void main')",
                            shader_file
                        ));
                    }
                    println!(
                        "    ✓ Shadertoy GLSL structure verified: {:?} (offline check; run 'wallrsd' for live GPU Naga AST verification)",
                        sh.entry
                    );
                } else {
                    if !shader_code.contains("@fragment") && !shader_code.contains("fs_main") {
                        return Err(format!(
                            "WGSL shader {:?} is missing fragment entry point (expected '@fragment' or 'fs_main')",
                            shader_file
                        ));
                    }
                    let open_braces = shader_code.chars().filter(|&c| c == '{').count();
                    let close_braces = shader_code.chars().filter(|&c| c == '}').count();
                    if open_braces != close_braces {
                        return Err(format!(
                            "WGSL shader {:?} has unbalanced braces: {} open vs {} close",
                            shader_file, open_braces, close_braces
                        ));
                    }
                    println!(
                        "    ✓ Native WGSL structure verified: {:?} (offline check; run 'wallrsd' for live GPU Naga AST verification)",
                        sh.entry
                    );
                }
            }

            if sh.audio == Some(true) {
                println!("    ✓ PipeWire audio reactivity enabled");
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
            let vol = vid.volume.unwrap_or(0.0);
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

    if let Some(thumb) = &manifest.wallpaper.thumbnail {
        let thumb_file = if thumb.is_absolute() {
            thumb.clone()
        } else {
            base_dir.join(thumb)
        };
        if !thumb_file.exists() {
            return Err(format!("Thumbnail file does not exist: {:?}", thumb_file));
        }
        println!("  • Thumbnail: {:?}", thumb);
    }

    println!(
        "✓ Wallpaper '{}' is valid and ready to use!",
        manifest.wallpaper.name
    );
    Ok(())
}
