//! Categorically broken audio: silence, saturation, a wedged converter.
//!
//! # The gap this fills
//!
//! A capture source that dies is caught by the supervisor's restart policy. One
//! that is alive but writing nothing is caught by its stall detector. One that
//! is merely going *deaf* is what [`super::super::super::audio::quality`]'s
//! noise-floor sampling is for.
//!
//! None of them catches a source that writes a segment of exactly the right
//! length, at exactly the right time, containing nothing. A muted mixer
//! channel, a line-in with nothing plugged into it, an RTSP camera whose audio
//! track was disabled in its web UI — all produce a valid, punctual stream of
//! zeros. The supervisor sees fresh segments and reports `Connected`, the
//! gauge reads 1, and the station records nothing for as long as it takes
//! somebody to notice.
//!
//! # Why these three can be alerted on when a noise floor cannot
//!
//! `acoustic_health` deliberately does not alert: a noise floor moves for real
//! reasons — weather, season, leaf-out, a lawnmower — and a threshold picked
//! without a season of recordings to calibrate against would fire on all of
//! them and teach the operator to ignore the channel.
//!
//! These faults are different in kind, not degree. No microphone in any weather
//! produces digitally exact zeros, sits at full scale for a fifth of a segment,
//! or returns the same sample value for fifteen seconds. There is nothing to
//! calibrate: each one is a signal that cannot come from a working input, so a
//! fixed rule is honest here in a way a noise-floor threshold would not be.

/// A way a capture source can be alive, punctual and useless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFault {
    /// Every sample is zero, or within a hair of it.
    ///
    /// A muted channel, an unplugged input, or an RTSP source whose audio
    /// track is disabled. A working microphone always carries thermal noise;
    /// exact zeros are a digital path with nothing behind it.
    DigitallySilent,
    /// The signal is pinned at one non-zero value.
    ///
    /// A wedged analogue-to-digital converter or a preamp latched to a rail.
    /// It is not silence — the level may be large — but it carries no
    /// information at all.
    StuckLevel,
    /// A large fraction of samples sit at full scale.
    ///
    /// Gain set far too high, or a failing preamp. The waveform is a square
    /// wave, every spectrogram is broadband, and the classifier produces
    /// confident nonsense from it.
    Saturated,
}

impl StreamFault {
    /// A short description for a log line or the health page.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DigitallySilent => "digital silence — the input is muted or unplugged",
            Self::StuckLevel => "a stuck level — the converter or preamp is wedged",
            Self::Saturated => "saturation — the gain is far too high",
        }
    }
}

/// Below this absolute amplitude a sample counts as digitally zero.
///
/// Half the least-significant bit of 16-bit PCM (`1/32768 ≈ 3.05e-5`). Chosen
/// against the *format* rather than as a taste threshold: anything a 16-bit
/// capture path can represent as non-zero is above it, so this cannot mistake a
/// very quiet night for an unplugged cable.
pub const SILENCE_EPS: f32 = 1.5e-5;

/// Peak-to-trough range below which the signal counts as stuck.
///
/// Deliberately the same magnitude as [`SILENCE_EPS`]: "varies by less than one
/// 16-bit step across fifteen seconds" is not a quiet input, it is an input
/// that is not being read.
pub const FLAT_EPS: f32 = 1.5e-5;

/// Absolute amplitude at or above which a sample counts as clipped.
///
/// Not exactly 1.0: a resampler and a float conversion both put a sample that
/// was at full scale slightly under it, so an exact test would miss most real
/// clipping.
pub const CLIP_LEVEL: f32 = 0.999;

/// Fraction of clipped samples at which a segment counts as saturated.
///
/// A fifth. Real audio clips transiently — a wing-flap on the housing, a car
/// door — and a low threshold would fire on those. A working input does not
/// spend a fifth of fifteen seconds at full scale.
pub const CLIP_FRACTION: f32 = 0.20;

/// Fewest samples before a verdict is offered.
///
/// A short read at the end of a file has no shape to judge, and calling it
/// broken would fire on the ordinary case of a segment still being written.
pub const MIN_SAMPLES: usize = 1_000;

/// Judge one segment's samples.
///
/// `None` means nothing categorically wrong was found — which is not a claim
/// that the audio is *good*. Quality lives in
/// [`super::assess_quality`]; this answers only the narrower question of
/// whether the input is connected at all.
///
/// Order matters: digital silence is also flat, so it is tested first and the
/// more specific verdict wins. Reporting a muted channel as "stuck level"
/// would send the operator to look at the wrong end of the cable.
#[must_use]
pub fn assess_stream(samples: &[f32]) -> Option<StreamFault> {
    if samples.len() < MIN_SAMPLES {
        return None;
    }

    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut max_abs = 0.0_f32;
    let mut clipped = 0_usize;
    for &s in samples {
        // NaN would poison every comparison below and make the verdict depend
        // on sample order. A decoder that emits one is broken in its own right;
        // treating the segment as unjudgeable is the honest answer.
        if !s.is_finite() {
            return None;
        }
        min = min.min(s);
        max = max.max(s);
        max_abs = max_abs.max(s.abs());
        if s.abs() >= CLIP_LEVEL {
            clipped += 1;
        }
    }

    if max_abs <= SILENCE_EPS {
        return Some(StreamFault::DigitallySilent);
    }
    if max - min <= FLAT_EPS {
        return Some(StreamFault::StuckLevel);
    }
    // `as f32` on counts bounded by a segment's sample count: a 15-second
    // segment at 48 kHz is 720 000, far inside f32's exact-integer range.
    #[allow(clippy::cast_precision_loss)]
    let fraction = clipped as f32 / samples.len() as f32;
    if fraction >= CLIP_FRACTION {
        return Some(StreamFault::Saturated);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        CLIP_FRACTION, CLIP_LEVEL, FLAT_EPS, MIN_SAMPLES, SILENCE_EPS, StreamFault, assess_stream,
    };

    /// A plausible quiet-but-working input: low-level noise, nothing clipped.
    fn working(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                // Deterministic pseudo-noise around zero, peaking well under
                // full scale — the shape of a real garden at night.
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32;
                0.01_f32.mul_add((t * 1.13).sin(), 0.02 * (t * 0.37).sin())
            })
            .collect()
    }

    // ── nothing wrong ───────────────────────────────────────────────────

    #[test]
    fn a_working_input_reports_no_fault() {
        // The gate that matters most: a false positive here tells an operator
        // their microphone is unplugged when it is not, and the next real
        // alert is the one they ignore.
        assert_eq!(assess_stream(&working(48_000)), None);
    }

    #[test]
    fn a_very_quiet_night_is_not_mistaken_for_an_unplugged_cable() {
        // The distinction the whole module rests on. This signal is 60 dB below
        // the "working" fixture — quieter than any real garden — and must still
        // read as connected, because it carries information.
        let quiet: Vec<f32> = working(48_000).iter().map(|s| s * 0.001).collect();
        let peak = quiet.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            peak > SILENCE_EPS,
            "test setup: {peak} is below the epsilon"
        );
        assert_eq!(assess_stream(&quiet), None);
    }

    #[test]
    fn a_transient_clip_is_not_saturation() {
        // Counterpart to the saturation gate: real audio clips briefly — a
        // wing-flap on the housing, a car door — and firing on that would make
        // the alert worthless.
        let mut s = working(48_000);
        for v in s.iter_mut().take(4_000) {
            *v = 1.0;
        }
        let fraction = 4_000.0 / 48_000.0;
        assert!(
            fraction < CLIP_FRACTION,
            "test setup: {fraction} is not transient"
        );
        assert_eq!(assess_stream(&s), None);
    }

    // ── digital silence ─────────────────────────────────────────────────

    #[test]
    fn all_zeros_is_digital_silence() {
        assert_eq!(
            assess_stream(&vec![0.0_f32; 48_000]),
            Some(StreamFault::DigitallySilent)
        );
    }

    #[test]
    fn dither_below_one_sixteen_bit_step_is_still_silence() {
        // A muted digital path is not always bit-exact zero; a resampler can
        // leave sub-LSB dust. The epsilon is set against the *format* so this
        // still reads as silence.
        let dust: Vec<f32> = (0..48_000)
            .map(|i| {
                if i % 2 == 0 {
                    SILENCE_EPS
                } else {
                    -SILENCE_EPS
                }
            })
            .collect();
        assert_eq!(assess_stream(&dust), Some(StreamFault::DigitallySilent));
    }

    #[test]
    fn a_signal_one_step_above_the_epsilon_is_not_silence() {
        // Counterpart, pinning that the epsilon is a boundary and not a mute
        // button: anything a 16-bit path can represent gets through.
        let s: Vec<f32> = (0..48_000)
            .map(|i| {
                if i % 2 == 0 {
                    1.0 / 32_768.0
                } else {
                    -1.0 / 32_768.0
                }
            })
            .collect();
        assert_eq!(assess_stream(&s), None);
    }

    // ── stuck level ─────────────────────────────────────────────────────

    #[test]
    fn a_constant_non_zero_level_is_stuck_not_silent() {
        // A wedged converter latched to a rail. Reporting this as silence
        // would send the operator to check the cable rather than the input
        // stage — the level is large, it just never changes.
        assert_eq!(
            assess_stream(&vec![0.4_f32; 48_000]),
            Some(StreamFault::StuckLevel)
        );
        assert_eq!(
            assess_stream(&vec![-0.4_f32; 48_000]),
            Some(StreamFault::StuckLevel)
        );
    }

    #[test]
    fn silence_wins_over_stuck_because_it_is_the_more_specific_answer() {
        // All-zeros is also perfectly flat, so both rules match. The order in
        // `assess_stream` decides, and the specific verdict is the one that
        // names the right end of the cable.
        assert_eq!(
            assess_stream(&vec![0.0_f32; 48_000]),
            Some(StreamFault::DigitallySilent),
            "a muted input was reported as a wedged converter"
        );
    }

    #[test]
    fn a_signal_that_varies_by_more_than_a_step_is_not_stuck() {
        let s: Vec<f32> = (0..48_000)
            .map(|i| {
                if i % 2 == 0 {
                    0.4
                } else {
                    FLAT_EPS.mul_add(4.0, 0.4)
                }
            })
            .collect();
        assert_eq!(assess_stream(&s), None);
    }

    // ── saturation ──────────────────────────────────────────────────────

    #[test]
    fn a_segment_pinned_at_full_scale_is_saturated() {
        // Alternating rails: not flat, not silent, and useless. This is what a
        // gain-blown input actually looks like.
        let s: Vec<f32> = (0..48_000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert_eq!(assess_stream(&s), Some(StreamFault::Saturated));
    }

    #[test]
    fn saturation_counts_both_rails() {
        // Clipping is symmetric and the count is on absolute value; counting
        // only the positive rail would halve every measurement and miss the
        // ordinary symmetric case above.
        let mut s = working(48_000);
        for v in s.iter_mut().take(12_000) {
            *v = -1.0;
        }
        assert_eq!(assess_stream(&s), Some(StreamFault::Saturated));
    }

    #[test]
    fn a_sample_just_under_the_clip_level_does_not_count_as_clipped() {
        // `CLIP_LEVEL` is under 1.0 because a resampler leaves a full-scale
        // sample slightly below it. This pins that it is still a threshold: a
        // loud-but-clean signal is not saturation.
        let s: Vec<f32> = (0..48_000)
            .map(|i| {
                if i % 2 == 0 {
                    CLIP_LEVEL - 0.01
                } else {
                    -CLIP_LEVEL + 0.01
                }
            })
            .collect();
        assert_eq!(assess_stream(&s), None);
    }

    // ── refusing to judge ───────────────────────────────────────────────

    #[test]
    fn too_few_samples_produces_no_verdict() {
        // A short read at the end of a file, or a segment still being written.
        // Calling that broken would fire on the ordinary case.
        assert_eq!(assess_stream(&vec![0.0_f32; MIN_SAMPLES - 1]), None);
        assert_eq!(
            assess_stream(&vec![0.0_f32; MIN_SAMPLES]),
            Some(StreamFault::DigitallySilent)
        );
    }

    #[test]
    fn an_empty_segment_produces_no_verdict() {
        assert_eq!(assess_stream(&[]), None);
    }

    #[test]
    fn a_segment_of_nan_is_unjudgeable_rather_than_silent() {
        // Without the guard this is the worst possible answer. `f32::min` and
        // `f32::max` quietly return the non-NaN operand, so an all-NaN segment
        // leaves `max_abs` at its initial 0.0 and reads as *digital silence* —
        // sending the operator out to check a cable when the decoder is the
        // thing that is broken.
        assert_eq!(assess_stream(&vec![f32::NAN; 48_000]), None);
    }

    #[test]
    fn a_segment_of_infinities_is_unjudgeable_rather_than_saturated() {
        // The other half: unguarded, every sample counts as clipped and the
        // verdict is "turn your gain down", which is not the problem.
        assert_eq!(assess_stream(&vec![f32::INFINITY; 48_000]), None);
        assert_eq!(assess_stream(&vec![f32::NEG_INFINITY; 48_000]), None);
    }

    #[test]
    fn one_bad_sample_does_not_change_a_verdict_it_should_not() {
        // Counterpart: the guard is `return None` on *any* non-finite sample,
        // which is deliberately strict. This pins that the strictness is the
        // intent — a single NaN in an otherwise working segment makes the
        // whole segment unjudgeable rather than being silently skipped, so a
        // decoder fault can never masquerade as a clean reading.
        let mut s = working(48_000);
        assert_eq!(assess_stream(&s), None, "test setup: the fixture is clean");
        s[100] = f32::NAN;
        assert_eq!(assess_stream(&s), None);

        let mut silent = vec![0.0_f32; 48_000];
        assert_eq!(
            assess_stream(&silent),
            Some(StreamFault::DigitallySilent),
            "test setup"
        );
        silent[100] = f32::NAN;
        assert_eq!(
            assess_stream(&silent),
            None,
            "a NaN in a silent segment still produced a verdict"
        );
    }
}
