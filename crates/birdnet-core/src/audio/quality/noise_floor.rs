//! Adaptive minimum-statistics noise floor tracker.
//!
//! Tracks a running estimate of the background noise floor across
//! successive audio frames using the minimum-statistics algorithm.
//! The tracker maintains a sliding window of smoothed short-time power
//! values and reports the minimum as the noise floor estimate.
//!
//! Designed for continuous operation: call [`NoiseFloorTracker::update`]
//! with each new audio frame; call [`NoiseFloorTracker::noise_floor_dbfs`]
//! to query the current estimate.
//!
//! ## References
//!
//! R. Martin, "Noise Power Spectral Density Estimation Based on Optimal
//! Smoothing and Minimum Statistics", *IEEE Trans. Speech Audio Process.*,
//! 9(5):504–512, 2001.

/// Frame size in samples for short-time power estimation.
const FRAME_SAMPLES: usize = 512;

/// Number of frames in the sliding window (~3 s at 48 kHz, `FRAME_SAMPLES=512`).
const WINDOW_FRAMES: usize = 64;

/// Smoothing factor for the exponential power average (0 < α < 1).
/// Higher = slower adaptation, more stable estimate.
const SMOOTHING_ALPHA: f32 = 0.95;

/// Noise overestimation correction factor.
///
/// The minimum-statistics algorithm slightly underestimates the noise
/// floor; multiplying by this factor compensates.
const OVERESTIMATION: f32 = 1.6;

/// Adaptive minimum-statistics noise floor tracker.
///
/// Call [`update`](Self::update) once per audio frame.
/// After [`WINDOW_FRAMES`] frames the tracker is *calibrated* and
/// [`noise_floor_dbfs`](Self::noise_floor_dbfs) returns a reliable
/// estimate.
#[derive(Debug, Clone)]
pub struct NoiseFloorTracker {
    /// Circular buffer of smoothed short-time power values.
    window: Vec<f32>,
    /// Next write index in the circular buffer.
    pos: usize,
    /// Current exponentially-smoothed power estimate.
    smoothed_power: f32,
    /// Whether the window has been fully populated at least once.
    full: bool,
}

impl NoiseFloorTracker {
    /// Create a new, uncalibrated tracker.
    ///
    /// Call [`update`](Self::update) at least [`WINDOW_FRAMES`] times
    /// before relying on the noise floor estimate.
    #[must_use]
    pub fn new() -> Self {
        Self {
            window: vec![1e-10_f32; WINDOW_FRAMES],
            pos: 0,
            smoothed_power: 1e-10,
            full: false,
        }
    }

    /// Update the tracker with the next audio frame.
    ///
    /// Only the first [`FRAME_SAMPLES`] samples of `samples` are used;
    /// extra samples are ignored.  If `samples` is shorter than
    /// [`FRAME_SAMPLES`] all samples are used.
    ///
    /// Returns the current noise floor estimate in linear RMS amplitude.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, samples: &[f32]) -> f32 {
        let n = samples.len().min(FRAME_SAMPLES);
        let n_f = n as f32;

        let frame_power = if n == 0 {
            0.0
        } else {
            samples[..n].iter().map(|s| s * s).sum::<f32>() / n_f
        };

        // Exponential smoothing of short-time power
        self.smoothed_power =
            SMOOTHING_ALPHA.mul_add(self.smoothed_power, (1.0 - SMOOTHING_ALPHA) * frame_power);

        // Store in circular window
        self.window[self.pos] = self.smoothed_power;
        self.pos += 1;
        if self.pos >= WINDOW_FRAMES {
            self.pos = 0;
            self.full = true;
        }

        self.noise_floor_rms()
    }

    /// Current noise floor estimate in linear RMS amplitude.
    #[must_use]
    pub fn noise_floor_rms(&self) -> f32 {
        let min_power = self
            .window
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            .max(1e-20_f32);
        (min_power * OVERESTIMATION).sqrt()
    }

    /// Current noise floor estimate in dBFS.
    ///
    /// Returns values ≤ 0 dBFS. Returns −96 dBFS for effectively
    /// silent conditions.
    #[must_use]
    pub fn noise_floor_dbfs(&self) -> f32 {
        let rms = self.noise_floor_rms();
        if rms < 1e-10 {
            return -96.0;
        }
        20.0 * rms.log10()
    }

    /// Return `true` once the sliding window is fully populated.
    ///
    /// Before calibration the estimate is computed from an
    /// initialised-to-silence window and may be unreliable.
    #[must_use]
    pub const fn is_calibrated(&self) -> bool {
        self.full
    }

    /// Reset the tracker to an uncalibrated state.
    pub fn reset(&mut self) {
        self.window.fill(1e-10);
        self.pos = 0;
        self.smoothed_power = 1e-10;
        self.full = false;
    }
}

impl Default for NoiseFloorTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn silence(n: usize) -> Vec<f32> {
        vec![0.0_f32; n]
    }

    fn loud_noise(n: usize, amplitude: f32) -> Vec<f32> {
        vec![amplitude; n]
    }

    fn tone(freq_hz: f32, amplitude: f32, n_samples: usize, sample_rate: u32) -> Vec<f32> {
        (0..n_samples)
            .map(|i| amplitude * (2.0 * PI * freq_hz * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn new_tracker_is_not_calibrated() {
        assert!(!NoiseFloorTracker::new().is_calibrated());
    }

    #[test]
    fn calibrated_after_window_frames() {
        let mut t = NoiseFloorTracker::new();
        let frame = silence(FRAME_SAMPLES);
        for _ in 0..WINDOW_FRAMES {
            t.update(&frame);
        }
        assert!(t.is_calibrated());
    }

    #[test]
    fn noise_floor_drops_after_quiet_period() {
        let mut t = NoiseFloorTracker::new();
        let loud = loud_noise(FRAME_SAMPLES, 0.5);
        let quiet = silence(FRAME_SAMPLES);

        // Fill window with loud noise
        for _ in 0..WINDOW_FRAMES {
            t.update(&loud);
        }
        let floor_loud = t.noise_floor_rms();

        // Fill window again with silence
        for _ in 0..WINDOW_FRAMES {
            t.update(&quiet);
        }
        let floor_quiet = t.noise_floor_rms();

        assert!(
            floor_quiet < floor_loud,
            "noise floor should drop after quiet period: {floor_quiet} >= {floor_loud}"
        );
    }

    #[test]
    fn noise_floor_dbfs_is_negative() {
        let mut t = NoiseFloorTracker::new();
        let frame = loud_noise(FRAME_SAMPLES, 0.1);
        for _ in 0..WINDOW_FRAMES {
            t.update(&frame);
        }
        let dbfs = t.noise_floor_dbfs();
        assert!(dbfs < 0.0, "dBFS should be negative, got {dbfs}");
    }

    #[test]
    fn reset_clears_calibration() {
        let mut t = NoiseFloorTracker::new();
        let frame = silence(FRAME_SAMPLES);
        for _ in 0..WINDOW_FRAMES {
            t.update(&frame);
        }
        assert!(t.is_calibrated());
        t.reset();
        assert!(!t.is_calibrated());
    }

    #[test]
    fn tone_has_stable_noise_floor() {
        let mut t = NoiseFloorTracker::new();
        let frame = tone(2000.0, 0.05, FRAME_SAMPLES, 48_000);
        for _ in 0..WINDOW_FRAMES * 2 {
            t.update(&frame);
        }
        // A sustained constant-amplitude tone has a well-defined noise floor
        let floor = t.noise_floor_dbfs();
        assert!(
            floor < 0.0 && floor > -80.0,
            "tone noise floor out of range: {floor}"
        );
    }

    #[test]
    fn short_frame_does_not_panic() {
        let mut t = NoiseFloorTracker::new();
        let short = vec![0.1_f32; 16];
        for _ in 0..WINDOW_FRAMES {
            t.update(&short);
        }
        assert!(t.is_calibrated());
    }

    #[test]
    fn empty_frame_does_not_panic() {
        let mut t = NoiseFloorTracker::new();
        t.update(&[]);
        // Should not panic; floor remains initialised value
        assert!(t.noise_floor_rms() >= 0.0);
    }
}
