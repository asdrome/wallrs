use std::path::Path;
use std::sync::Arc;
use wallrs_audio::{BackgroundAudioPlayer, SpectrumHandle};
use wallrs_proto::{AudioTrackConfig, PropertyValue};
use wallrs_render::WallpaperRenderer;

/// Consolidated audio controller for an output surface.
///
/// Encapsulates background audio playback, volume levels, mute state,
/// pause synchronization, and PipeWire spectrum handle attachment.
pub struct AudioController {
    muted: bool,
    volume: f64,
    paused: bool,
    track: Option<BackgroundAudioPlayer>,
    spectrum_handle: Option<SpectrumHandle>,
}

impl Default for AudioController {
    fn default() -> Self {
        Self::new(true)
    }
}

impl AudioController {
    /// Creates a new `AudioController` with the specified mute state.
    pub fn new(muted: bool) -> Self {
        Self {
            muted,
            volume: 50.0,
            paused: false,
            track: None,
            spectrum_handle: None,
        }
    }

    /// Returns whether audio is currently muted.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Returns the current volume level (0.0 to 100.0).
    pub fn volume(&self) -> f64 {
        self.volume
    }

    /// Returns whether audio playback is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Returns whether a background audio track is active.
    pub fn has_track(&self) -> bool {
        self.track.is_some()
    }

    /// Returns a reference to the active background audio player, if present.
    pub fn track(&self) -> Option<&BackgroundAudioPlayer> {
        self.track.as_ref()
    }

    /// Returns a mutable reference to the active background audio player, if present.
    pub fn track_mut(&mut self) -> Option<&mut BackgroundAudioPlayer> {
        self.track.as_mut()
    }

    /// Returns whether this controller has an active PipeWire spectrum handle attached.
    pub fn has_spectrum(&self) -> bool {
        self.spectrum_handle.is_some()
    }

    /// Returns a reference to the PipeWire spectrum handle, if attached.
    pub fn spectrum_handle(&self) -> Option<&SpectrumHandle> {
        self.spectrum_handle.as_ref()
    }

    /// Returns the latest frequency spectrum snapshot, if a spectrum handle is attached.
    pub fn spectrum(&self) -> Option<Arc<Vec<f32>>> {
        self.spectrum_handle.as_ref().map(|h| h.latest())
    }

    /// Sets the muted state for this controller, propagating it to both the renderer
    /// and the background audio track without duplicated dispatch.
    pub fn set_muted(&mut self, muted: bool, renderer: Option<&mut (dyn WallpaperRenderer + '_)>) {
        self.muted = muted;
        if let Some(r) = renderer {
            let _ = r.set_property("mute", PropertyValue::Bool(muted));
        }
        if let Some(player) = &mut self.track {
            let _ = player.set_property("mute", PropertyValue::Bool(muted));
        }
    }

    /// Toggles the muted state and returns the new muted value.
    pub fn toggle_mute(&mut self, renderer: Option<&mut (dyn WallpaperRenderer + '_)>) -> bool {
        let new_state = !self.muted;
        self.set_muted(new_state, renderer);
        new_state
    }

    /// Sets the playback volume (clamped to 0.0..=100.0) and propagates it to
    /// both the renderer and the background audio track.
    pub fn set_volume(&mut self, volume: f64, renderer: Option<&mut (dyn WallpaperRenderer + '_)>) {
        self.volume = volume.clamp(0.0, 100.0);
        if let Some(r) = renderer {
            let _ = r.set_property("volume", PropertyValue::Number(self.volume as f32));
        }
        if let Some(player) = &mut self.track {
            player.set_volume(self.volume);
        }
    }

    /// Sets the pause state for audio playback, propagating it to both the renderer
    /// and the background audio track.
    pub fn set_paused(
        &mut self,
        paused: bool,
        renderer: Option<&mut (dyn WallpaperRenderer + '_)>,
    ) {
        self.paused = paused;
        if let Some(r) = renderer {
            let _ = r.set_property("pause", PropertyValue::Bool(paused));
        }
        if let Some(player) = &mut self.track {
            player.set_paused(paused);
        }
    }

    /// Handles standard audio-related properties ("mute", "volume", "pause", "speed").
    ///
    /// Returns `true` if the property was handled and consumed as an audio property.
    pub fn set_property(
        &mut self,
        key: &str,
        value: &PropertyValue,
        renderer: Option<&mut (dyn WallpaperRenderer + '_)>,
    ) -> bool {
        match key {
            "mute" => {
                if let PropertyValue::Bool(b) = value {
                    self.set_muted(*b, renderer);
                    return true;
                }
            }
            "volume" => {
                if let PropertyValue::Number(n) = value {
                    self.set_volume(*n as f64, renderer);
                    return true;
                }
            }
            "pause" => {
                if let PropertyValue::Bool(b) = value {
                    self.set_paused(*b, renderer);
                    return true;
                }
            }
            "speed" => {
                if let Some(player) = &mut self.track {
                    let _ = player.set_property(key, value.clone());
                }
            }
            _ => {}
        }
        false
    }

    /// Loads a background audio track from an optional configuration.
    ///
    /// Synchronizes the track's muted state according to `allow_audio` and the controller's current state,
    /// sets pause state if currently paused, and propagates the initial mute state to the renderer.
    pub fn load_background_track(
        &mut self,
        config: Option<&AudioTrackConfig>,
        base_dir: &Path,
        allow_audio: bool,
        renderer: Option<&mut (dyn WallpaperRenderer + '_)>,
    ) {
        if let Some(audio_cfg) = config {
            match BackgroundAudioPlayer::from_config(audio_cfg, base_dir) {
                Ok(mut player) => {
                    if self.paused {
                        player.set_paused(true);
                    }
                    if let Some(vol) = audio_cfg.volume {
                        self.volume = (vol as f64).clamp(0.0, 100.0);
                    }
                    player.set_volume(self.volume);

                    let initial_muted = !allow_audio;
                    self.track = Some(player);
                    self.set_muted(initial_muted, renderer);
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "Failed to load background audio track");
                    self.track = None;
                }
            }
        } else {
            self.track = None;
        }
    }

    /// Synchronizes audio state for a renderer that produces its own audio (e.g. video).
    pub fn sync_renderer_audio(
        &mut self,
        allow_audio: bool,
        renderer: Option<&mut (dyn WallpaperRenderer + '_)>,
    ) {
        let initial_muted = !allow_audio;
        self.set_muted(initial_muted, renderer);
    }

    /// Attaches an audio spectrum handle (e.g. from PipeWire capture).
    pub fn attach_spectrum(&mut self, handle: Option<SpectrumHandle>) {
        self.spectrum_handle = handle;
    }

    /// Detaches the current spectrum handle.
    pub fn detach_spectrum(&mut self) {
        self.spectrum_handle = None;
    }

    /// Clears active background audio track and detaches spectrum handle.
    pub fn clear(&mut self) {
        self.track = None;
        self.spectrum_handle = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wallrs_render::{FrameContext, RendererError};

    struct MockRenderer {
        properties: HashMap<String, PropertyValue>,
    }

    impl MockRenderer {
        fn new() -> Self {
            Self {
                properties: HashMap::new(),
            }
        }
    }

    impl WallpaperRenderer for MockRenderer {
        fn init(
            &mut self,
            _device: &wgpu::Device,
            _queue: &wgpu::Queue,
            _target_format: wgpu::TextureFormat,
        ) -> Result<(), RendererError> {
            Ok(())
        }
        fn resize(&mut self, _width: u32, _height: u32) {}
        fn update(&mut self, _ctx: &FrameContext) {}
        fn render(&mut self, _encoder: &mut wgpu::CommandEncoder, _view: &wgpu::TextureView) {}
        fn set_property(&mut self, key: &str, value: PropertyValue) -> Result<(), RendererError> {
            self.properties.insert(key.to_string(), value);
            Ok(())
        }
    }

    #[test]
    fn test_audio_controller_initial_state() {
        let controller = AudioController::new(true);
        assert!(controller.is_muted());
        assert_eq!(controller.volume(), 50.0);
        assert!(!controller.is_paused());
        assert!(!controller.has_track());
        assert!(!controller.has_spectrum());
        assert!(controller.spectrum().is_none());
    }

    #[test]
    fn test_audio_controller_mute_toggle() {
        let mut controller = AudioController::new(true);
        let mut mock = MockRenderer::new();

        let new_state = controller.toggle_mute(Some(&mut mock));
        assert!(!new_state);
        assert!(!controller.is_muted());
        assert_eq!(
            mock.properties.get("mute"),
            Some(&PropertyValue::Bool(false))
        );

        let new_state = controller.toggle_mute(Some(&mut mock));
        assert!(new_state);
        assert!(controller.is_muted());
        assert_eq!(
            mock.properties.get("mute"),
            Some(&PropertyValue::Bool(true))
        );
    }

    #[test]
    fn test_audio_controller_volume_clamping() {
        let mut controller = AudioController::new(false);
        let mut mock = MockRenderer::new();

        controller.set_volume(150.0, Some(&mut mock));
        assert_eq!(controller.volume(), 100.0);
        assert_eq!(
            mock.properties.get("volume"),
            Some(&PropertyValue::Number(100.0))
        );

        controller.set_volume(-20.0, Some(&mut mock));
        assert_eq!(controller.volume(), 0.0);
        assert_eq!(
            mock.properties.get("volume"),
            Some(&PropertyValue::Number(0.0))
        );
    }

    #[test]
    fn test_audio_controller_pause_dispatch() {
        let mut controller = AudioController::new(false);
        let mut mock = MockRenderer::new();

        controller.set_paused(true, Some(&mut mock));
        assert!(controller.is_paused());
        assert_eq!(
            mock.properties.get("pause"),
            Some(&PropertyValue::Bool(true))
        );

        controller.set_paused(false, Some(&mut mock));
        assert!(!controller.is_paused());
        assert_eq!(
            mock.properties.get("pause"),
            Some(&PropertyValue::Bool(false))
        );
    }

    #[test]
    fn test_audio_controller_set_property_dispatch() {
        let mut controller = AudioController::new(false);
        let mut mock = MockRenderer::new();

        let handled = controller.set_property("mute", &PropertyValue::Bool(true), Some(&mut mock));
        assert!(handled);
        assert!(controller.is_muted());
        assert_eq!(
            mock.properties.get("mute"),
            Some(&PropertyValue::Bool(true))
        );

        let handled =
            controller.set_property("volume", &PropertyValue::Number(75.0), Some(&mut mock));
        assert!(handled);
        assert_eq!(controller.volume(), 75.0);
        assert_eq!(
            mock.properties.get("volume"),
            Some(&PropertyValue::Number(75.0))
        );

        let handled =
            controller.set_property("custom_prop", &PropertyValue::Number(1.0), Some(&mut mock));
        assert!(!handled);
    }

    #[test]
    fn test_audio_controller_spectrum_attachment() {
        let mut controller = AudioController::new(true);
        assert!(!controller.has_spectrum());

        let handle = SpectrumHandle::new(32);
        handle.update(vec![0.5; 32]);
        controller.attach_spectrum(Some(handle));

        assert!(controller.has_spectrum());
        let spec = controller
            .spectrum()
            .expect("spectrum snapshot should exist");
        assert_eq!(spec.len(), 32);
        assert_eq!(spec[0], 0.5);

        controller.detach_spectrum();
        assert!(!controller.has_spectrum());
        assert!(controller.spectrum().is_none());
    }

    #[test]
    fn test_audio_controller_clear() {
        let mut controller = AudioController::new(false);
        let handle = SpectrumHandle::new(16);
        controller.attach_spectrum(Some(handle));
        assert!(controller.has_spectrum());

        controller.clear();
        assert!(!controller.has_spectrum());
        assert!(!controller.has_track());
    }
}
