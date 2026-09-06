use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::PropertyValue;

#[derive(Debug, Error)]
pub enum AudioPlayerError {
    #[error("Audio file not found: {0:?}")]
    FileNotFound(PathBuf),

    #[error("Failed to initialize MPV audio player: {0}")]
    MpvInit(String),

    #[error("Invalid property: {0}")]
    InvalidProperty(String),
}

/// Headless background audio player powered by libmpv.
///
/// Designed to play ambient music or sound effects alongside static images,
/// parallax wallpapers, or shaders with negligible CPU footprint (`vo=null`, `video=no`).
pub struct BackgroundAudioPlayer {
    mpv: Option<libmpv2::Mpv>,
    path: PathBuf,
    volume: f64,
    loop_track: bool,
}

// Safety: BackgroundAudioPlayer owns the mpv instance and is accessed exclusively
// on the engine/render event loop thread.
unsafe impl Send for BackgroundAudioPlayer {}

impl BackgroundAudioPlayer {
    /// Creates and immediately starts playback of an audio file in loop.
    pub fn new(path: &Path, volume: f64, loop_track: bool) -> Result<Self, AudioPlayerError> {
        if !path.exists() {
            return Err(AudioPlayerError::FileNotFound(path.to_path_buf()));
        }

        let mpv = libmpv2::Mpv::new().map_err(|e| AudioPlayerError::MpvInit(format!("{e:?}")))?;

        // Configure mpv for headless audio playback
        let _ = mpv.set_property("vo", "null");
        let _ = mpv.set_property("video", "no");
        let _ = mpv.set_property("audio-display", "no");
        let _ = mpv.set_property("keep-open", "yes");
        let _ = mpv.set_property("idle", "yes");
        let _ = mpv.set_property("volume", volume.clamp(0.0, 100.0));

        if loop_track {
            let _ = mpv.set_property("loop-file", "inf");
        } else {
            let _ = mpv.set_property("loop-file", "no");
        }

        let canonical_str = path.to_string_lossy();
        if let Err(e) = mpv.command("loadfile", &[&canonical_str, "replace"]) {
            tracing::warn!(path = ?path, error = ?e, "MPV loadfile command returned warning/error");
        }

        Ok(Self {
            mpv: Some(mpv),
            path: path.to_path_buf(),
            volume,
            loop_track,
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
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property("pause", paused);
        }
    }

    /// Returns whether the audio player is currently paused.
    pub fn is_paused(&self) -> bool {
        self.mpv
            .as_ref()
            .and_then(|mpv| mpv.get_property::<bool>("pause").ok())
            .unwrap_or(false)
    }

    /// Sets the output volume (0.0 to 100.0).
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 100.0);
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property("volume", self.volume);
        }
    }

    /// Updates runtime properties (volume, mute, speed, pause).
    pub fn set_property(
        &mut self,
        key: &str,
        value: PropertyValue,
    ) -> Result<(), AudioPlayerError> {
        let Some(mpv) = &self.mpv else {
            return Ok(());
        };

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
                    let _ = mpv.set_property("mute", b);
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
                    let _ = mpv.set_property("speed", n as f64);
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
        if let Some(mpv) = &self.mpv {
            let _ = mpv.command("stop", &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_audio_player_lifecycle() {
        let temp_dir = std::env::temp_dir();
        let dummy_audio = temp_dir.join("wallrs_test_audio.wav");
        std::fs::write(&dummy_audio, b"RIFF....WAVEfmt ....data....").unwrap();

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
