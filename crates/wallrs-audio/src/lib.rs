use std::fmt;
use std::sync::Arc;

/// Error returned by audio capture and analysis operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    PipeWireUnavailable(String),
    StreamCreationFailed(String),
    AnalysisFailed(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PipeWireUnavailable(msg) => write!(f, "PipeWire unavailable: {msg}"),
            Self::StreamCreationFailed(msg) => write!(f, "Stream creation failed: {msg}"),
            Self::AnalysisFailed(msg) => write!(f, "Audio analysis failed: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Lock-free handle allowing the render thread to read the latest calculated frequency spectrum.
#[derive(Debug, Clone)]
pub struct SpectrumHandle {
    spectrum: Arc<Vec<f32>>,
}

impl SpectrumHandle {
    pub fn new(bands: usize) -> Self {
        Self {
            spectrum: Arc::new(vec![0.0; bands]),
        }
    }

    pub fn latest(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.spectrum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spectrum_handle() {
        let handle = SpectrumHandle::new(32);
        let data = handle.latest();
        assert_eq!(data.len(), 32);
        assert_eq!(data[0], 0.0);
    }
}
