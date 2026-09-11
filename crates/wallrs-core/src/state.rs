use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wallrs_proto::PropertyValue;

/// Configuration of a single output saved to persistent state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SavedOutputConfig {
    Wallpaper {
        path: PathBuf,
        muted: bool,
        #[serde(default)]
        properties: HashMap<String, PropertyValue>,
    },
    Color {
        color: [f32; 4],
    },
}

/// Root snapshot of daemon session state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateSnapshot {
    pub version: u32,
    #[serde(default)]
    pub outputs: HashMap<String, SavedOutputConfig>,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            version: 1,
            outputs: HashMap::new(),
        }
    }
}

/// Resolves the default XDG state file path for wallrs.
/// Conforms to XDG Base Directory specification: `$XDG_STATE_HOME/wallrs/state.json`
/// with fallback to `$HOME/.local/state/wallrs/state.json`.
pub fn default_state_path() -> PathBuf {
    if let Ok(val) = std::env::var("XDG_STATE_HOME")
        && !val.trim().is_empty()
    {
        return PathBuf::from(val).join("wallrs").join("state.json");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("wallrs")
            .join("state.json");
    }
    std::env::temp_dir().join("wallrs").join("state.json")
}

/// Loads the state snapshot from the specified path.
/// Returns None if the file does not exist or fails to parse.
pub fn load_state(path: &Path) -> Option<StateSnapshot> {
    if !path.exists() {
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<StateSnapshot>(&content) {
            Ok(snapshot) => {
                tracing::info!(
                    path = %path.display(),
                    outputs = snapshot.outputs.len(),
                    "Loaded session state"
                );
                Some(snapshot)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to parse session state JSON; starting fresh"
                );
                None
            }
        },
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to read session state file"
            );
            None
        }
    }
}

/// Atomically saves the state snapshot to disk using a temporary file and rename.
pub fn save_state(path: &Path, snapshot: &StateSnapshot) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    let temp_path = path.with_file_name(format!(".{file_name}.tmp.{}", std::process::id()));
    std::fs::write(&temp_path, json.as_bytes())?;
    std::fs::rename(&temp_path, path)?;
    tracing::debug!(path = %path.display(), "Session state saved atomically");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_snapshot_roundtrip() {
        let mut snapshot = StateSnapshot::default();
        let mut props = HashMap::new();
        props.insert("speed".into(), PropertyValue::Number(1.5));
        props.insert("volume".into(), PropertyValue::Number(50.0));

        snapshot.outputs.insert(
            "eDP-1".into(),
            SavedOutputConfig::Wallpaper {
                path: PathBuf::from("/home/user/wallpapers/aurora"),
                muted: false,
                properties: props,
            },
        );
        snapshot.outputs.insert(
            "HDMI-A-1".into(),
            SavedOutputConfig::Color {
                color: [0.1, 0.2, 0.3, 1.0],
            },
        );

        let json = serde_json::to_string_pretty(&snapshot).expect("serialize");
        let decoded: StateSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(snapshot, decoded);
    }

    #[test]
    fn test_save_and_load_atomic() {
        let temp_dir =
            std::env::temp_dir().join(format!("wallrs_test_state_{}", std::process::id()));
        let state_path = temp_dir.join("state.json");

        let mut snapshot = StateSnapshot::default();
        snapshot.outputs.insert(
            "DP-1".into(),
            SavedOutputConfig::Color {
                color: [1.0, 0.0, 0.0, 1.0],
            },
        );

        save_state(&state_path, &snapshot).expect("save state");
        assert!(state_path.exists());

        let loaded = load_state(&state_path).expect("load state");
        assert_eq!(snapshot, loaded);

        // Corrupted file test
        std::fs::write(&state_path, b"not valid json").expect("write garbage");
        let corrupted = load_state(&state_path);
        assert!(corrupted.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_default_state_path_not_empty() {
        let path = default_state_path();
        assert!(path.to_string_lossy().ends_with("state.json"));
    }
}
