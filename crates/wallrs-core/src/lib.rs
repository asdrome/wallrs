pub mod engine;
pub mod ipc;
pub mod output;

pub use engine::{Engine, EngineError, EngineState};
pub use ipc::{IpcError, bind_socket, register_ipc_source};
pub use output::{OutputError, OutputSurface};
pub use wallrs_proto as proto;
pub use wallrs_render as render;

/// Core engine orchestrator configuration.
#[derive(Debug, Default)]
pub struct EngineConfig {
    pub socket_path: Option<std::path::PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_config_default() {
        let config = EngineConfig::default();
        assert!(config.socket_path.is_none());
    }

    #[test]
    fn test_core_smoke() {
        let config = EngineConfig::default();
        assert_eq!(config.socket_path, None);
    }
}
