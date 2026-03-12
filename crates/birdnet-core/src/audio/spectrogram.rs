//! Pure Rust mel spectrogram generation.
//!
//! Produces output numerically compatible with librosa's `melspectrogram`.
//! This is critical because `BirdNET` models were trained on librosa-generated
//! spectrograms -- numerical equivalence ensures model accuracy.
//!
//! Uses `realfft` for the FFT computation (pure Rust, no C dependencies).
//! All other math (Hann window, mel filterbank, power spectrum) is hand-rolled.

use std::fmt;

use realfft::RealFftPlanner;

/// Mel spectrogram parameters matching librosa defaults for `BirdNET`.
#[derive(Debug, Clone)]
pub struct MelConfig {
    /// FFT window size.
    pub n_fft: usize,
    /// Number of samples between successive frames.
    pub hop_length: usize,
    /// Number of mel frequency bands.
    pub n_mels: usize,
    /// Minimum frequency for mel filterbank (Hz).
    pub fmin: f32,
    /// Maximum frequency for mel filterbank (Hz). If `None`, uses `sample_rate / 2`.
    pub fmax: Option<f32>,
    /// Power of the magnitude spectrogram (1.0 = amplitude, 2.0 = power).
    pub power: f32,
}

impl Default for MelConfig {
    fn default() -> Self {
        Self {
            n_fft: 2048,
            hop_length: 512,
            n_mels: 128,
            fmin: 0.0,
            fmax: None,
            power: 2.0,
        }
    }
}

/// Result of mel spectrogram computation.
#[derive(Debug, Clone)]
pub struct MelSpectrogram {
    /// Mel spectrogram data in row-major order: `[n_mels][n_frames]`.
    pub data: Vec<f32>,
    /// Number of mel bands (rows).
    pub n_mels: usize,
    /// Number of time frames (columns).
    pub n_frames: usize,
}

impl MelSpectrogram {
    /// Access a single value at `(mel_band, frame)`.
    pub fn get(&self, mel: usize, frame: usize) -> f32 {
        self.data[mel * self.n_frames + frame]
    }

    /// Convert to log scale (power to dB), matching `librosa.power_to_db()`.
    ///
    /// Applies `10 * log10(max(S, ref_value))` with a floor at `top_db` below peak.
    #[must_use]
    pub fn to_db(&self, ref_value: f32, top_db: f32) -> Self {
        let log_ref = 10.0 * ref_value.max(f32::MIN_POSITIVE).log10();
        let mut db_data = Vec::with_capacity(self.data.len());

        for &val in &self.data {
            let db = 10.0_f32.mul_add(val.max(f32::MIN_POSITIVE).log10(), -log_ref);
            db_data.push(db);
        }

        // Apply top_db floor
        let max_db = db_data.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        let floor = max_db - top_db;
        for val in &mut db_data {
            *val = val.max(floor);
        }

        Self {
            data: db_data,
            n_mels: self.n_mels,
            n_frames: self.n_frames,
        }
    }
}

/// Errors during spectrogram computation.
#[derive(Debug)]
pub enum SpectrogramError {
    /// Input audio is too short for the configured FFT size.
    InputTooShort { samples: usize, n_fft: usize },
    /// Invalid configuration parameters.
    InvalidConfig(String),
    /// FFT computation failed.
    Fft(String),
}

impl fmt::Display for SpectrogramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooShort { samples, n_fft } => {
                write!(f, "input too short: {samples} samples < n_fft {n_fft}")
            }
            Self::InvalidConfig(msg) => write!(f, "invalid spectrogram config: {msg}"),
            Self::Fft(msg) => write!(f, "FFT error: {msg}"),
        }
    }
}

impl std::error::Error for SpectrogramError {}

/// Compute a mel spectrogram from mono f32 audio samples.
///
/// Matches librosa's `melspectrogram` output for `BirdNET` model compatibility.
///
/// # Errors
///
/// Returns [`SpectrogramError`] if the input is too short or config is invalid.
#[allow(clippy::cast_precision_loss)]
pub fn mel_spectrogram(
    samples: &[f32],
    sample_rate: u32,
    config: &MelConfig,
) -> Result<MelSpectrogram, SpectrogramError> {
    if config.n_fft == 0 || config.hop_length == 0 || config.n_mels == 0 {
        return Err(SpectrogramError::InvalidConfig(
            "n_fft, hop_length, n_mels must be non-zero".into(),
        ));
    }
    if samples.len() < config.n_fft {
        return Err(SpectrogramError::InputTooShort {
            samples: samples.len(),
            n_fft: config.n_fft,
        });
    }

    let fmax = config.fmax.unwrap_or(sample_rate as f32 / 2.0);

    // Step 1: Compute STFT (Short-Time Fourier Transform)
    let stft = compute_stft(samples, config.n_fft, config.hop_length)?;
    let n_frames = stft.n_frames;
    let n_fft_bins = config.n_fft / 2 + 1;

    // Step 2: Compute power spectrogram
    let power_spec = compute_power_spectrum(&stft.magnitudes, config.power);

    // Step 3: Build mel filterbank
    let mel_filters = mel_filterbank(
        config.n_mels,
        n_fft_bins,
        sample_rate as f32,
        config.fmin,
        fmax,
    );

    // Step 4: Apply mel filterbank to power spectrogram
    let mel_data = apply_mel_filters(
        &mel_filters,
        &power_spec,
        config.n_mels,
        n_fft_bins,
        n_frames,
    );

    Ok(MelSpectrogram {
        data: mel_data,
        n_mels: config.n_mels,
        n_frames,
    })
}

// --- Internal: STFT ---

struct StftResult {
    /// Magnitude values: `[n_fft_bins][n_frames]` in row-major order.
    magnitudes: Vec<f32>,
    n_frames: usize,
}

#[allow(clippy::cast_precision_loss)]
fn compute_stft(
    samples: &[f32],
    n_fft: usize,
    hop_length: usize,
) -> Result<StftResult, SpectrogramError> {
    let n_fft_bins = n_fft / 2 + 1;

    // librosa centers frames by padding with n_fft/2 on each side
    let pad = n_fft / 2;
    let padded_len = samples.len() + 2 * pad;
    let mut padded = vec![0.0_f32; padded_len];
    padded[pad..pad + samples.len()].copy_from_slice(samples);

    // Reflect padding (matching librosa's default reflect mode)
    for i in 0..pad.min(samples.len()) {
        padded[pad - 1 - i] = samples[i.min(samples.len() - 1)];
        let right_idx = pad + samples.len() + i;
        if right_idx < padded_len {
            padded[right_idx] = samples[samples.len() - 1 - i.min(samples.len() - 1)];
        }
    }

    let n_frames = 1 + (padded_len - n_fft) / hop_length;
    let window = hann_window(n_fft);

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n_fft);

    let mut magnitudes = vec![0.0_f32; n_fft_bins * n_frames];
    let mut fft_input = vec![0.0_f32; n_fft];
    let mut fft_output = fft.make_output_vec();

    for frame in 0..n_frames {
        let start = frame * hop_length;

        // Apply window
        for (i, sample) in fft_input.iter_mut().enumerate() {
            *sample = padded[start + i] * window[i];
        }

        // Forward FFT
        fft.process(&mut fft_input, &mut fft_output)
            .map_err(|e| SpectrogramError::Fft(e.to_string()))?;

        // Compute magnitude for each frequency bin
        for (bin, complex) in fft_output.iter().enumerate() {
            let mag = complex.re.hypot(complex.im);
            magnitudes[bin * n_frames + frame] = mag;
        }
    }

    Ok(StftResult {
        magnitudes,
        n_frames,
    })
}

// --- Internal: Power spectrum ---

fn compute_power_spectrum(magnitudes: &[f32], power: f32) -> Vec<f32> {
    if (power - 1.0).abs() < f32::EPSILON {
        // Amplitude spectrogram (power=1.0), magnitudes already computed
        magnitudes.to_vec()
    } else if (power - 2.0).abs() < f32::EPSILON {
        // Power spectrogram (power=2.0), square the magnitudes
        magnitudes.iter().map(|&m| m * m).collect()
    } else {
        magnitudes.iter().map(|&m| m.powf(power)).collect()
    }
}

// --- Internal: Mel filterbank ---

/// Build a mel filterbank matrix: `[n_mels][n_fft_bins]` in row-major order.
///
/// Matches librosa's `mel()` with HTK mel scale and Slaney normalization.
#[allow(clippy::cast_precision_loss)]
fn mel_filterbank(
    n_mels: usize,
    n_fft_bins: usize,
    sample_rate: f32,
    fmin: f32,
    fmax: f32,
) -> Vec<f32> {
    // Convert Hz to mel scale (HTK formula)
    let fmin_mel = hz_to_mel(fmin);
    let fmax_mel = hz_to_mel(fmax);

    // n_mels + 2 evenly spaced points in mel space
    let n_points = n_mels + 2;
    let mel_points: Vec<f32> = (0..n_points)
        .map(|i| fmin_mel + (fmax_mel - fmin_mel) * i as f32 / (n_points - 1) as f32)
        .collect();

    // Convert mel points back to Hz
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    // Convert Hz to FFT bin indices (float for interpolation)
    let fft_freqs: Vec<f32> = (0..n_fft_bins)
        .map(|i| i as f32 * sample_rate / ((n_fft_bins - 1) as f32 * 2.0))
        .collect();

    // Build triangular filters
    let mut filters = vec![0.0_f32; n_mels * n_fft_bins];

    for mel in 0..n_mels {
        let f_left = hz_points[mel];
        let f_center = hz_points[mel + 1];
        let f_right = hz_points[mel + 2];

        for (bin, &freq) in fft_freqs.iter().enumerate() {
            if freq >= f_left && freq <= f_center && f_center > f_left {
                filters[mel * n_fft_bins + bin] = (freq - f_left) / (f_center - f_left);
            } else if freq > f_center && freq <= f_right && f_right > f_center {
                filters[mel * n_fft_bins + bin] = (f_right - freq) / (f_right - f_center);
            }
        }

        // Slaney normalization: normalize area of each filter to 1
        let enorm = 2.0 / (hz_points[mel + 2] - hz_points[mel]);
        for bin in 0..n_fft_bins {
            filters[mel * n_fft_bins + bin] *= enorm;
        }
    }

    filters
}

/// Apply mel filterbank to power spectrogram via matrix multiplication.
///
/// `filters`: `[n_mels][n_fft_bins]`, `power_spec`: `[n_fft_bins][n_frames]`
/// Result: `[n_mels][n_frames]`
fn apply_mel_filters(
    filters: &[f32],
    power_spec: &[f32],
    n_mels: usize,
    n_fft_bins: usize,
    n_frames: usize,
) -> Vec<f32> {
    let mut mel_spec = vec![0.0_f32; n_mels * n_frames];

    for mel in 0..n_mels {
        for frame in 0..n_frames {
            let mut sum = 0.0_f32;
            for bin in 0..n_fft_bins {
                sum += filters[mel * n_fft_bins + bin] * power_spec[bin * n_frames + frame];
            }
            mel_spec[mel * n_frames + frame] = sum;
        }
    }

    mel_spec
}

// --- Internal: Utility functions ---

/// Hann window function (periodic, matching librosa STFT convention).
#[allow(clippy::cast_precision_loss)]
fn hann_window(size: usize) -> Vec<f32> {
    let n = size as f32;
    (0..size)
        .map(|i| {
            let phase = i as f32 * std::f32::consts::TAU / n;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// Convert frequency in Hz to mel scale (HTK formula).
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Convert mel scale to frequency in Hz (HTK formula).
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn hann_window_properties() {
        let w = hann_window(256);
        assert_eq!(w.len(), 256);
        // First value should be zero (periodic Hann window)
        assert!(w[0].abs() < 1e-6);
        // Middle value should be near 1.0
        assert!((w[128] - 1.0).abs() < 0.01);
        // All values should be in [0, 1]
        for &val in &w {
            assert!(
                (0.0..=1.0).contains(&val),
                "window value {val} out of range"
            );
        }
    }

    #[test]
    fn mel_hz_roundtrip() {
        for &hz in &[0.0, 100.0, 440.0, 1000.0, 4000.0, 8000.0, 22050.0] {
            let mel = hz_to_mel(hz);
            let back = mel_to_hz(mel);
            assert!(
                (hz - back).abs() < 0.01,
                "roundtrip failed for {hz}: got {back}"
            );
        }
    }

    #[test]
    fn mel_scale_is_monotonic() {
        let freqs = [0.0, 100.0, 500.0, 1000.0, 5000.0, 10000.0];
        let mels: Vec<f32> = freqs.iter().map(|&f| hz_to_mel(f)).collect();
        for i in 1..mels.len() {
            assert!(mels[i] > mels[i - 1], "mel scale should be monotonic");
        }
    }

    #[test]
    fn mel_filterbank_shape() {
        let n_mels = 128;
        let n_fft_bins = 1025; // n_fft=2048 -> 1025 bins
        let filters = mel_filterbank(n_mels, n_fft_bins, 48000.0, 0.0, 24000.0);
        assert_eq!(filters.len(), n_mels * n_fft_bins);

        // Each filter should have at least some non-zero values
        for mel in 0..n_mels {
            let row = &filters[mel * n_fft_bins..(mel + 1) * n_fft_bins];
            let sum: f32 = row.iter().sum();
            assert!(sum > 0.0, "mel filter {mel} is all zeros");
        }
    }

    #[test]
    fn mel_spectrogram_sine_wave() {
        // Generate a 440Hz sine wave at 48kHz, 1 second
        let sample_rate = 48000_u32;
        let duration_samples = sample_rate as usize;
        let samples: Vec<f32> = (0..duration_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect();

        let config = MelConfig::default();
        let result = mel_spectrogram(&samples, sample_rate, &config).unwrap();

        assert_eq!(result.n_mels, 128);
        assert!(result.n_frames > 0);
        assert_eq!(result.data.len(), result.n_mels * result.n_frames);

        // The 440Hz energy should be concentrated in lower mel bands
        // Sum energy in bottom quarter vs top quarter
        let mut bottom_energy = 0.0_f32;
        for mel in 0..32 {
            for f in 0..result.n_frames {
                bottom_energy += result.get(mel, f);
            }
        }
        let mut top_energy = 0.0_f32;
        for mel in 96..128 {
            for f in 0..result.n_frames {
                top_energy += result.get(mel, f);
            }
        }

        assert!(
            bottom_energy > top_energy,
            "440Hz energy should be in lower mel bands: bottom={bottom_energy}, top={top_energy}"
        );
    }

    #[test]
    fn input_too_short_returns_error() {
        let samples = vec![0.0; 100]; // Less than n_fft=2048
        let config = MelConfig::default();
        let result = mel_spectrogram(&samples, 48000, &config);
        assert!(matches!(
            result,
            Err(SpectrogramError::InputTooShort { .. })
        ));
    }

    #[test]
    fn invalid_config_returns_error() {
        let samples = vec![0.0; 4096];
        let config = MelConfig {
            n_fft: 0,
            ..MelConfig::default()
        };
        let result = mel_spectrogram(&samples, 48000, &config);
        assert!(matches!(result, Err(SpectrogramError::InvalidConfig(_))));
    }

    #[test]
    fn to_db_conversion() {
        let sample_rate = 48000_u32;
        let samples: Vec<f32> = (0..48000)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
            })
            .collect();

        let config = MelConfig::default();
        let mel = mel_spectrogram(&samples, sample_rate, &config).unwrap();
        let db = mel.to_db(1.0, 80.0);

        assert_eq!(db.n_mels, mel.n_mels);
        assert_eq!(db.n_frames, mel.n_frames);

        // dB values should be finite and bounded by top_db floor
        let max_db = db.data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min_db = db.data.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(max_db.is_finite(), "max dB should be finite");
        assert!(min_db.is_finite(), "min dB should be finite");
        // Floor should be max_db - top_db
        assert!(
            (min_db - (max_db - 80.0)).abs() < 0.01,
            "floor should be max_db - top_db"
        );
    }

    #[test]
    fn default_config_matches_librosa_defaults() {
        let config = MelConfig::default();
        assert_eq!(config.n_fft, 2048);
        assert_eq!(config.hop_length, 512);
        assert_eq!(config.n_mels, 128);
        assert!((config.fmin - 0.0).abs() < f32::EPSILON);
        assert!(config.fmax.is_none());
        assert!((config.power - 2.0).abs() < f32::EPSILON);
    }
}
