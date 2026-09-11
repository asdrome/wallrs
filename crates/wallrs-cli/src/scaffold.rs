use crate::validate::validate_wallpaper;
use crate::xdg;
use clap::Args;
use std::path::{Path, PathBuf};

// Universal lightweight procedural fallback template (< 1 KB)
const UNIVERSAL_FALLBACK_TOML: &str = r#"[wallpaper]
type = "shader"
name = "universal-shader"
author = "wallrs"
description = "Minimal procedural shader wallpaper"

[shader]
entry = "aurora.wgsl"
audio = false
"#;

const UNIVERSAL_FALLBACK_WGSL: &str = r#"struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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
};

@group(0) @binding(0)
var<uniform> u_params: ShaderUniforms;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
    );
    var out: VertexOutput;
    let p = pos[in_vertex_index];
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, -p.y * 0.5 + 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = u_params.time * 0.4;
    let wave1 = sin(uv.x * 3.5 + t) * 0.12;
    let wave2 = cos(uv.y * 2.8 - t * 0.7) * 0.12;
    let col_dark = vec3<f32>(0.04, 0.06, 0.16);
    let col_teal = vec3<f32>(0.12, 0.60, 0.52);
    let col_violet = vec3<f32>(0.55, 0.22, 0.68);
    let f = clamp(uv.y + wave1 + wave2, 0.0, 1.0);
    let rgb = mix(col_dark, mix(col_teal, col_violet, uv.x), f);
    return vec4<f32>(rgb, 1.0);
}
"#;

fn apply_embedded_fallback(target_dir: &Path) -> Result<(), String> {
    std::fs::write(target_dir.join("wallpaper.toml"), UNIVERSAL_FALLBACK_TOML)
        .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
    std::fs::write(target_dir.join("aurora.wgsl"), UNIVERSAL_FALLBACK_WGSL)
        .map_err(|e| format!("Failed to write aurora.wgsl: {e}"))?;
    Ok(())
}

#[derive(Args, Debug, Clone)]
pub struct NewWallpaperArgs {
    /// Name or destination path for the new wallpaper directory
    pub name: String,

    /// Wallpaper backend type: "shader", "image", or "video"
    #[arg(short = 't', long = "type")]
    pub r#type: Option<String>,

    /// Specific template name to use as base (e.g. aurora-shader, parallax-landscape, video-wallpaper)
    #[arg(long = "template")]
    pub template: Option<String>,

    /// Parent destination directory
    #[arg(short = 'd', long = "dir")]
    pub dir: Option<PathBuf>,

    /// Store in the current working directory instead of standard XDG wallpapers directory
    #[arg(long = "local")]
    pub local: bool,

    /// Author name (defaults to git user or current username)
    #[arg(long)]
    pub author: Option<String>,

    /// Short description of the wallpaper
    #[arg(long)]
    pub description: Option<String>,

    /// Overwrite destination directory if it already exists
    #[arg(short = 'f', long = "force")]
    pub force: bool,

    // --- Shader options ---
    /// Path to an existing shader file (.wgsl or .glsl) to copy into the wallpaper
    #[arg(long)]
    pub shader: Option<PathBuf>,

    /// Enable PipeWire audio reactivity for shaders
    #[arg(long)]
    pub audio: bool,

    /// Generate a Shadertoy-compatible GLSL template instead of WGSL
    #[arg(long)]
    pub glsl: bool,

    // --- Image options ---
    /// Image layer file paths to copy into the wallpaper (can be passed multiple times)
    #[arg(short = 'l', long = "layer")]
    pub layers: Vec<PathBuf>,

    /// Parallax movement factor for the top image layer (e.g. 0.35)
    #[arg(long)]
    pub parallax: Option<f32>,

    // --- Video options ---
    /// Path to a video file (.mp4, .webm, etc.) to copy into the wallpaper
    #[arg(long)]
    pub video: Option<PathBuf>,

    /// Initial video playback volume (0.0 to 100.0)
    #[arg(long)]
    pub volume: Option<f32>,

    /// Disable video looping
    #[arg(long)]
    pub no_loop: bool,

    // --- Ambient audio options ---
    /// Optional ambient audio file (.ogg, .mp3, .flac, .wav) to copy into the wallpaper
    #[arg(long)]
    pub audio_track: Option<PathBuf>,

    /// Initial volume for the ambient audio track (0.0 to 100.0)
    #[arg(long)]
    pub audio_volume: Option<f32>,
}

fn detect_author() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()?;

    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    std::env::var("USER").ok().filter(|s| !s.is_empty())
}

fn resolve_destination(name_arg: &str, dir_arg: Option<&Path>, local: bool) -> (PathBuf, String) {
    let raw = PathBuf::from(name_arg);
    let wallpaper_name = raw
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-wallpaper")
        .to_string();

    let target_dir =
        if name_arg.contains('/') || name_arg.starts_with('.') || name_arg.starts_with('~') {
            raw
        } else if let Some(parent) = dir_arg {
            parent.join(&wallpaper_name)
        } else if local {
            Path::new(".").join(&wallpaper_name)
        } else {
            xdg::default_user_wallpapers_dir().join(&wallpaper_name)
        };

    (target_dir, wallpaper_name)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

fn find_template_on_disk(template_name: &str) -> Option<PathBuf> {
    for base_dir in xdg::template_search_dirs() {
        let candidate = base_dir.join(template_name);
        if candidate.join("wallpaper.toml").exists() {
            return Some(candidate);
        }
    }
    None
}

fn determine_template_name(args: &NewWallpaperArgs, wallpaper_type: &str) -> String {
    if let Some(ref t) = args.template {
        return t.clone();
    }

    match wallpaper_type {
        "shader" => {
            if args.glsl {
                "shadertoy-plasma".to_string()
            } else if args.audio {
                "audio-visualizer".to_string()
            } else {
                "aurora-shader".to_string()
            }
        }
        "image" => "parallax-landscape".to_string(),
        "video" => "video-wallpaper".to_string(),
        _ => "aurora-shader".to_string(),
    }
}

pub fn handle_new_wallpaper(args: NewWallpaperArgs) -> Result<(), String> {
    let (target_dir, wallpaper_name) =
        resolve_destination(&args.name, args.dir.as_deref(), args.local);

    if target_dir.exists() {
        if !args.force {
            return Err(format!(
                "Destination directory {:?} already exists. Use --force to overwrite.",
                target_dir
            ));
        }
    } else {
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("Failed to create directory {:?}: {}", target_dir, e))?;
    }

    let mut wallpaper_type = args.r#type.as_deref().map(|s| s.to_lowercase());
    if wallpaper_type.is_none() {
        if args.shader.is_some() || args.glsl || args.audio {
            wallpaper_type = Some("shader".into());
        } else if args.video.is_some() {
            wallpaper_type = Some("video".into());
        } else if !args.layers.is_empty() {
            wallpaper_type = Some("image".into());
        } else {
            wallpaper_type = Some("shader".into());
        }
    }

    let wallpaper_type = wallpaper_type.unwrap();
    if !["shader", "image", "video"].contains(&wallpaper_type.as_str()) {
        return Err(format!(
            "Invalid wallpaper type '{}'. Must be one of: 'shader', 'image', 'video'",
            wallpaper_type
        ));
    }

    let author = args
        .author
        .clone()
        .or_else(detect_author)
        .unwrap_or_else(|| "Anonymous".into());
    let description = args
        .description
        .clone()
        .unwrap_or_else(|| match wallpaper_type.as_str() {
            "shader" => "Procedural shader wallpaper".into(),
            "image" => "Multi-layer image wallpaper".into(),
            "video" => "Looping video wallpaper".into(),
            _ => "Live wallpaper for wallrs".into(),
        });

    let has_custom_assets =
        !args.layers.is_empty() || args.video.is_some() || args.shader.is_some();

    let mut custom_shader_entry: Option<String> = None;
    let mut custom_layers: Vec<String> = Vec::new();
    let mut custom_video_path: Option<String> = None;

    if has_custom_assets {
        // User supplied their own assets: copy ONLY user assets, never template files!
        if let Some(ref shader_src) = args.shader {
            if !shader_src.exists() {
                return Err(format!("Specified shader file not found: {:?}", shader_src));
            }
            let file_name = shader_src
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("Invalid shader filename: {:?}", shader_src))?;
            std::fs::copy(shader_src, target_dir.join(file_name))
                .map_err(|e| format!("Failed to copy shader file: {e}"))?;
            custom_shader_entry = Some(file_name.to_string());
        }

        if !args.layers.is_empty() {
            for layer_path in &args.layers {
                if !layer_path.exists() {
                    return Err(format!("Specified layer file not found: {:?}", layer_path));
                }
                let file_name = layer_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| format!("Invalid layer filename: {:?}", layer_path))?;
                std::fs::copy(layer_path, target_dir.join(file_name))
                    .map_err(|e| format!("Failed to copy layer file: {e}"))?;
                custom_layers.push(file_name.to_string());
            }
        }

        if let Some(ref video_src) = args.video {
            if !video_src.exists() {
                return Err(format!("Specified video file not found: {:?}", video_src));
            }
            let file_name = video_src
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("Invalid video filename: {:?}", video_src))?;
            std::fs::copy(video_src, target_dir.join(file_name))
                .map_err(|e| format!("Failed to copy video file: {e}"))?;
            custom_video_path = Some(file_name.to_string());
        }
    } else {
        // No custom assets provided: copy full template from disk if available, or write universal fallback
        let template_name = determine_template_name(&args, &wallpaper_type);
        if let Some(src_dir) = find_template_on_disk(&template_name) {
            copy_dir_recursive(&src_dir, &target_dir)
                .map_err(|e| format!("Failed to copy template from {:?}: {}", src_dir, e))?;
        } else {
            apply_embedded_fallback(&target_dir)?;
        }
    }

    let mut custom_audio_path: Option<String> = None;
    if let Some(ref audio_src) = args.audio_track {
        if !audio_src.exists() {
            return Err(format!("Specified audio track not found: {:?}", audio_src));
        }
        let file_name = audio_src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("Invalid audio filename: {:?}", audio_src))?;
        std::fs::copy(audio_src, target_dir.join(file_name))
            .map_err(|e| format!("Failed to copy audio file: {e}"))?;
        custom_audio_path = Some(file_name.to_string());
    }

    // 3. Rewrite wallpaper.toml with sanitized metadata and configuration
    let manifest_path = target_dir.join("wallpaper.toml");
    let existing_template_content = if !has_custom_assets && manifest_path.exists() {
        std::fs::read_to_string(&manifest_path).ok()
    } else {
        None
    };

    let mut toml_str = format!(
        "[wallpaper]\ntype = \"{}\"\nname = \"{}\"\nauthor = \"{}\"\ndescription = \"{}\"\n",
        wallpaper_type, wallpaper_name, author, description
    );

    match wallpaper_type.as_str() {
        "shader" => {
            if let Some(ref content) = existing_template_content
                && let Some(idx) = content.find("[shader]")
            {
                toml_str.push('\n');
                toml_str.push_str(&content[idx..]);
            } else {
                let entry = if let Some(e) = custom_shader_entry {
                    e
                } else if target_dir.join("visualizer.wgsl").exists() {
                    "visualizer.wgsl".into()
                } else if target_dir.join("plasma.glsl").exists() {
                    "plasma.glsl".into()
                } else {
                    "aurora.wgsl".into()
                };
                let audio_val = if args.audio { "true" } else { "false" };
                toml_str.push_str(&format!(
                    "\n[shader]\nentry = \"{}\"\naudio = {}\n",
                    entry, audio_val
                ));
            }
        }
        "image" => {
            if let Some(ref content) = existing_template_content
                && (content.contains("[image]") || content.contains("[[image.layers]]"))
            {
                let idx = content
                    .find("[image]")
                    .or_else(|| content.find("[[image.layers]]"))
                    .unwrap();
                toml_str.push('\n');
                toml_str.push_str(&content[idx..]);
            } else {
                toml_str.push('\n');
                for (i, layer_file) in custom_layers.iter().enumerate() {
                    toml_str.push_str(&format!("[[image.layers]]\npath = \"{}\"\n", layer_file));
                    if i == custom_layers.len() - 1
                        && let Some(p) = args.parallax
                    {
                        toml_str.push_str(&format!("parallax = {:.2}\n", p));
                    }
                    toml_str.push('\n');
                }
            }
        }
        "video" => {
            if let Some(ref content) = existing_template_content
                && let Some(idx) = content.find("[video]")
            {
                toml_str.push('\n');
                toml_str.push_str(&content[idx..]);
            } else {
                let vid = custom_video_path.unwrap_or_else(|| "sample.mp4".into());
                let vol = args.volume.unwrap_or(0.0);
                let lp = !args.no_loop;
                toml_str.push_str(&format!(
                    "\n[video]\npath = \"{}\"\nvolume = {:.1}\nloop = {}\n",
                    vid, vol, lp
                ));
            }
        }
        _ => unreachable!(),
    }

    if let Some(audio_file) = custom_audio_path {
        let vol = args.audio_volume.unwrap_or(40.0);
        toml_str.push_str(&format!(
            "\n[audio]\npath = \"{}\"\nvolume = {:.1}\nloop = true\n",
            audio_file, vol
        ));
    }

    std::fs::write(&manifest_path, &toml_str)
        .map_err(|e| format!("Failed to write manifest at {:?}: {}", manifest_path, e))?;

    println!(
        "Created new {} wallpaper at {:?}",
        wallpaper_type, target_dir
    );

    // 4. Validate output
    println!("\nValidating generated wallpaper...");
    if let Err(e) = validate_wallpaper(&target_dir) {
        eprintln!("Validation warning: {e}");
    }

    println!("\nUsage:");
    println!("  wallctl validate {:?}", target_dir);
    println!("  wallctl set-wallpaper {:?}", target_dir);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scaffold_shader_with_template() {
        let temp_dir = std::env::temp_dir().join(format!(
            "wallrs_test_scaffold_shader_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let args = NewWallpaperArgs {
            name: "test-shader-wall".into(),
            r#type: Some("shader".into()),
            template: None,
            dir: Some(temp_dir.clone()),
            local: false,
            author: Some("Tester".into()),
            description: Some("Test shader wallpaper".into()),
            force: true,
            shader: None,
            audio: true,
            glsl: false,
            layers: Vec::new(),
            parallax: None,
            video: None,
            volume: None,
            no_loop: false,
            audio_track: None,
            audio_volume: None,
        };

        handle_new_wallpaper(args).expect("Failed to scaffold shader wallpaper");

        let wall_dir = temp_dir.join("test-shader-wall");
        assert!(wall_dir.join("wallpaper.toml").exists());
        assert!(wall_dir.join("visualizer.wgsl").exists());

        validate_wallpaper(&wall_dir).expect("Validation should pass");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_scaffold_default_aurora_shader() {
        let temp_dir = std::env::temp_dir().join(format!(
            "wallrs_test_scaffold_aurora_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let args = NewWallpaperArgs {
            name: "test-aurora-wall".into(),
            r#type: Some("shader".into()),
            template: None,
            dir: Some(temp_dir.clone()),
            local: false,
            author: Some("Tester".into()),
            description: Some("Test aurora shader".into()),
            force: true,
            shader: None,
            audio: false,
            glsl: false,
            layers: Vec::new(),
            parallax: None,
            video: None,
            volume: None,
            no_loop: false,
            audio_track: None,
            audio_volume: None,
        };

        handle_new_wallpaper(args).expect("Failed to scaffold default aurora shader");

        let wall_dir = temp_dir.join("test-aurora-wall");
        assert!(wall_dir.join("wallpaper.toml").exists());
        assert!(wall_dir.join("aurora.wgsl").exists());

        validate_wallpaper(&wall_dir).expect("Validation should pass");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_scaffold_image_from_disk_template() {
        let temp_dir =
            std::env::temp_dir().join(format!("wallrs_test_scaffold_img_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let args = NewWallpaperArgs {
            name: "test-image-wall".into(),
            r#type: Some("image".into()),
            template: None,
            dir: Some(temp_dir.clone()),
            local: false,
            author: Some("Tester".into()),
            description: Some("Test image wallpaper".into()),
            force: true,
            shader: None,
            audio: false,
            glsl: false,
            layers: Vec::new(),
            parallax: None,
            video: None,
            volume: None,
            no_loop: false,
            audio_track: None,
            audio_volume: None,
        };

        handle_new_wallpaper(args).expect("Failed to scaffold image wallpaper");

        let wall_dir = temp_dir.join("test-image-wall");
        assert!(wall_dir.join("wallpaper.toml").exists());
        assert!(wall_dir.join("bg.png").exists());
        assert!(wall_dir.join("fg.png").exists());

        validate_wallpaper(&wall_dir).expect("Validation should pass");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_scaffold_custom_image_layers_no_dead_weight() {
        let temp_dir = std::env::temp_dir().join(format!(
            "wallrs_test_scaffold_custom_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);

        let input_dir = temp_dir.join("inputs");
        std::fs::create_dir_all(&input_dir).unwrap();
        let custom_img = input_dir.join("my_layer.png");
        let png_bytes = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&custom_img, png_bytes).unwrap();

        let args = NewWallpaperArgs {
            name: "test-custom-wall".into(),
            r#type: Some("image".into()),
            template: None,
            dir: Some(temp_dir.clone()),
            local: false,
            author: Some("Tester".into()),
            description: Some("Test custom image wallpaper".into()),
            force: true,
            shader: None,
            audio: false,
            glsl: false,
            layers: vec![custom_img],
            parallax: Some(0.35),
            video: None,
            volume: None,
            no_loop: false,
            audio_track: None,
            audio_volume: None,
        };

        handle_new_wallpaper(args).expect("Failed to scaffold custom image wallpaper");

        let wall_dir = temp_dir.join("test-custom-wall");
        assert!(wall_dir.join("wallpaper.toml").exists());
        assert!(wall_dir.join("my_layer.png").exists());

        // CRITICAL CHECK: Verify that template defaults (bg, fg, sun, moon, stars) were NOT copied!
        assert!(!wall_dir.join("bg.png").exists());
        assert!(!wall_dir.join("fg.png").exists());
        assert!(!wall_dir.join("sun.png").exists());
        assert!(!wall_dir.join("moon.png").exists());
        assert!(!wall_dir.join("stars.png").exists());

        validate_wallpaper(&wall_dir).expect("Validation should pass");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_universal_fallback_standalone() {
        let temp_dir = std::env::temp_dir().join(format!(
            "wallrs_test_scaffold_fallback_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        apply_embedded_fallback(&temp_dir).expect("Universal fallback write should succeed");
        assert!(temp_dir.join("wallpaper.toml").exists());
        assert!(temp_dir.join("aurora.wgsl").exists());

        validate_wallpaper(&temp_dir).expect("Universal fallback should pass validation");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
