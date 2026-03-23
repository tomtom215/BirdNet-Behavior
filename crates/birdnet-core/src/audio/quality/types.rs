//! Audio quality assessment types.
//!
//! Provides the result types and threshold configuration for the
//! quality assessment pipeline applied to each audio chunk before
//! it enters the ML inference stage.

use std::fmt;

/// Composite quality score for an audio chunk.
///
/// Aggregates several acoustic metrics to determine whether
/// the chunk is suitable for bird detection inference.
/// Unusable chunks are discarded before the mel spectrogram
/// is computed, saving CPU cycles and suppressing false positives
/// caused by rain, wind, or clipping.
#[derive(Debug, Clone)]
pub struct QualityScore {
    /// Signal-to-noise ratio estimate in dB.
    ///
    /// Computed by comparing the peak short-time RMS to the lowest
    /// percentile of frame energies (noise floor proxy).
    /// Typical threshold: 3.0 dB.
    pub snr_db: f32,

    /// Spectral flatness measure \[0.0, 1.0\].
    ///
    /// Ratio of geometric mean to arithmetic mean of the power spectrum.
    /// 0 = perfectly tonal (single sine wave), 1 = flat white noise.
    /// Bird vocalisations are tonal; values < 0.7 are typical.
    pub spectral_flatness: f32,

    /// Estimated noise floor in dBFS.
    ///
    /// Negative values relative to full-scale. Typical quiet outdoor
    /// background: −60 to −40 dBFS. Values above −20 dBFS indicate
    /// strong broadband interference.
    pub noise_floor_dbfs: f32,

    /// Rain or impulsive noise detected.
    ///
    /// Set when the high-frequency energy fraction and temporal
    /// variance of frame energy both exceed empirical thresholds.
    pub rain_detected: bool,

    /// Composite usability score \[0.0, 1.0\].
    ///
    /// Weighted combination of SNR score, spectral flatness score,
    /// and a rain penalty. Scores below [`QualityThresholds::min_score`]
    /// (default 0.25) are considered unusable.
    pub score: f32,
}

impl QualityScore {
    /// Return `true` if the chunk passes all configured thresholds.
    #[must_use]
    pub fn is_usable(&self, thresholds: &QualityThresholds) -> bool {
        !self.rain_detected
            && self.snr_db >= thresholds.min_snr_db
            && self.spectral_flatness <= thresholds.max_spectral_flatness
            && self.score >= thresholds.min_score
    }
}

impl fmt::Display for QualityScore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "QualityScore {{ snr={:.1}dB flatness={:.3} floor={:.1}dBFS rain={} score={:.2} }}",
            self.snr_db,
            self.spectral_flatness,
            self.noise_floor_dbfs,
            self.rain_detected,
            self.score,
        )
    }
}

/// Thresholds used to classify a [`QualityScore`] as usable or not.
///
/// Tune these based on your deployment environment.  Outdoor deployments
/// with noisy backgrounds may need lower `min_snr_db`.
#[derive(Debug, Clone)]
pub struct QualityThresholds {
    /// Minimum acceptable SNR in dB.
    ///
    /// Default: 3.0 dB.  Lower values accept noisier audio.
    pub min_snr_db: f32,

    /// Maximum acceptable spectral flatness.
    ///
    /// Default: 0.85.  Values approaching 1.0 indicate white-noise-like
    /// audio with no tonal content.
    pub max_spectral_flatness: f32,

    /// Minimum composite quality score.
    ///
    /// Default: 0.25.
    pub min_score: f32,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            min_snr_db: 3.0,
            max_spectral_flatness: 0.85,
            min_score: 0.25,
        }
    }
}

/// Error returned by the quality assessment pipeline.
#[derive(Debug)]
pub enum QualityError {
    /// Audio chunk is too short for reliable analysis.
    TooShort { len: usize, required: usize },
    /// Sample rate is not supported.
    UnsupportedSampleRate(u32),
}

impl fmt::Display for QualityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { len, required } => {
                write!(f, "audio too short: {len} samples, need >= {required}")
            }
            Self::UnsupportedSampleRate(r) => {
                write!(f, "unsupported sample rate: {r} Hz (expected 48000)")
            }
        }
    }
}

impl std::error::Error for QualityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds_reasonable() {
        let t = QualityThresholds::default();
        assert!(t.min_snr_db > 0.0);
        assert!(t.max_spectral_flatness < 1.0);
        assert!(t.min_score > 0.0 && t.min_score < 1.0);
    }

    #[test]
    fn is_usable_high_quality() {
        let score = QualityScore {
            snr_db: 15.0,
            spectral_flatness: 0.3,
            noise_floor_dbfs: -50.0,
            rain_detected: false,
            score: 0.8,
        };
        assert!(score.is_usable(&QualityThresholds::default()));
    }

    #[test]
    fn is_usable_rain_detected() {
        let score = QualityScore {
            snr_db: 20.0,
            spectral_flatness: 0.4,
            noise_floor_dbfs: -40.0,
            rain_detected: true,
            score: 0.9,
        };
        // Rain flag alone is enough to fail
        assert!(!score.is_usable(&QualityThresholds::default()));
    }

    #[test]
    fn is_usable_low_snr() {
        let score = QualityScore {
            snr_db: 1.5,
            spectral_flatness: 0.3,
            noise_floor_dbfs: -30.0,
            rain_detected: false,
            score: 0.6,
        };
        assert!(!score.is_usable(&QualityThresholds::default()));
    }

    #[test]
    fn display_formats_correctly() {
        let score = QualityScore {
            snr_db: 12.3,
            spectral_flatness: 0.456,
            noise_floor_dbfs: -48.7,
            rain_detected: false,
            score: 0.72,
        };
        let s = format!("{score}");
        assert!(s.contains("12.3dB"));
        assert!(s.contains("0.456"));
    }

    #[test]
    fn quality_error_display() {
        let e = QualityError::TooShort {
            len: 100,
            required: 1024,
        };
        assert!(format!("{e}").contains("100"));
        let e2 = QualityError::UnsupportedSampleRate(22_050);
        assert!(format!("{e2}").contains("22050"));
    }
}
