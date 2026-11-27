//! What capture did to the audio before the classifier heard it (audit ACC-06).
//!
//! A source's pipeline toggles (`dc_removal`, `high_pass`, `agc`), its gain and
//! its equaliser chain change the samples a classifier scores, and they are on
//! by default (a 5 Hz DC block and a 120 Hz high-pass). Every parity
//! measurement in `docs/book/guides/parity.md` was made on files dropped into
//! the watch directory, which skip capture; measured once with the real V2.4
//! model on the magpie clip, the default filters moved its top-10 logits by a
//! median of 1.21 (at most 3.63) from BirdNET-Pi's. So which conditioning a
//! detection's audio went through is part of its provenance, and it was not
//! recorded anywhere a researcher could read it back.
//!
//! [`describe`] renders it as one canonical, versioned string
//! ([`CONDITIONING_VERSION`]), so a run record can say exactly what was applied;
//! [`InputChange`] measures what it does to a window of samples.
//!
//! # The two backends differ
//!
//! The flags are applied by a one-pole IIR in the microphone tee and by
//! ffmpeg's two-pole `highpass` on RTSP and `PipeWire` sources
//! (`audio::capture::types::AudioPipeline`), so one flag is two filters. The
//! descriptor names the backend and the filter order; an operator's equaliser
//! chain replaces the flags on both and is written out as its own
//! specification.

use crate::audio::capture::types::{AudioPipeline, DC_BLOCK_CUTOFF_HZ, HIGH_PASS_CUTOFF_HZ};
use crate::audio::eq::EqChain;

/// The version of the descriptor's format.
pub const CONDITIONING_VERSION: &str = "conditioning-1";

/// Which path the audio took into the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    /// The microphone tee (`arecord`): one-pole filters, peak AGC.
    Tee,
    /// ffmpeg (RTSP, `PipeWire`): its `-af` chain.
    Ffmpeg,
    /// A file that did not pass through capture (the watch directory, an
    /// ingested recorder card): nothing applied by this station.
    NotCaptured,
}

impl CaptureBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tee => "tee",
            Self::Ffmpeg => "ffmpeg",
            Self::NotCaptured => "not-captured",
        }
    }
}

/// The conditioning a source's audio went through, as one canonical string.
///
/// Stage order is signal order. Examples:
///
/// ```text
/// conditioning-1 backend=tee dc=highpass:5Hz:1pole hp=highpass:120Hz:1pole gain=0dB agc=off
/// conditioning-1 backend=ffmpeg eq="highpass:120; notch:50:20" gain=-3dB agc=dynaudnorm
/// conditioning-1 backend=not-captured
/// ```
///
/// The same inputs always give the same string, and any change to what is
/// applied changes it, so it can be compared across runs as text.
#[must_use]
pub fn describe(
    backend: CaptureBackend,
    pipeline: AudioPipeline,
    eq: &EqChain,
    gain_db: f32,
) -> String {
    use std::fmt::Write as _;
    let mut out = format!("{CONDITIONING_VERSION} backend={}", backend.as_str());
    if backend == CaptureBackend::NotCaptured {
        return out;
    }
    let poles = match backend {
        CaptureBackend::Tee => "1pole",
        _ => "2pole",
    };
    if eq.is_empty() {
        if pipeline.dc_removal {
            let _ = write!(out, " dc=highpass:{DC_BLOCK_CUTOFF_HZ}Hz:{poles}");
        } else {
            out.push_str(" dc=off");
        }
        if pipeline.high_pass {
            let _ = write!(out, " hp=highpass:{HIGH_PASS_CUTOFF_HZ}Hz:{poles}");
        } else {
            out.push_str(" hp=off");
        }
    } else {
        let _ = write!(out, " eq=\"{}\"", eq.to_spec());
    }
    let gain = if gain_db.is_finite() { gain_db } else { 0.0 };
    let _ = write!(out, " gain={gain}dB");
    out.push_str(match (pipeline.agc, backend) {
        (false, _) => " agc=off",
        (true, CaptureBackend::Tee) => " agc=peak",
        (true, _) => " agc=dynaudnorm",
    });
    out
}

/// The first-order high-pass the microphone tee applies for each flag.
///
/// `y[n] = α(y[n−1] + x[n] − x[n−1])`, `α = RC / (RC + 1/fs)`: the same
/// arithmetic, restated here so the change it makes can be measured without
/// a capture process. `tests/the_capture_filters_change_what_the_model_hears.rs`
/// holds its response to the table in `AudioPipeline`'s documentation.
#[derive(Debug, Clone, Copy)]
pub struct OnePoleHighPass {
    alpha: f32,
    prev_x: f32,
    prev_y: f32,
}

impl OnePoleHighPass {
    /// A filter with its corner at `cutoff_hz` for audio at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: u32, cutoff_hz: f32) -> Self {
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        #[allow(clippy::cast_precision_loss)]
        let dt = 1.0 / sample_rate as f32;
        Self {
            alpha: rc / (rc + dt),
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    /// One sample through the filter.
    pub fn step(&mut self, x: f32) -> f32 {
        let y = self.alpha * (self.prev_y + x - self.prev_x);
        self.prev_x = x;
        self.prev_y = y;
        y
    }
}

/// How much a conditioning changed one window of samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputChange {
    /// Energy of the change relative to the input's, in dB:
    /// `10 log10(Σ(y−x)² / Σx²)`. `-inf` for no change.
    pub residual_db: f64,
    /// The output's energy relative to the input's, in dB.
    pub level_change_db: f64,
    /// The largest absolute change of one sample.
    pub max_abs_change: f32,
}

impl InputChange {
    /// Measure `after` against `before` (equal lengths; the shorter one bounds
    /// the comparison). `None` for an input with no energy.
    #[must_use]
    pub fn between(before: &[f32], after: &[f32]) -> Option<Self> {
        let mut ex = 0.0_f64;
        let mut ey = 0.0_f64;
        let mut ed = 0.0_f64;
        let mut max = 0.0_f32;
        for (x, y) in before.iter().zip(after) {
            ex += f64::from(*x) * f64::from(*x);
            ey += f64::from(*y) * f64::from(*y);
            let d = y - x;
            ed += f64::from(d) * f64::from(d);
            max = max.max(d.abs());
        }
        (ex > 0.0).then(|| Self {
            residual_db: 10.0 * (ed / ex).log10(),
            level_change_db: 10.0 * (ey / ex).log10(),
            max_abs_change: max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_microphone_says_both_of_its_filters() {
        let s = describe(
            CaptureBackend::Tee,
            AudioPipeline::default(),
            &EqChain::default(),
            0.0,
        );
        assert_eq!(
            s,
            "conditioning-1 backend=tee dc=highpass:5Hz:1pole hp=highpass:120Hz:1pole gain=0dB agc=off"
        );
    }

    #[test]
    fn a_camera_with_the_same_flags_says_its_filters_are_other_ones() {
        let s = describe(
            CaptureBackend::Ffmpeg,
            AudioPipeline::default(),
            &EqChain::default(),
            0.0,
        );
        assert!(s.contains("hp=highpass:120Hz:2pole"), "{s}");
        assert_ne!(
            s,
            describe(
                CaptureBackend::Tee,
                AudioPipeline::default(),
                &EqChain::default(),
                0.0
            )
        );
    }

    #[test]
    fn every_toggle_and_the_chain_change_the_string() {
        let base = AudioPipeline::default();
        let d = |p: AudioPipeline, eq: &EqChain, g: f32| describe(CaptureBackend::Tee, p, eq, g);
        let none = EqChain::default();
        let reference = d(base, &none, 0.0);
        let variants = [
            d(
                AudioPipeline {
                    high_pass: false,
                    ..base
                },
                &none,
                0.0,
            ),
            d(
                AudioPipeline {
                    dc_removal: false,
                    ..base
                },
                &none,
                0.0,
            ),
            d(AudioPipeline { agc: true, ..base }, &none, 0.0),
            d(base, &none, -3.0),
            d(
                base,
                &EqChain::parse("highpass:120; notch:50:20").unwrap(),
                0.0,
            ),
        ];
        for v in &variants {
            assert_ne!(v, &reference);
        }
        assert!(variants[0].contains("hp=off"));
        assert!(variants[1].contains("dc=off"));
        assert!(
            variants[4].contains("eq=\"highpass:120; notch:50:20\""),
            "{}",
            variants[4]
        );
    }

    #[test]
    fn a_dropped_file_is_said_to_be_unconditioned() {
        assert_eq!(
            describe(
                CaptureBackend::NotCaptured,
                AudioPipeline::default(),
                &EqChain::default(),
                6.0
            ),
            "conditioning-1 backend=not-captured"
        );
    }

    #[test]
    fn no_change_is_minus_infinity_and_a_halving_is_six_db() {
        let x = [0.5_f32, -0.25, 0.125];
        let same = InputChange::between(&x, &x).unwrap();
        assert!(same.residual_db.is_infinite() && same.residual_db < 0.0);
        let half: Vec<f32> = x.iter().map(|v| v / 2.0).collect();
        let c = InputChange::between(&x, &half).unwrap();
        assert!((c.level_change_db + 6.0206).abs() < 1e-3, "{c:?}");
        assert!(
            (c.residual_db + 6.0206).abs() < 1e-3,
            "the change is the other half"
        );
        assert_eq!(InputChange::between(&[0.0], &[1.0]), None);
    }
}
