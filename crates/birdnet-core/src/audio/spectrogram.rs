//! Pure Rust mel spectrogram generation.
//!
//! Produces output numerically compatible with librosa's `melspectrogram`.
//! This is critical because `BirdNET` models were trained on librosa-generated
//! spectrograms — numerical equivalence ensures model accuracy.
//!
//! Uses `realfft` for the FFT computation (pure Rust, no C dependencies).

mod compute;
pub mod live;

use std::fmt;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

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

    let stft = compute::compute_stft(samples, config.n_fft, config.hop_length)?;
    let n_frames = stft.n_frames;
    let n_fft_bins = config.n_fft / 2 + 1;

    let power_spec = compute::compute_power_spectrum(&stft.magnitudes, config.power);

    let mel_filters = compute::mel_filterbank(
        config.n_mels,
        n_fft_bins,
        sample_rate as f32,
        config.fmin,
        fmax,
    );

    let mel_data = compute::apply_mel_filters(
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

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

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

    #[test]
    fn input_too_short_returns_error() {
        let samples = vec![0.0; 100];
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
    fn mel_spectrogram_sine_wave() {
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

        let max_db = db.data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min_db = db.data.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(max_db.is_finite(), "max dB should be finite");
        assert!(min_db.is_finite(), "min dB should be finite");
        assert!(
            (min_db - (max_db - 80.0)).abs() < 0.01,
            "floor should be max_db - top_db"
        );
    }
}
