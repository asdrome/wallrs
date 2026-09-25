use gst::prelude::*;
use gstreamer as gst;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use thiserror::Error;
use wallrs_proto::PropertyValue;

#[derive(Debug, Error)]
pub enum AudioPlayerError {
    #[error("Audio file not found: {0:?}")]
    FileNotFound(PathBuf),

    #[error("Failed to initialize GStreamer: {0}")]
    GstInit(String),

    #[error("Failed to create GStreamer pipeline: {0}")]
    PipelineCreate(String),

    #[error("Invalid property: {0}")]
    InvalidProperty(String),
}

/// Headless background audio player powered by GStreamer.
///
/// Designed to play ambient music or sound effects alongside static images,
/// parallax wallpapers, or shaders with negligible CPU footprint (`video-sink=fakesink`).
pub struct BackgroundAudioPlayer {
    pipeline: gst::Element,
    path: PathBuf,
    volume: f64,
    loop_track: bool,
    is_paused: bool,
    stop_sender: Option<Sender<()>>,
    worker_handle: Option<JoinHandle<()>>,
}

// Safety: BackgroundAudioPlayer encapsulates thread-safe GStreamer elements
// and is accessed on the engine/render event loop thread.
unsafe impl Send for BackgroundAudioPlayer {}

impl BackgroundAudioPlayer {
    /// Creates and immediately starts playback of an audio file in loop.
    pub fn new(path: &Path, volume: f64, loop_track: bool) -> Result<Self, AudioPlayerError> {
        if !path.exists() {
            return Err(AudioPlayerError::FileNotFound(path.to_path_buf()));
        }

        gst::init().map_err(|e| AudioPlayerError::GstInit(e.to_string()))?;

        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let uri = format!("file://{}", canonical.display());

        // Attempt playbin3, falling back to playbin
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin").build())
            .map_err(|e| AudioPlayerError::PipelineCreate(e.to_string()))?;

        playbin.set_property("uri", uri.as_str());

        // Configure headless audio-only sinks
        if let Ok(video_sink) = gst::ElementFactory::make("fakesink").build() {
            playbin.set_property("video-sink", &video_sink);
        }

        // Prefer pipewiresink when available, fallback to autoaudiosink
        let audio_sink = gst::ElementFactory::make("pipewiresink")
            .build()
            .or_else(|_| gst::ElementFactory::make("autoaudiosink").build())
            .ok();

        if let Some(asink) = audio_sink {
            playbin.set_property("audio-sink", &asink);
        }

        let clamped_volume = volume.clamp(0.0, 100.0);
        // GStreamer playbin volume scale is 0.0 to 1.0 (1.0 = 100%)
        playbin.set_property("volume", clamped_volume / 100.0);

        // Always initialize in a strictly muted state.
        // Unmuting is explicitly triggered by daemon policy or CLI commands.
        playbin.set_property("mute", true);

        let _ = playbin.set_state(gst::State::Playing);

        // Setup background bus watcher thread for looping upon EOS
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let bus = playbin.bus();
        let playbin_weak = playbin.downgrade();

        let worker_handle = if let Some(bus) = bus {
            std::thread::Builder::new()
                .name("wallrs-audio-bus".into())
                .spawn(move || {
                    loop {
                        if stop_rx.try_recv().is_ok() {
                            break;
                        }
                        if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(150)) {
                            match msg.view() {
                                gst::MessageView::Eos(..) => {
                                    if loop_track {
                                        if let Some(p) = playbin_weak.upgrade() {
                                            let _ = p.seek_simple(
                                                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                                gst::ClockTime::ZERO,
                                            );
                                        } else {
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                }
                                gst::MessageView::Error(err) => {
                                    tracing::warn!(
                                        error = %err.error(),
                                        debug = ?err.debug(),
                                        "GStreamer audio playback error"
                                    );
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                })
                .ok()
        } else {
            None
        };

        Ok(Self {
            pipeline: playbin,
            path: path.to_path_buf(),
            volume: clamped_volume,
            loop_track,
            is_paused: false,
            stop_sender: Some(stop_tx),
            worker_handle,
        })
    }

    /// Helper constructor resolving relative paths against wallpaper base directory.
    pub fn from_config(
        config: &wallrs_proto::AudioTrackConfig,
        base_dir: &Path,
    ) -> Result<Self, AudioPlayerError> {
        let resolved = if config.path.is_absolute() {
            config.path.clone()
        } else {
            base_dir.join(&config.path)
        };
        let volume = config.volume.unwrap_or(50.0) as f64;
        let loop_track = config.r#loop.unwrap_or(true);
        Self::new(&resolved, volume, loop_track)
    }

    /// Sets playback pause state.
    pub fn set_paused(&mut self, paused: bool) {
        self.is_paused = paused;
        let target_state = if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        let _ = self.pipeline.set_state(target_state);
    }

    /// Returns whether the audio player is currently paused.
    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    /// Sets the output volume (0.0 to 100.0).
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 100.0);
        self.pipeline.set_property("volume", self.volume / 100.0);
    }

    /// Updates runtime properties (volume, mute, speed, pause).
    pub fn set_property(
        &mut self,
        key: &str,
        value: PropertyValue,
    ) -> Result<(), AudioPlayerError> {
        match key {
            "volume" => match value {
                PropertyValue::Number(n) => {
                    self.set_volume(n as f64);
                    Ok(())
                }
                _ => Err(AudioPlayerError::InvalidProperty(
                    "volume must be a numeric value (0..100)".into(),
                )),
            },
            "mute" => match value {
                PropertyValue::Bool(b) => {
                    self.pipeline.set_property("mute", b);
                    Ok(())
                }
                _ => Err(AudioPlayerError::InvalidProperty(
                    "mute must be a boolean".into(),
                )),
            },
            "pause" => match value {
                PropertyValue::Bool(b) => {
                    self.set_paused(b);
                    Ok(())
                }
                _ => Err(AudioPlayerError::InvalidProperty(
                    "pause must be a boolean".into(),
                )),
            },
            "speed" => match value {
                PropertyValue::Number(n) => {
                    let rate = (n as f64).clamp(0.1, 10.0);
                    let _ = self.pipeline.seek(
                        rate,
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::SeekType::None,
                        gst::ClockTime::NONE,
                        gst::SeekType::None,
                        gst::ClockTime::NONE,
                    );
                    Ok(())
                }
                _ => Err(AudioPlayerError::InvalidProperty(
                    "speed must be a numeric value".into(),
                )),
            },
            _ => Ok(()),
        }
    }

    /// Returns the active audio file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns current volume.
    pub fn volume(&self) -> f64 {
        self.volume
    }

    /// Returns whether the track is looping.
    pub fn is_loop(&self) -> bool {
        self.loop_track
    }
}

impl Drop for BackgroundAudioPlayer {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_sender.take() {
            let _ = tx.send(());
        }
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_wav() -> Vec<u8> {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + 1000u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // 1 channel
        wav.extend_from_slice(&44100u32.to_le_bytes()); // 44.1kHz
        wav.extend_from_slice(&(44100u32 * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // 16-bit
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1000u32.to_le_bytes());
        wav.resize(44 + 1000, 0);
        wav
    }

    #[test]
    fn test_background_audio_player_lifecycle() {
        let temp_dir = std::env::temp_dir();
        let dummy_audio = temp_dir.join("wallrs_test_audio.wav");
        std::fs::write(&dummy_audio, generate_test_wav()).unwrap();

        let mut player = BackgroundAudioPlayer::new(&dummy_audio, 30.0, true).unwrap();
        assert_eq!(player.volume(), 30.0);
        assert!(player.is_loop());

        player.set_paused(true);
        assert!(player.is_paused());

        player.set_volume(80.0);
        assert_eq!(player.volume(), 80.0);

        player
            .set_property("volume", PropertyValue::Number(45.0))
            .unwrap();
        assert_eq!(player.volume(), 45.0);

        player
            .set_property("mute", PropertyValue::Bool(true))
            .unwrap();

        let _ = std::fs::remove_file(dummy_audio);
    }

    #[test]
    fn test_background_audio_player_nonexistent_file() {
        let res = BackgroundAudioPlayer::new(Path::new("/nonexistent/path/sound.mp3"), 50.0, true);
        assert!(matches!(res, Err(AudioPlayerError::FileNotFound(_))));
    }
}
