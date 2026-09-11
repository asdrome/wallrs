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
    ToggleMute {
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
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub wallpaper: Option<PathBuf>,
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
    #[serde(default)]
    pub audio: Option<AudioTrackConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallpaperMeta {
    pub r#type: String, // "image", "shader", "video"
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageConfig {
    #[serde(default)]
    pub layers: Vec<ImageLayerConfig>,
    #[serde(default)]
    pub day_night: Option<DayNightScheduleConfig>,
    #[serde(default)]
    pub fps: Option<u32>,
}

/// Global day/night lighting schedule and custom ambient tint curve for an image wallpaper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayNightScheduleConfig {
    /// Hour when dawn begins (default 5.5 / 05:30)
    pub dawn_start: Option<f32>,
    /// Hour when full daylight is reached (default 8.0 / 08:00)
    pub day_start: Option<f32>,
    /// Hour when dusk begins (default 18.0 / 18:00)
    pub dusk_start: Option<f32>,
    /// Hour when full night is reached (default 21.0 / 21:00)
    pub night_start: Option<f32>,
    /// Custom ambient tint keyframe nodes
    #[serde(default)]
    pub tint_curve: Option<Vec<TintNodeConfig>>,
}

/// Keyframe node for custom ambient tint curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TintNodeConfig {
    pub hour: f32,
    pub tint: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub parallax: Option<f32>,
    #[serde(default)]
    pub pan: Option<PanConfig>,
    #[serde(default)]
    pub oscillation: Option<OscillationConfig>,
    #[serde(default, deserialize_with = "deserialize_day_night")]
    pub day_night: Option<DayNightMode>,
    #[serde(default)]
    pub tint: Option<[f32; 3]>,
}

/// Dynamic day/night behavior for an image layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DayNightMode {
    /// Ambient lighting tint based on time of day (dawn, noon, sunset, night)
    Tint,
    /// Layer only visible at night (fades in at dusk, fades out at dawn)
    Night,
    /// Layer only visible during the day (fades out at dusk, fades in at dawn)
    Day,
}

/// Flexible deserializer for `Option<DayNightMode>` accepting bools or strings.
pub fn deserialize_day_night<'de, D>(deserializer: D) -> Result<Option<DayNightMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct DayNightOptVisitor;

    impl<'de> serde::de::Visitor<'de> for DayNightOptVisitor {
        type Value = Option<DayNightMode>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a boolean or string (\"tint\", \"night\", \"day\")")
        }

        fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v {
                Ok(Some(DayNightMode::Tint))
            } else {
                Ok(None)
            }
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            match v.to_ascii_lowercase().as_str() {
                "tint" => Ok(Some(DayNightMode::Tint)),
                "night" => Ok(Some(DayNightMode::Night)),
                "day" => Ok(Some(DayNightMode::Day)),
                other => Err(serde::de::Error::unknown_variant(
                    other,
                    &["tint", "night", "day"],
                )),
            }
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }
    }

    deserializer.deserialize_any(DayNightOptVisitor)
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
pub struct OscillationConfig {
    pub speed: f32,
    pub amplitude: f32,
    #[serde(default = "default_oscillation_axis")]
    pub axis: String,
    #[serde(default)]
    pub phase: Option<f32>,
}

fn default_oscillation_axis() -> String {
    "y".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderConfig {
    pub entry: PathBuf,
    #[serde(default)]
    pub audio: Option<bool>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrackConfig {
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

        let cmd_mute = Command::ToggleMute {
            output: Some("DP-1".into()),
        };
        let json_mute = serde_json::to_string(&cmd_mute).unwrap();
        let parsed_mute: Command = serde_json::from_str(&json_mute).unwrap();
        assert_eq!(cmd_mute, parsed_mute);

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
            muted: false,
            wallpaper: Some(PathBuf::from("/path/to/wallpaper")),
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
        assert!(manifest.audio.is_none());
    }

    #[test]
    fn test_wallpaper_manifest_with_audio() {
        let toml_data = r#"
[wallpaper]
type = "image"
name = "cyber-cafe"

[[image.layers]]
path = "cafe.png"

[audio]
path = "ambient.ogg"
volume = 35.5
loop = true
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        assert_eq!(manifest.wallpaper.name, "cyber-cafe");
        let audio = manifest.audio.expect("audio config missing");
        assert_eq!(audio.path, PathBuf::from("ambient.ogg"));
        assert_eq!(audio.volume, Some(35.5));
        assert_eq!(audio.r#loop, Some(true));
    }

    #[test]
    fn test_wallpaper_manifest_with_thumbnail() {
        let toml_data = r#"
[wallpaper]
type = "shader"
name = "cyber-grid"
thumbnail = "thumb.png"

[shader]
entry = "main.wgsl"
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        assert_eq!(manifest.wallpaper.name, "cyber-grid");
        assert_eq!(
            manifest.wallpaper.thumbnail,
            Some(PathBuf::from("thumb.png"))
        );
    }

    #[test]
    fn test_wallpaper_manifest_with_author_and_shader_audio() {
        let toml_data = r#"
[wallpaper]
type = "shader"
name = "aurora-live"
author = "Developer"
description = "A dynamic aurora shader"

[shader]
entry = "aurora.wgsl"
audio = true
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        assert_eq!(manifest.wallpaper.name, "aurora-live");
        assert_eq!(manifest.wallpaper.author, Some("Developer".into()));
        assert_eq!(
            manifest.wallpaper.description,
            Some("A dynamic aurora shader".into())
        );
        let shader = manifest.shader.expect("shader config missing");
        assert_eq!(shader.entry, PathBuf::from("aurora.wgsl"));
        assert_eq!(shader.audio, Some(true));
    }

    #[test]
    fn test_wallpaper_manifest_image_with_oscillation() {
        let toml_data = r#"
[wallpaper]
type = "image"
name = "floating-island"

[[image.layers]]
path = "island.png"
parallax = 0.3
oscillation = { speed = 1.5, amplitude = 0.02, axis = "y", phase = 0.5 }
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        let img = manifest.image.expect("image config missing");
        assert_eq!(img.layers.len(), 1);
        let layer = &img.layers[0];
        assert_eq!(layer.parallax, Some(0.3));
        let osc = layer.oscillation.as_ref().expect("oscillation missing");
        assert_eq!(osc.speed, 1.5);
        assert_eq!(osc.amplitude, 0.02);
        assert_eq!(osc.axis, "y");
        assert_eq!(osc.phase, Some(0.5));
    }

    #[test]
    fn test_wallpaper_manifest_image_with_day_night_and_tint() {
        let toml_data = r#"
[wallpaper]
type = "image"
name = "day-night-cycle"

[[image.layers]]
path = "sky.png"
day_night = true
tint = [0.95, 0.90, 1.0]

[[image.layers]]
path = "stars.png"
day_night = "night"

[[image.layers]]
path = "birds.png"
day_night = "day"
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        let img = manifest.image.expect("image config missing");
        assert_eq!(img.layers.len(), 3);

        assert_eq!(img.layers[0].day_night, Some(DayNightMode::Tint));
        assert_eq!(img.layers[0].tint, Some([0.95, 0.90, 1.0]));

        assert_eq!(img.layers[1].day_night, Some(DayNightMode::Night));
        assert_eq!(img.layers[1].tint, None);

        assert_eq!(img.layers[2].day_night, Some(DayNightMode::Day));
    }

    #[test]
    fn test_day_night_schedule_manifest() {
        let toml_data = r#"
[wallpaper]
type = "image"
name = "custom-schedule"

[image.day_night]
dawn_start = 6.0
day_start = 8.5
dusk_start = 17.5
night_start = 20.5

[[image.day_night.tint_curve]]
hour = 2.0
tint = [0.15, 0.22, 0.40]

[[image.day_night.tint_curve]]
hour = 12.0
tint = [1.0, 1.0, 1.0]

[[image.layers]]
path = "bg.png"
day_night = "tint"
"#;

        let manifest = WallpaperManifest::from_toml_str(toml_data).expect("failed to parse TOML");
        let img = manifest.image.expect("image config missing");
        let dn = img.day_night.expect("day_night config missing");
        assert_eq!(dn.dawn_start, Some(6.0));
        assert_eq!(dn.day_start, Some(8.5));
        assert_eq!(dn.dusk_start, Some(17.5));
        assert_eq!(dn.night_start, Some(20.5));

        let curve = dn.tint_curve.expect("tint_curve missing");
        assert_eq!(curve.len(), 2);
        assert_eq!(curve[0].hour, 2.0);
        assert_eq!(curve[0].tint, [0.15, 0.22, 0.40]);
        assert_eq!(curve[1].hour, 12.0);
        assert_eq!(curve[1].tint, [1.0, 1.0, 1.0]);
    }
}
