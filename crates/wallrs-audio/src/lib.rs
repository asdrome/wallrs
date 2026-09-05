use arc_swap::ArcSwap;
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::pod::Pod;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use thiserror::Error;

/// Error returned by audio capture and analysis operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioError {
    #[error("PipeWire unavailable: {0}")]
    PipeWireUnavailable(String),

    #[error("Stream creation failed: {0}")]
    StreamCreationFailed(String),

    #[error("Audio analysis failed: {0}")]
    AnalysisFailed(String),

    #[error("Audio capture timed out: {0}")]
    Timeout(String),
}

/// Lock-free handle allowing the render thread to read the latest calculated frequency spectrum.
#[derive(Debug, Clone)]
pub struct SpectrumHandle {
    spectrum: Arc<ArcSwap<Vec<f32>>>,
}

impl SpectrumHandle {
    pub fn new(bands: usize) -> Self {
        Self {
            spectrum: Arc::new(ArcSwap::from_pointee(vec![0.0; bands])),
        }
    }

    pub fn update(&self, new_spectrum: Vec<f32>) {
        self.spectrum.store(Arc::new(new_spectrum));
    }

    pub fn latest(&self) -> Arc<Vec<f32>> {
        self.spectrum.load_full()
    }
}

/// Summary audio reactivity metrics extracted from frequency bands.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioMetrics {
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub volume: f32,
}

impl AudioMetrics {
    /// Extracts bass, mid, treble, and volume metrics from a spectrum slice.
    pub fn from_spectrum(spectrum: &[f32]) -> Self {
        if spectrum.is_empty() {
            return Self::default();
        }
        let b_end = 4.min(spectrum.len());
        let m_end = 16.min(spectrum.len());
        let t_end = 32.min(spectrum.len());

        let bass_slice = &spectrum[0..b_end];
        let mid_slice = if b_end < m_end {
            &spectrum[b_end..m_end]
        } else {
            &[]
        };
        let treble_slice = if m_end < t_end {
            &spectrum[m_end..t_end]
        } else {
            &[]
        };

        let avg = |slice: &[f32]| {
            if slice.is_empty() {
                0.0
            } else {
                slice.iter().copied().sum::<f32>() / slice.len() as f32
            }
        };

        Self {
            bass: avg(bass_slice),
            mid: avg(mid_slice),
            treble: avg(treble_slice),
            volume: avg(&spectrum[0..t_end]),
        }
    }
}

/// Fast Fourier Transform and logarithmic frequency bucketing analyzer.
pub struct FftAnalyzer {
    pub fft_size: usize,
    pub sample_rate: f32,
    pub num_bands: usize,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    band_ranges: Vec<(usize, usize)>,
    smoothed_bands: Vec<f32>,
    complex_buffer: Vec<Complex<f32>>,
    scratch_buffer: Vec<Complex<f32>>,
}

impl FftAnalyzer {
    pub fn new(fft_size: usize, sample_rate: f32, num_bands: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let scratch_len = fft.get_inplace_scratch_len();

        // Precompute Hann window: w[n] = 0.5 * (1 - cos(2*pi*n / (N - 1)))
        let window: Vec<f32> = (0..fft_size)
            .map(|n| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (fft_size - 1) as f32).cos())
            })
            .collect();

        // Precompute logarithmic frequency bands (from 20 Hz to min(20000 Hz, sample_rate / 2))
        let min_freq = 20.0f32;
        let max_freq = (sample_rate * 0.5).min(20000.0);
        let log_min = min_freq.ln();
        let log_max = max_freq.ln();
        let log_step = (log_max - log_min) / num_bands as f32;

        let bin_width = sample_rate / fft_size as f32;
        let mut band_ranges = Vec::with_capacity(num_bands);

        for b in 0..num_bands {
            let f_start = (log_min + b as f32 * log_step).exp();
            let f_end = (log_min + (b + 1) as f32 * log_step).exp();

            let mut start_bin = (f_start / bin_width).round() as usize;
            let mut end_bin = (f_end / bin_width).round() as usize;

            if start_bin < 1 {
                start_bin = 1;
            }
            if end_bin <= start_bin {
                end_bin = start_bin + 1;
            }
            if end_bin > fft_size / 2 {
                end_bin = fft_size / 2;
            }
            band_ranges.push((start_bin, end_bin));
        }

        Self {
            fft_size,
            sample_rate,
            num_bands,
            fft,
            window,
            band_ranges,
            smoothed_bands: vec![0.0; num_bands],
            complex_buffer: vec![Complex::new(0.0, 0.0); fft_size],
            scratch_buffer: vec![Complex::new(0.0, 0.0); scratch_len],
        }
    }

    /// Processes a slice of `fft_size` audio samples and returns normalized spectrum values in `[0.0, 1.0]`.
    pub fn analyze(&mut self, samples: &[f32]) -> &[f32] {
        if samples.len() < self.fft_size {
            return &self.smoothed_bands;
        }

        // Apply Hann window and populate complex buffer
        for (i, &sample) in samples.iter().enumerate().take(self.fft_size) {
            self.complex_buffer[i] = Complex::new(sample * self.window[i], 0.0);
        }

        // Perform in-place forward FFT
        self.fft
            .process_with_scratch(&mut self.complex_buffer, &mut self.scratch_buffer);

        // Group into logarithmic bands
        let norm_factor = 2.0 / self.fft_size as f32;
        for (b, &(start_bin, end_bin)) in self.band_ranges.iter().enumerate() {
            let mut max_mag = 0.0f32;
            for bin in start_bin..end_bin {
                let c = self.complex_buffer[bin];
                let mag = (c.re * c.re + c.im * c.im).sqrt() * norm_factor;
                if mag > max_mag {
                    max_mag = mag;
                }
            }

            // Power-scale for visual dynamic range and clamp
            let target_val = (max_mag * 4.0).clamp(0.0, 1.0);

            // Temporal attack / decay filter
            let attack = 0.75;
            let decay = 0.15;
            if target_val > self.smoothed_bands[b] {
                self.smoothed_bands[b] =
                    self.smoothed_bands[b] * (1.0 - attack) + target_val * attack;
            } else {
                self.smoothed_bands[b] =
                    self.smoothed_bands[b] * (1.0 - decay) + target_val * decay;
            }
        }

        &self.smoothed_bands
    }
}

struct StreamUserData {
    format: spa::param::audio::AudioInfoRaw,
    spectrum_handle: SpectrumHandle,
    analyzer: FftAnalyzer,
    sample_accumulator: Vec<f32>,
}

fn run_audio_capture_loop(
    init_tx: mpsc::Sender<Result<(), AudioError>>,
    spectrum_handle: SpectrumHandle,
    bands: usize,
) {
    pw::init();

    let mainloop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(m) => m,
        Err(e) => {
            let _ = init_tx.send(Err(AudioError::PipeWireUnavailable(format!(
                "Failed to create MainLoop: {e}"
            ))));
            return;
        }
    };

    let context = match pw::context::ContextRc::new(&mainloop, None) {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(AudioError::PipeWireUnavailable(format!(
                "Failed to create Context: {e}"
            ))));
            return;
        }
    };

    let core = match context.connect_rc(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(AudioError::PipeWireUnavailable(format!(
                "Failed to connect to PipeWire core: {e}"
            ))));
            return;
        }
    };

    let fft_size = 2048;
    let default_rate = 48000.0;
    let analyzer = FftAnalyzer::new(fft_size, default_rate, bands);

    let user_data = StreamUserData {
        format: Default::default(),
        spectrum_handle,
        analyzer,
        sample_accumulator: Vec::with_capacity(fft_size * 2),
    };

    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
        "target.object" => "@DEFAULT_AUDIO_SINK@",
    };

    let stream = match pw::stream::StreamBox::new(&core, "wallrs-audio-capture", props) {
        Ok(s) => s,
        Err(e) => {
            let _ = init_tx.send(Err(AudioError::StreamCreationFailed(format!(
                "Failed to create stream: {e}"
            ))));
            return;
        }
    };

    let listener_res = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }

            let (media_type, media_subtype) = match format_utils::parse_format(param) {
                Ok(v) => v,
                Err(_) => return,
            };

            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                return;
            }

            if user_data.format.parse(param).is_ok() {
                let rate = user_data.format.rate() as f32;
                if rate > 0.0 && (rate - user_data.analyzer.sample_rate).abs() > 1.0 {
                    user_data.analyzer = FftAnalyzer::new(
                        user_data.analyzer.fft_size,
                        rate,
                        user_data.analyzer.num_bands,
                    );
                }
                tracing::info!(
                    rate = user_data.format.rate(),
                    channels = user_data.format.channels(),
                    "PipeWire audio format negotiated"
                );
            }
        })
        .process(|stream, user_data| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                let n_channels = user_data.format.channels().max(1) as usize;
                let n_floats = data.chunk().size() as usize / std::mem::size_of::<f32>();

                if let Some(samples) = data.data()
                    && samples.len() >= n_floats * std::mem::size_of::<f32>()
                {
                    // Extract channel 0 or average channels to mono
                    let float_slice = unsafe {
                        std::slice::from_raw_parts(samples.as_ptr() as *const f32, n_floats)
                    };

                    for chunk in float_slice.chunks_exact(n_channels) {
                        let mono = chunk.iter().sum::<f32>() / n_channels as f32;
                        user_data.sample_accumulator.push(mono);
                    }

                    // When we have enough samples, analyze and publish
                    let fft_size = user_data.analyzer.fft_size;
                    while user_data.sample_accumulator.len() >= fft_size {
                        let spectrum_slice = user_data
                            .analyzer
                            .analyze(&user_data.sample_accumulator[..fft_size]);
                        user_data.spectrum_handle.update(spectrum_slice.to_vec());

                        // Slide by half the window (50% overlap for fluid visual responsiveness)
                        let hop_size = fft_size / 2;
                        user_data.sample_accumulator.drain(..hop_size);
                    }
                }
            }
        })
        .register();

    if let Err(e) = listener_res {
        let _ = init_tx.send(Err(AudioError::StreamCreationFailed(format!(
            "Failed to register stream listener: {e}"
        ))));
        return;
    }

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };

    let serialized = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    );

    let values: Vec<u8> = match serialized {
        Ok((cursor, _)) => cursor.into_inner(),
        Err(e) => {
            let _ = init_tx.send(Err(AudioError::StreamCreationFailed(format!(
                "Failed to serialize audio param: {e:?}"
            ))));
            return;
        }
    };

    let pod = match Pod::from_bytes(&values) {
        Some(p) => p,
        None => {
            let _ = init_tx.send(Err(AudioError::StreamCreationFailed(
                "Failed to deserialize SPA pod".into(),
            )));
            return;
        }
    };

    let mut params = [pod];

    let connect_res = stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    );

    if let Err(e) = connect_res {
        let _ = init_tx.send(Err(AudioError::StreamCreationFailed(format!(
            "Failed to connect PipeWire stream: {e}"
        ))));
        return;
    }

    // Inform parent that initialization succeeded
    let _ = init_tx.send(Ok(()));

    // Run the event loop
    mainloop.run();
}

/// Spawns the background PipeWire audio capture and FFT analysis thread.
pub fn spawn_capture(bands: usize) -> Result<(SpectrumHandle, JoinHandle<()>), AudioError> {
    let handle = SpectrumHandle::new(bands);
    let handle_clone = handle.clone();
    let (tx, rx) = mpsc::channel();

    let thread = std::thread::Builder::new()
        .name("wallrs-audio".into())
        .spawn(move || {
            run_audio_capture_loop(tx, handle_clone, bands);
        })
        .map_err(|e| AudioError::PipeWireUnavailable(e.to_string()))?;

    // Wait for stream to connect or fail with a 1.5 second timeout
    match rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(Ok(())) => Ok((handle, thread)),
        Ok(Err(e)) => Err(e),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(AudioError::Timeout(
            "PipeWire connection initialization timed out".into(),
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(AudioError::PipeWireUnavailable(
            "Audio thread terminated unexpectedly".into(),
        )),
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

        handle.update(vec![0.5; 32]);
        let data2 = handle.latest();
        assert_eq!(data2[0], 0.5);
    }

    #[test]
    fn test_fft_analyzer_sine_wave_detection() {
        let fft_size = 2048;
        let sample_rate = 48000.0;
        let num_bands = 32;
        let mut analyzer = FftAnalyzer::new(fft_size, sample_rate, num_bands);

        // Generate pure 440 Hz (Concert A) sine wave: y(t) = sin(2 * pi * 440 * t)
        let freq = 440.0f32;
        let samples: Vec<f32> = (0..fft_size)
            .map(|n| {
                let t = n as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect();

        let spectrum = analyzer.analyze(&samples);
        assert_eq!(spectrum.len(), num_bands);

        // Find the index of the maximum band
        let mut max_band = 0;
        let mut max_val = 0.0f32;
        for (i, &val) in spectrum.iter().enumerate() {
            if val > max_val {
                max_val = val;
                max_band = i;
            }
        }

        // Expected band for 440 Hz in 32 bands spanning 20Hz - 20000Hz:
        // log(440/20) / log(20000/20) * 32 = log(22) / log(1000) * 32 = 1.3424 / 3 * 32 ~= 14.3 -> Band 14
        assert!(
            (13..=15).contains(&max_band),
            "Expected 440 Hz peak around band 14, got band {max_band} with value {max_val}"
        );
        assert!(
            max_val > 0.3,
            "Expected significant magnitude at peak, got {max_val}"
        );

        // Low frequency test: 100 Hz (Bass)
        let freq_bass = 100.0f32;
        let samples_bass: Vec<f32> = (0..fft_size)
            .map(|n| {
                let t = n as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq_bass * t).sin()
            })
            .collect();

        // Reset analyzer state
        let mut analyzer_bass = FftAnalyzer::new(fft_size, sample_rate, num_bands);
        let spectrum_bass = analyzer_bass.analyze(&samples_bass);

        let mut max_bass_band = 0;
        let mut max_bass_val = 0.0f32;
        for (i, &val) in spectrum_bass.iter().enumerate() {
            if val > max_bass_val {
                max_bass_val = val;
                max_bass_band = i;
            }
        }

        // 100 Hz should be in a lower band than 440 Hz (around band 7)
        assert!(
            max_bass_band < max_band,
            "100 Hz band ({max_bass_band}) should be strictly lower than 440 Hz band ({max_band})"
        );
        assert!(
            (5..=9).contains(&max_bass_band),
            "Expected 100 Hz peak around band 7, got band {max_bass_band}"
        );
    }

    #[test]
    fn test_audio_metrics() {
        let mut bands = vec![0.0f32; 32];
        bands[0] = 0.8;
        bands[1] = 0.6;
        bands[5] = 0.4;
        bands[20] = 0.2;

        let metrics = AudioMetrics::from_spectrum(&bands);
        assert!(metrics.bass > 0.3);
        assert!(metrics.mid > 0.0);
        assert!(metrics.treble > 0.0);
        assert!(metrics.volume > 0.0);

        let empty = AudioMetrics::from_spectrum(&[]);
        assert_eq!(empty, AudioMetrics::default());
    }
}
