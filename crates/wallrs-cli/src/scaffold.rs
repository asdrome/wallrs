use crate::validate::validate_wallpaper;
use crate::xdg;
use clap::Args;
use std::path::{Path, PathBuf};

// Embedded fallbacks sourced directly from repository examples
const EMBEDDED_AURORA_TOML: &str = include_str!("../../../examples/aurora-shader/wallpaper.toml");
const EMBEDDED_AURORA_WGSL: &str = include_str!("../../../examples/aurora-shader/aurora.wgsl");

const EMBEDDED_PLASMA_TOML: &str =
    include_str!("../../../examples/shadertoy-plasma/wallpaper.toml");
const EMBEDDED_PLASMA_GLSL: &str = include_str!("../../../examples/shadertoy-plasma/plasma.glsl");

const EMBEDDED_VISUALIZER_TOML: &str =
    include_str!("../../../examples/audio-visualizer/wallpaper.toml");
const EMBEDDED_VISUALIZER_WGSL: &str =
    include_str!("../../../examples/audio-visualizer/visualizer.wgsl");

const EMBEDDED_LANDSCAPE_TOML: &str =
    include_str!("../../../examples/parallax-landscape/wallpaper.toml");
const EMBEDDED_LANDSCAPE_BG: &[u8] = include_bytes!("../../../examples/parallax-landscape/bg.png");
const EMBEDDED_LANDSCAPE_CLOUDS: &[u8] =
    include_bytes!("../../../examples/parallax-landscape/clouds.png");
const EMBEDDED_LANDSCAPE_FG: &[u8] = include_bytes!("../../../examples/parallax-landscape/fg.png");

const EMBEDDED_VIDEO_TOML: &str = include_str!("../../../examples/video-wallpaper/wallpaper.toml");
const EMBEDDED_VIDEO_SAMPLE: &[u8] = include_bytes!("../../../examples/video-wallpaper/sample.mp4");

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

fn apply_embedded_fallback(template_name: &str, target_dir: &Path) -> Result<(), String> {
    match template_name {
        "shadertoy-plasma" => {
            std::fs::write(target_dir.join("wallpaper.toml"), EMBEDDED_PLASMA_TOML)
                .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
            std::fs::write(target_dir.join("plasma.glsl"), EMBEDDED_PLASMA_GLSL)
                .map_err(|e| format!("Failed to write plasma.glsl: {e}"))?;
        }
        "audio-visualizer" => {
            std::fs::write(target_dir.join("wallpaper.toml"), EMBEDDED_VISUALIZER_TOML)
                .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
            std::fs::write(target_dir.join("visualizer.wgsl"), EMBEDDED_VISUALIZER_WGSL)
                .map_err(|e| format!("Failed to write visualizer.wgsl: {e}"))?;
        }
        "parallax-landscape" => {
            std::fs::write(target_dir.join("wallpaper.toml"), EMBEDDED_LANDSCAPE_TOML)
                .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
            std::fs::write(target_dir.join("bg.png"), EMBEDDED_LANDSCAPE_BG)
                .map_err(|e| format!("Failed to write bg.png: {e}"))?;
            std::fs::write(target_dir.join("clouds.png"), EMBEDDED_LANDSCAPE_CLOUDS)
                .map_err(|e| format!("Failed to write clouds.png: {e}"))?;
            std::fs::write(target_dir.join("fg.png"), EMBEDDED_LANDSCAPE_FG)
                .map_err(|e| format!("Failed to write fg.png: {e}"))?;
        }
        "video-wallpaper" => {
            std::fs::write(target_dir.join("wallpaper.toml"), EMBEDDED_VIDEO_TOML)
                .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
            std::fs::write(target_dir.join("sample.mp4"), EMBEDDED_VIDEO_SAMPLE)
                .map_err(|e| format!("Failed to write sample.mp4: {e}"))?;
        }
        _ => {
            // Default to aurora-shader
            std::fs::write(target_dir.join("wallpaper.toml"), EMBEDDED_AURORA_TOML)
                .map_err(|e| format!("Failed to write wallpaper.toml: {e}"))?;
            std::fs::write(target_dir.join("aurora.wgsl"), EMBEDDED_AURORA_WGSL)
                .map_err(|e| format!("Failed to write aurora.wgsl: {e}"))?;
        }
    }
    Ok(())
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

    let template_name = determine_template_name(&args, &wallpaper_type);

    // 1. Copy template files from disk or embedded fallback
    if let Some(src_dir) = find_template_on_disk(&template_name) {
        copy_dir_recursive(&src_dir, &target_dir)
            .map_err(|e| format!("Failed to copy template from {:?}: {}", src_dir, e))?;
    } else {
        apply_embedded_fallback(&template_name, &target_dir)?;
    }

    // 2. Custom asset overrides
    let mut custom_shader_entry: Option<String> = None;
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

    let mut custom_layers: Vec<String> = Vec::new();
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

    let mut custom_video_path: Option<String> = None;
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
    let mut toml_str = format!(
        "[wallpaper]\ntype = \"{}\"\nname = \"{}\"\nauthor = \"{}\"\ndescription = \"{}\"\n",
        wallpaper_type, wallpaper_name, author, description
    );

    match wallpaper_type.as_str() {
        "shader" => {
            let entry = if let Some(e) = custom_shader_entry {
                e
            } else if template_name == "shadertoy-plasma" {
                "plasma.glsl".into()
            } else if template_name == "audio-visualizer" {
                "visualizer.wgsl".into()
            } else {
                "aurora.wgsl".into()
            };
            let audio_val = if args.audio || template_name == "audio-visualizer" {
                "true"
            } else {
                "false"
            };
            toml_str.push_str(&format!(
                "\n[shader]\nentry = \"{}\"\naudio = {}\n",
                entry, audio_val
            ));
        }
        "image" => {
            toml_str.push('\n');
            if !custom_layers.is_empty() {
                for (i, layer_file) in custom_layers.iter().enumerate() {
                    toml_str.push_str(&format!("[[image.layers]]\npath = \"{}\"\n", layer_file));
                    if i == custom_layers.len() - 1
                        && let Some(p) = args.parallax
                    {
                        toml_str.push_str(&format!("parallax = {:.2}\n", p));
                    }
                    toml_str.push('\n');
                }
            } else {
                toml_str.push_str(
                    "[[image.layers]]\npath = \"bg.png\"\n\n[[image.layers]]\npath = \"fg.png\"\nparallax = 0.40\npan = { speed = 0.005, axis = \"x\" }\n",
                );
            }
        }
        "video" => {
            let vid = custom_video_path.unwrap_or_else(|| "sample.mp4".into());
            let vol = args.volume.unwrap_or(50.0);
            let lp = !args.no_loop;
            toml_str.push_str(&format!(
                "\n[video]\npath = \"{}\"\nvolume = {:.1}\nloop = {}\n",
                vid, vol, lp
            ));
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
    fn test_scaffold_shader_with_embedded_fallback() {
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
    fn test_scaffold_image_with_embedded_fallback() {
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
}
