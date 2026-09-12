//! Integrated loudness (ITU-R BS.1770) for the clips this station exports.
//!
//! # The problem
//!
//! Clips are written at whatever level the microphone delivered. A gallery of
//! them is unusable: the listener rides the volume control between every one,
//! and a quiet clip at the end of a playlist gets missed entirely. The standard
//! fix is to measure each file's loudness and apply one gain, which is what
//! this module measures and what [`Extractor`](super::Extractor) applies.
//!
//! # Where it is applied, and where it must never be
//!
//! At **clip write time only**. The analysis input — the samples the mel
//! spectrogram and the classifier see — is untouched, because a per-clip gain
//! would move every confidence score and make two stations' thresholds mean
//! different things. Living under `extraction` rather than beside
//! [`crate::audio::soundlevel`] is deliberate for that reason: this is an
//! export concern, and the module tree should say so.
//!
//! # What is implemented, and what is not
//!
//! **Implemented, per BS.1770-4:** the two-stage K-weighting filter, 400 ms
//! blocks at 75 % overlap, the `-0.691` offset, the absolute gate at −70 LUFS
//! and the relative gate at 10 LU below the absolutely-gated mean. Mono only —
//! every clip this station writes is mono — so the channel weights `G_i`, which
//! are `1.0` for the front channels anyway, do not appear.
//!
//! **Not implemented: an ITU true-peak meter.** BS.1770's true peak requires
//! 4× oversampling through a specified interpolation filter, and this module
//! does not do that: the ceiling below is a **sample peak**. That is stated
//! rather than glossed because the difference is real — inter-sample peaks in
//! ordinary material run up to about 1 dB above the sample peak — and the
//! default ceiling is chosen with that headroom in mind rather than pretending
//! the two are the same.

use crate::audio::biquad::{Biquad, BiquadError};

/// The offset in BS.1770's loudness equation.
///
/// Not a fudge factor: it is what makes a 1 kHz sine at −23 dBFS in both
/// channels of a stereo programme read −23.0 LUFS, by cancelling the
/// K-weighting's own gain at 1 kHz. `the_k_weighting_gain_at_1k_is_the_offset`
/// asserts that relationship against the filters actually built here, which is
/// the check that the coefficients below are right.
const OFFSET_DB: f64 = -0.691;

/// Absolute gate: blocks quieter than this contribute nothing.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;

/// Relative gate, in LU below the absolutely-gated mean.
const RELATIVE_GATE_LU: f64 = 10.0;

/// Block length, in seconds. BS.1770 §3.
const BLOCK_SECS: f64 = 0.4;

/// Block overlap. 75 % means a new block starts every 100 ms.
const BLOCK_OVERLAP: f64 = 0.75;

// ── K-weighting, from the analogue prototype ────────────────────────────────
//
// BS.1770 tabulates these coefficients at 48 kHz only, and a station may write
// clips at 44.1 kHz or 32 kHz. Building them from the prototype is what makes
// the measurement correct at any rate; `the_48k_coefficients_match_the_table`
// checks the construction against the standard's own table at the one rate it
// publishes.

/// Stage 1 — the high-shelf "pre-filter", modelling the head.
const SHELF_F0: f64 = 1_681.974_450_955_533;
/// Stage 1 shelf gain, in dB.
const SHELF_GAIN_DB: f64 = 3.999_843_853_973_347;
/// Stage 1 quality factor.
const SHELF_Q: f64 = 0.707_175_236_955_419_6;
/// The exponent relating the shelf's mid-band gain to its high-band gain.
const SHELF_VB_EXP: f64 = 0.499_666_774_154_541_6;

/// Stage 2 — the RLB high-pass.
const RLB_F0: f64 = 38.135_470_876_024_44;
/// Stage 2 quality factor.
const RLB_Q: f64 = 0.500_327_037_323_877_3;

/// The two-section K-weighting filter for `sample_rate`.
///
/// # Errors
///
/// [`BiquadError`] when the sample rate is low enough to put a section's
/// corner at or past Nyquist, or when the transcribed coefficients would not
/// be stable — neither of which any real capture rate reaches.
pub fn k_weighting(sample_rate: u32) -> Result<[Biquad; 2], BiquadError> {
    if sample_rate == 0 {
        return Err(BiquadError::NonPositiveParameter);
    }
    let fs = f64::from(sample_rate);

    // Stage 1: high shelf.
    let k = (std::f64::consts::PI * SHELF_F0 / fs).tan();
    let vh = 10.0_f64.powf(SHELF_GAIN_DB / 20.0);
    let vb = vh.powf(SHELF_VB_EXP);
    let kk = k * k;
    let a0 = 1.0 + k / SHELF_Q + kk;
    let shelf = Biquad::from_normalised(
        (vb * k / SHELF_Q + kk + vh) / a0,
        2.0 * (kk - vh) / a0,
        (vh - vb * k / SHELF_Q + kk) / a0,
        2.0 * (kk - 1.0) / a0,
        (1.0 - k / SHELF_Q + kk) / a0,
    )?;

    // Stage 2: RLB high pass. Its numerator is `1, -2, 1` un-normalised — the
    // form the standard gives — so the section carries a small passband gain
    // of `a0` rather than unity. That is part of the specified response, not an
    // oversight: the `-0.691` offset is calibrated against the whole chain.
    let k = (std::f64::consts::PI * RLB_F0 / fs).tan();
    let kk = k * k;
    let a0 = 1.0 + k / RLB_Q + kk;
    let rlb = Biquad::from_normalised(
        1.0,
        -2.0,
        1.0,
        2.0 * (kk - 1.0) / a0,
        (1.0 - k / RLB_Q + kk) / a0,
    )?;

    Ok([shelf, rlb])
}

/// The integrated loudness of `samples`, in LUFS.
///
/// `None` when there is nothing to measure: fewer samples than one 400 ms
/// block, or every block below the absolute gate (digital silence, or a clip
/// so quiet that BS.1770 declines to call it a level).
///
/// # Errors
///
/// Returns the [`BiquadError`] from [`k_weighting`] when the sample rate
/// cannot carry the filter.
pub fn integrated_lufs(samples: &[f32], sample_rate: u32) -> Result<Option<f64>, BiquadError> {
    let mut filters = k_weighting(sample_rate)?;
    let filtered: Vec<f64> = samples
        .iter()
        .map(|s| {
            let mut y = filters[0].process(*s);
            #[allow(clippy::cast_possible_truncation)]
            {
                y = filters[1].process(y as f32);
            }
            y
        })
        .collect();
    Ok(integrated_from_filtered(&filtered, sample_rate))
}

/// The gated mean of the block powers, as LUFS.
///
/// Split out from [`integrated_lufs`] so the gating can be exercised on
/// hand-built block powers, without a signal that happens to produce them.
fn integrated_from_filtered(filtered: &[f64], sample_rate: u32) -> Option<f64> {
    let block_len = block_len_samples(sample_rate);
    let step = block_step_samples(sample_rate);
    if block_len == 0 || step == 0 || filtered.len() < block_len {
        return None;
    }

    // Mean square per block.
    let mut powers: Vec<f64> = Vec::new();
    let mut start = 0;
    while start + block_len <= filtered.len() {
        let sum: f64 = filtered[start..start + block_len]
            .iter()
            .map(|y| y * y)
            .sum();
        #[allow(clippy::cast_precision_loss)]
        powers.push(sum / block_len as f64);
        start += step;
    }
    gated_lufs(&powers)
}

/// BS.1770's two-stage gate over per-block mean squares.
fn gated_lufs(powers: &[f64]) -> Option<f64> {
    let loud = |z: f64| {
        if z > 0.0 {
            10.0_f64.mul_add(z.log10(), OFFSET_DB)
        } else {
            f64::NEG_INFINITY
        }
    };

    // Absolute gate.
    let above: Vec<f64> = powers
        .iter()
        .copied()
        .filter(|z| loud(*z) > ABSOLUTE_GATE_LUFS)
        .collect();
    if above.is_empty() {
        return None;
    }

    // Relative gate, from the mean of what survived the absolute one.
    #[allow(clippy::cast_precision_loss)]
    let mean_above = above.iter().sum::<f64>() / above.len() as f64;
    let threshold = loud(mean_above) - RELATIVE_GATE_LU;
    let kept: Vec<f64> = above.into_iter().filter(|z| loud(*z) > threshold).collect();
    if kept.is_empty() {
        return None;
    }

    #[allow(clippy::cast_precision_loss)]
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    Some(loud(mean))
}

/// Samples in one 400 ms block.
fn block_len_samples(sample_rate: u32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (f64::from(sample_rate) * BLOCK_SECS).round() as usize
    }
}

/// Samples between the starts of consecutive blocks (75 % overlap → 100 ms).
fn block_step_samples(sample_rate: u32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (f64::from(sample_rate) * BLOCK_SECS * (1.0 - BLOCK_OVERLAP)).round() as usize
    }
}

/// The largest absolute sample value in `samples`.
///
/// A **sample** peak, not an ITU true peak — see the module header.
#[must_use]
pub fn sample_peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// What one clip's normalisation would do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normalisation {
    /// The measured integrated loudness, in LUFS.
    pub measured_lufs: f64,
    /// The linear gain to apply.
    pub gain: f32,
    /// Whether the ceiling, rather than the target, decided the gain.
    ///
    /// True means the clip could not reach `target_lufs` without a sample
    /// passing the ceiling, so it was turned up as far as the ceiling allowed
    /// and no further. Worth recording: a clip that is *quiet and peaky* — one
    /// loud wing-beat over a distant song — cannot be brought to target by a
    /// single gain, and a caller that assumed otherwise would be wrong about
    /// what is on disk.
    pub peak_limited: bool,
}

/// Plan the gain for one clip.
///
/// `target_lufs` is where the clip should land (EBU R128 uses −23; a station
/// listening on a phone usually wants louder). `ceiling_dbfs` is the sample
/// peak no sample may pass after the gain, and must be negative.
///
/// `None` when there is nothing to normalise: [`integrated_lufs`] found no
/// measurable level, or the clip is digital silence, in which case any gain is
/// still silence.
///
/// # Errors
///
/// Returns the [`BiquadError`] from [`k_weighting`].
pub fn plan(
    samples: &[f32],
    sample_rate: u32,
    target_lufs: f64,
    ceiling_dbfs: f64,
) -> Result<Option<Normalisation>, BiquadError> {
    let Some(measured_lufs) = integrated_lufs(samples, sample_rate)? else {
        return Ok(None);
    };
    let peak = sample_peak(samples);
    if peak <= 0.0 {
        return Ok(None);
    }

    let wanted_db = target_lufs - measured_lufs;
    let wanted = 10.0_f64.powf(wanted_db / 20.0);
    let ceiling = 10.0_f64.powf(ceiling_dbfs / 20.0);
    let max_gain = ceiling / f64::from(peak);
    let peak_limited = wanted > max_gain;
    let gain = if peak_limited { max_gain } else { wanted };

    #[allow(clippy::cast_possible_truncation)]
    Ok(Some(Normalisation {
        measured_lufs,
        gain: gain as f32,
        peak_limited,
    }))
}

/// Apply `gain` in place, clamping to the 16-bit range the WAV writer uses.
///
/// The clamp is a backstop, not the mechanism: [`plan`] already sized the gain
/// so no sample reaches the ceiling. It exists because a sample that came in
/// above full scale — which a decoder can produce — would otherwise wrap when
/// the writer casts it.
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{Normalisation, apply_gain, integrated_lufs, k_weighting, plan, sample_peak};

    const SR: u32 = 48_000;

    /// `secs` of a sine at `hz`, amplitude `amp`.
    fn sine(hz: f64, amp: f64, secs: f64, sample_rate: u32) -> Vec<f32> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (f64::from(sample_rate) * secs) as usize;
        (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f64 / f64::from(sample_rate);
                #[allow(clippy::cast_possible_truncation)]
                {
                    (amp * (2.0 * std::f64::consts::PI * hz * t).sin()) as f32
                }
            })
            .collect()
    }

    /// dBFS → linear amplitude.
    fn amp_of(dbfs: f64) -> f64 {
        10.0_f64.powf(dbfs / 20.0)
    }

    /// The K-weighting chain's magnitude at `hz`, in dB.
    fn k_gain_db(hz: f32, sample_rate: u32) -> f64 {
        let f = k_weighting(sample_rate).expect("filter");
        let mag = f[0].magnitude_at(hz, sample_rate) * f[1].magnitude_at(hz, sample_rate);
        20.0 * mag.log10()
    }

    /// The relationship the whole measurement rests on.
    ///
    /// BS.1770's `-0.691` offset is not a fudge: it exists to cancel the
    /// K-weighting's own gain at 1 kHz, which is what makes a 1 kHz sine at
    /// −23 dBFS in two channels read −23.0 LUFS. So the chain built here must
    /// have a gain of **+0.691 dB** at 1 kHz — and if the coefficients above
    /// were transcribed wrongly, this is where it shows, without needing a
    /// reference implementation to compare against.
    #[test]
    fn the_k_weighting_gain_at_1k_is_the_offset() {
        let measured = k_gain_db(1_000.0, SR);
        assert!(
            (measured - 0.691).abs() < 0.02,
            "K-weighting gain at 1 kHz is {measured:.4} dB; BS.1770's offset says it \
             must be +0.691 dB, so the coefficients are wrong"
        );
    }

    /// The same relationship at every rate a station can write at.
    ///
    /// This is the reason the filter is built from the prototype rather than
    /// transcribed from the standard's 48 kHz table: a station configured for
    /// 44.1 kHz would otherwise be measured through a filter designed for a
    /// different sample rate.
    ///
    /// The tolerance is wider than the 48 kHz one because the bilinear
    /// transform warps: the prototype's corner lands at a slightly different
    /// place at each rate, so the 1 kHz gain drifts. Measured across
    /// 8 k–192 kHz it runs from 0.6707 dB (192 kHz) to 0.7377 dB (16 kHz) —
    /// a spread of 0.067 dB, or 0.7 % of a decibel, which is inaudible and is
    /// a property of the construction rather than an error in it. The two rates
    /// a station actually captures at are checked tightly below.
    #[test]
    fn the_offset_relationship_holds_at_every_capture_rate() {
        for rate in [
            8_000, 16_000, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000,
        ] {
            let measured = k_gain_db(1_000.0, rate);
            assert!(
                (measured - 0.691).abs() < 0.05,
                "at {rate} Hz the K-weighting gain at 1 kHz is {measured:.4} dB"
            );
        }

        // The rates this station records at, held to the tight tolerance: the
        // wide one above must not be able to hide a real error at 44.1 or 48.
        for rate in [44_100, 48_000] {
            let measured = k_gain_db(1_000.0, rate);
            assert!(
                (measured - 0.691).abs() < 0.01,
                "at {rate} Hz — a rate this station really uses — the gain is {measured:.4} dB"
            );
        }
    }

    /// The 48 kHz coefficients, pinned.
    ///
    /// **What this is:** a regression pin, not an independent verification. The
    /// values are what this construction produces, and they reproduce the
    /// coefficients ITU-R BS.1770 tabulates for 48 kHz to the fourteen
    /// significant figures that table is usually printed with. The
    /// construction itself was checked line by line against libebur128's
    /// `ebur128_init_filter` — the implementation ffmpeg and most loudness
    /// tooling use — including the prototype constants, the `Vb` exponent, and
    /// the RLB stage's un-normalised `1, -2, 1` numerator.
    ///
    /// **It is also the more sensitive of the two, which was not the
    /// expectation.** Measured: replacing `SHELF_Q` with `1/√2`
    /// (`0.7071067811865475`) — the plausible-looking value anyone would write
    /// from habit, one part in ten thousand from the specified
    /// `0.7071752369554196` — moves the 1 kHz gain by less than the
    /// relationship test's tolerance and passes it. Only this pin catches it.
    /// So the two are complements: the relationship proves the *form* is right
    /// without needing any reference numbers, and the pin catches a constant
    /// typed wrongly.
    ///
    /// The first version of this test carried mis-transcribed values from
    /// memory and failed against a correct implementation. That is why the
    /// values here are the construction's own output, cross-checked against
    /// libebur128, rather than a table typed from recall.
    #[test]
    fn the_48k_coefficients_are_pinned() {
        let f = k_weighting(48_000).expect("filter");
        let (shelf, rlb) = (f[0].coefficients(), f[1].coefficients());

        let close = |got: f64, want: f64, what: &str| {
            assert!(
                (got - want).abs() < 1e-12,
                "{what}: got {got:.16}, pinned at {want:.16}"
            );
        };
        close(shelf.0, 1.535_124_859_586_970_2, "shelf b0");
        close(shelf.1, -2.691_696_189_406_380_7, "shelf b1");
        close(shelf.2, 1.198_392_810_852_85, "shelf b2");
        close(shelf.3, -1.690_659_293_182_410_3, "shelf a1");
        close(shelf.4, 0.732_480_774_215_850_1, "shelf a2");

        close(rlb.0, 1.0, "rlb b0");
        close(rlb.1, -2.0, "rlb b1");
        close(rlb.2, 1.0, "rlb b2");
        close(rlb.3, -1.990_047_454_833_979_7, "rlb a1");
        close(rlb.4, 0.990_072_250_366_209_9, "rlb a2");
    }

    /// End to end, against a signal whose loudness is known in closed form.
    ///
    /// For a *mono* sine at 1 kHz and amplitude `A`, the K-weighting gain
    /// (+0.691 dB) exactly cancels the offset, so
    /// `L = 20·log10(A) − 10·log10(2)` — the `10·log10(2)` being the mean
    /// square of a sine. At −23 dBFS that is **−26.01 LUFS**, three decibels
    /// below the stereo figure the standard quotes, because one channel is
    /// summed instead of two.
    #[test]
    fn a_mono_1k_sine_reads_its_closed_form_loudness() {
        for dbfs in [-23.0, -12.0, -40.0] {
            let s = sine(1_000.0, amp_of(dbfs), 3.0, SR);
            let measured = integrated_lufs(&s, SR)
                .expect("filter")
                .expect("a sine has a level");
            let expected = 10.0_f64.mul_add(-2.0_f64.log10(), dbfs);
            assert!(
                (measured - expected).abs() < 0.1,
                "a {dbfs} dBFS mono sine measured {measured:.3} LUFS, closed form says \
                 {expected:.3}"
            );
        }
    }

    /// Halving the amplitude lowers the measurement by 6.02 dB, at every level.
    ///
    /// The property that makes a single gain the right correction at all: the
    /// measurement is a level, so it must be linear in dB.
    #[test]
    fn the_measurement_moves_one_for_one_with_the_signal() {
        let base = integrated_lufs(&sine(1_000.0, amp_of(-20.0), 3.0, SR), SR)
            .expect("filter")
            .expect("level");
        let half = integrated_lufs(&sine(1_000.0, amp_of(-26.02), 3.0, SR), SR)
            .expect("filter")
            .expect("level");
        assert!(
            ((base - half) - 6.02).abs() < 0.05,
            "halving the amplitude moved the measurement by {:.3} dB, not 6.02",
            base - half
        );
    }

    /// Silence has no loudness, rather than a very negative one.
    #[test]
    fn digital_silence_has_no_measurable_loudness() {
        let s = vec![0.0_f32; SR as usize * 2];
        assert_eq!(integrated_lufs(&s, SR).expect("filter"), None);
    }

    /// A clip shorter than one 400 ms block has nothing to measure.
    #[test]
    fn a_clip_shorter_than_a_block_has_no_loudness() {
        let s = sine(1_000.0, amp_of(-20.0), 0.3, SR);
        assert_eq!(integrated_lufs(&s, SR).expect("filter"), None);
    }

    /// The relative gate is doing something: a long quiet tail must not drag
    /// the measurement down the way a plain average would.
    ///
    /// One second at −20 dBFS followed by four seconds at −50 dBFS. A plain
    /// mean of the block powers is dominated by neither — it is
    /// `(1·10^-2 + 4·10^-5)/5`, which reads about 7 dB below the loud part —
    /// while BS.1770's relative gate drops every block more than 10 LU below
    /// the mean, leaving the loud second to decide.
    #[test]
    fn the_relative_gate_keeps_a_quiet_tail_from_deciding_the_level() {
        let mut s = sine(1_000.0, amp_of(-20.0), 1.0, SR);
        s.extend(sine(1_000.0, amp_of(-50.0), 4.0, SR));
        let gated = integrated_lufs(&s, SR).expect("filter").expect("level");

        let loud_only = integrated_lufs(&sine(1_000.0, amp_of(-20.0), 1.0, SR), SR)
            .expect("filter")
            .expect("level");
        assert!(
            (gated - loud_only).abs() < 1.0,
            "with the gate the mixed clip reads {gated:.2} LUFS and the loud second alone \
             reads {loud_only:.2}; they should agree to about a decibel"
        );

        // The counterpart: an ungated mean really would be several decibels
        // lower, so the assertion above is about the gate and not about the
        // quiet tail being inaudible to the meter anyway.
        let plain_mean_lufs = {
            let loud_z = 10.0_f64.powi(-2) / 2.0;
            let quiet_z = 10.0_f64.powi(-5) / 2.0;
            let mean = 4.0_f64.mul_add(quiet_z, loud_z) / 5.0;
            10.0 * mean.log10()
        };
        assert!(
            loud_only - plain_mean_lufs > 4.0,
            "the ungated mean is {plain_mean_lufs:.2} against {loud_only:.2}; if those \
             were close, the gate would have nothing to prove"
        );
    }

    /// The gain is the difference between where the clip is and where it
    /// should be — when the peak leaves room for it.
    #[test]
    fn the_gain_is_the_distance_to_the_target() {
        // A −40 dBFS sine: quiet, and with plenty of peak headroom.
        let s = sine(1_000.0, amp_of(-40.0), 3.0, SR);
        let n = plan(&s, SR, -23.0, -1.0)
            .expect("filter")
            .expect("a sine can be normalised");
        assert!(!n.peak_limited, "{n:?}");

        let applied_db = 20.0 * f64::from(n.gain).log10();
        assert!(
            (applied_db - (-23.0 - n.measured_lufs)).abs() < 1e-6,
            "gain {applied_db:.4} dB does not close the gap from {:.4} to -23",
            n.measured_lufs
        );

        // And applying it lands the clip on target.
        let mut out = s;
        apply_gain(&mut out, n.gain);
        let after = integrated_lufs(&out, SR).expect("filter").expect("level");
        assert!(
            (after - -23.0).abs() < 0.1,
            "after normalising, the clip reads {after:.3} LUFS, not -23"
        );
    }

    /// The ceiling wins over the target. A quiet, peaky clip — one loud
    /// wing-beat over a distant song — cannot be brought to target by a single
    /// gain, and turning it up until a sample clips would be worse than leaving
    /// it quiet.
    #[test]
    fn the_ceiling_stops_the_gain_and_says_so() {
        // A quiet sine with one full-scale spike in it.
        let mut s = sine(1_000.0, amp_of(-40.0), 3.0, SR);
        s[1000] = 0.9;

        let n = plan(&s, SR, -23.0, -1.0)
            .expect("filter")
            .expect("normalisable");
        assert!(n.peak_limited, "the spike must limit the gain: {n:?}");

        let mut out = s;
        apply_gain(&mut out, n.gain);
        let ceiling = 10.0_f32.powf(-1.0 / 20.0);
        let peak = sample_peak(&out);
        assert!(
            peak <= ceiling + 1e-6,
            "peak {peak:.6} passed the -1 dBFS ceiling {ceiling:.6}"
        );
        assert!(
            peak > ceiling - 1e-3,
            "and it should be turned up as far as the ceiling allows, not less: {peak:.6}"
        );
    }

    /// Silence is not normalisable — any gain applied to it is still silence,
    /// and a gain computed from it would be infinite.
    #[test]
    fn silence_yields_no_plan() {
        let s = vec![0.0_f32; SR as usize * 2];
        assert_eq!(plan(&s, SR, -23.0, -1.0).expect("filter"), None);
    }

    /// Measuring an already-normalised clip returns the target, so a second
    /// pass is a no-op. This is what makes recording the measured value in the
    /// clip's metadata worth anything.
    #[test]
    fn normalising_twice_changes_nothing_the_second_time() {
        let s = sine(1_000.0, amp_of(-35.0), 3.0, SR);
        let first = plan(&s, SR, -23.0, -1.0).expect("filter").expect("plan");
        let mut once = s;
        apply_gain(&mut once, first.gain);

        let second = plan(&once, SR, -23.0, -1.0).expect("filter").expect("plan");
        let second_db = 20.0 * f64::from(second.gain).log10();
        assert!(
            second_db.abs() < 0.1,
            "a second pass wanted another {second_db:.4} dB; the first pass did not land"
        );
    }

    /// `Normalisation` is compared by value in the tests above; keep it so.
    #[test]
    fn a_plan_is_comparable() {
        let a = Normalisation {
            measured_lufs: -30.0,
            gain: 2.0,
            peak_limited: false,
        };
        assert_eq!(a, a);
    }
}
