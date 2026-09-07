pub mod engine;
pub mod ipc;
pub mod output;
pub mod state;

pub use engine::{Engine, EngineError, EngineState};
pub use ipc::{IpcError, bind_socket, register_ipc_source};
pub use output::{OutputError, OutputSurface};
pub use state::{SavedOutputConfig, StateSnapshot, default_state_path, load_state, save_state};
pub use wallrs_proto as proto;
pub use wallrs_render as render;

/// Core engine orchestrator configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub socket_path: Option<std::path::PathBuf>,
    pub max_fps: Option<u32>,
    pub fullscreen_pause: bool,
    pub pause_on_maximized: bool,
    pub allow_audio: bool,
    pub restore_state: bool,
    pub state_path: Option<std::path::PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            socket_path: None,
            max_fps: None,
            fullscreen_pause: true,
            pause_on_maximized: true,
            allow_audio: false,
            restore_state: true,
            state_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_default() {
        let config = EngineConfig::default();
        assert!(config.socket_path.is_none());
        assert!(config.max_fps.is_none());
        assert!(config.fullscreen_pause);
        assert!(config.pause_on_maximized);
        assert!(config.restore_state);
        assert!(config.state_path.is_none());
    }

    #[test]
    fn test_core_smoke() {
        let config = EngineConfig::default();
        assert_eq!(config.socket_path, None);
    }

    #[test]
    fn test_offscreen_screenshot_save() {
        let temp_dir = std::env::temp_dir();
        let shot_path = temp_dir.join(format!("wallrs_shot_{}.png", std::process::id()));

        let width = 64u32;
        let height = 64u32;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&[255, 0, 128, 255]); // RGBA
        }

        image::save_buffer(
            &shot_path,
            &pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("save test screenshot");

        assert!(shot_path.exists());
        let meta = std::fs::metadata(&shot_path).expect("metadata");
        assert!(meta.len() > 0);

        // Verify PNG magic header
        let bytes = std::fs::read(&shot_path).expect("read file");
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");

        let _ = std::fs::remove_file(&shot_path);
    }
}
