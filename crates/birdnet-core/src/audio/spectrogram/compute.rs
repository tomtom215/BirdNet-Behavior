//! Internal STFT and mel filterbank computations.

use realfft::RealFftPlanner;

use super::SpectrogramError;

// ---------------------------------------------------------------------------
// STFT
// ---------------------------------------------------------------------------

pub(super) struct StftResult {
    /// Magnitude values: `[n_fft_bins][n_frames]` in row-major order.
    pub(super) magnitudes: Vec<f32>,
    pub(super) n_frames: usize,
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn compute_stft(
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

// ---------------------------------------------------------------------------
// Power spectrum
// ---------------------------------------------------------------------------

pub(super) fn compute_power_spectrum(magnitudes: &[f32], power: f32) -> Vec<f32> {
    if (power - 1.0).abs() < f32::EPSILON {
        magnitudes.to_vec()
    } else if (power - 2.0).abs() < f32::EPSILON {
        magnitudes.iter().map(|&m| m * m).collect()
    } else {
        magnitudes.iter().map(|&m| m.powf(power)).collect()
    }
}

// ---------------------------------------------------------------------------
// Mel filterbank
// ---------------------------------------------------------------------------

/// Build a mel filterbank matrix: `[n_mels][n_fft_bins]` in row-major order.
///
/// Matches librosa's `mel()` with HTK mel scale and Slaney normalization.
#[allow(clippy::cast_precision_loss)]
pub(super) fn mel_filterbank(
    n_mels: usize,
    n_fft_bins: usize,
    sample_rate: f32,
    fmin: f32,
    fmax: f32,
) -> Vec<f32> {
    let fmin_mel = hz_to_mel(fmin);
    let fmax_mel = hz_to_mel(fmax);

    let n_points = n_mels + 2;
    let mel_points: Vec<f32> = (0..n_points)
        .map(|i| fmin_mel + (fmax_mel - fmin_mel) * i as f32 / (n_points - 1) as f32)
        .collect();

    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    let fft_freqs: Vec<f32> = (0..n_fft_bins)
        .map(|i| i as f32 * sample_rate / ((n_fft_bins - 1) as f32 * 2.0))
        .collect();

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
pub(super) fn apply_mel_filters(
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

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Hann window function (periodic, matching librosa STFT convention).
#[allow(clippy::cast_precision_loss)]
pub(super) fn hann_window(size: usize) -> Vec<f32> {
    let n = size as f32;
    (0..size)
        .map(|i| {
            let phase = i as f32 * std::f32::consts::TAU / n;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// Convert frequency in Hz to mel scale (HTK formula).
pub(super) fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// Convert mel scale to frequency in Hz (HTK formula).
pub(super) fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

// ---------------------------------------------------------------------------
// Unit tests for internal compute functions
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn hann_window_properties() {
        let w = hann_window(256);
        assert_eq!(w.len(), 256);
        assert!(w[0].abs() < 1e-6);
        assert!((w[128] - 1.0).abs() < 0.01);
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
        let n_fft_bins = 1025;
        let filters = mel_filterbank(n_mels, n_fft_bins, 48000.0, 0.0, 24000.0);
        assert_eq!(filters.len(), n_mels * n_fft_bins);

        for mel in 0..n_mels {
            let row = &filters[mel * n_fft_bins..(mel + 1) * n_fft_bins];
            let sum: f32 = row.iter().sum();
            assert!(sum > 0.0, "mel filter {mel} is all zeros");
        }
    }
}
