use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Selector for one or more display outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSelector {
    Named(String),
    Span(Vec<String>),
    All,
}

/// Dynamic property value that can be sent to a wallpaper renderer at runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropertyValue {
    Bool(bool),
    Number(f32),
    Text(String),
    Color([f32; 4]),
}

/// Commands accepted by `wallrsd` over the control Unix domain socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    SetWallpaper {
        output: OutputSelector,
        manifest_path: PathBuf,
    },
    SetProperty {
        output: OutputSelector,
        key: String,
        value: PropertyValue,
    },
    Pause {
        output: Option<String>,
    },
    Resume {
        output: Option<String>,
    },
    TogglePause {
        output: Option<String>,
    },
    Screenshot {
        output: String,
        path: PathBuf,
    },
    ListOutputs,
    Kill,
}

/// Information about a connected display output returned by `wallrsd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInfoProto {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub paused: bool,
}

/// Responses emitted by `wallrsd` over the control Unix domain socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Outputs(Vec<OutputInfoProto>),
    Error(String),
}

/// Error returned when loading or parsing a wallpaper manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Failed to read manifest file at {0:?}: {1}")]
    Io(PathBuf, std::io::Error),

    #[error("Failed to parse wallpaper manifest: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Top-level wallpaper manifest loaded from `wallpaper.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallpaperManifest {
    pub wallpaper: WallpaperMeta,
    #[serde(default)]
    pub image: Option<ImageConfig>,
    #[serde(default)]
    pub shader: Option<ShaderConfig>,
    #[serde(default)]
    pub video: Option<VideoConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallpaperMeta {
    pub r#type: String, // "image", "shader", "video"
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageConfig {
    #[serde(default)]
    pub layers: Vec<ImageLayerConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub parallax: Option<f32>,
    #[serde(default)]
    pub pan: Option<PanConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanConfig {
    pub speed: f32,
    #[serde(default = "default_pan_axis")]
    pub axis: String,
}

fn default_pan_axis() -> String {
    "x".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderConfig {
    pub entry: PathBuf,
    #[serde(default)]
    pub uniforms: std::collections::HashMap<String, f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub volume: Option<f32>,
    #[serde(default)]
    pub r#loop: Option<bool>,
}

impl std::str::FromStr for WallpaperManifest {
    type Err = toml::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

impl WallpaperManifest {
    pub fn from_toml_str(content: &str) -> Result<Self, toml::de::Error> {
        content.parse()
    }

    pub fn from_file(path: &std::path::Path) -> Result<Self, ManifestError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ManifestError::Io(path.to_path_buf(), e))?;
        content.parse().map_err(ManifestError::Parse)
    }
}

/// Returns the standard default path for the wallrs control Unix socket:
/// `$XDG_RUNTIME_DIR/wallrs.sock` with fallback to `/tmp/wallrs-<uid>.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("wallrs.sock")
    } else {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        PathBuf::from(format!("/tmp/wallrs-{user}.sock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_selector_serialization() {
        let all = OutputSelector::All;
        let json = serde_json::to_string(&all).unwrap();
        let parsed: OutputSelector = serde_json::from_str(&json).unwrap();
        assert_eq!(all, parsed);

        let named = OutputSelector::Named("eDP-1".to_string());
        let json = serde_json::to_string(&named).unwrap();
        let parsed: OutputSelector = serde_json::from_str(&json).unwrap();
        assert_eq!(named, parsed);
    }

    #[test]
    fn test_command_serialization_roundtrip() {
        let cmd = Command::SetProperty {
            output: OutputSelector::Named("eDP-1".into()),
            key: "color".into(),
            value: PropertyValue::Color([0.1, 0.2, 0.3, 1.0]),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, parsed);

        let cmd_toggle = Command::TogglePause {
            output: Some("HDMI-A-1".into()),
        };
        let json_toggle = serde_json::to_string(&cmd_toggle).unwrap();
        let parsed_toggle: Command = serde_json::from_str(&json_toggle).unwrap();
        assert_eq!(cmd_toggle, parsed_toggle);

        let cmd_list = Command::ListOutputs;
        let json_list = serde_json::to_string(&cmd_list).unwrap();
        let parsed_list: Command = serde_json::from_str(&json_list).unwrap();
        assert_eq!(cmd_list, parsed_list);
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let resp = Response::Outputs(vec![OutputInfoProto {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            paused: false,
        }]);
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, parsed);

        let resp_ok = Response::Ok;
        let json_ok = serde_json::to_string(&resp_ok).unwrap();
        let parsed_ok: Response = serde_json::from_str(&json_ok).unwrap();
        assert_eq!(resp_ok, parsed_ok);
    }

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        assert!(path.ends_with("wallrs.sock") || path.to_string_lossy().contains("wallrs-"));
    }

    #[test]
    fn test_wallpaper_manifest_parsing() {
        let toml_data = r#"
[wallpaper]
type = "image"
name = "valle"

[[image.layers]]
path = "fondo.png"

[[image.layers]]
path = "nubes.png"
parallax = 0.4
pan = { speed = 0.02, axis = "x" }
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        assert_eq!(manifest.wallpaper.r#type, "image");
        assert_eq!(manifest.wallpaper.name, "valle");
        let image = manifest.image.expect("image config missing");
        assert_eq!(image.layers.len(), 2);
        assert_eq!(image.layers[0].path, PathBuf::from("fondo.png"));
        assert_eq!(image.layers[0].parallax, None);
        assert_eq!(image.layers[0].pan, None);

        assert_eq!(image.layers[1].path, PathBuf::from("nubes.png"));
        assert_eq!(image.layers[1].parallax, Some(0.4));
        let pan = image.layers[1].pan.as_ref().unwrap();
        assert_eq!(pan.speed, 0.02);
        assert_eq!(pan.axis, "x");
    }
}
