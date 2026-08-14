//! `--channel-report`: what a stereo microphone is actually delivering.
//!
//! Answers, on the operator's own hardware and in their own acoustics, the one
//! question no amount of code reading settles: **is this station losing signal
//! to its stereo microphone?**
//!
//! The BirdNET model has a single audio input (`[1, N]`), so two channels must
//! become one before inference no matter what. Today that reduction is a plain
//! average — `sum / num_channels` in `birdnet_core::audio::decode`. For two
//! capsules sitting in the same spot that is harmless. For two capsules a few
//! centimetres apart it is a comb filter: the same wavefront reaches them at
//! different times, and averaging then cancels every frequency where the two
//! disagree in phase. Measured through this project's decode path with a half
//! period of delay, that costs about 66 dB — the signal essentially vanishes —
//! while a quarter period costs 3 dB and a full period costs nothing. Which of
//! those a given station sits in depends on capsule spacing and on where the
//! bird is, so it cannot be reasoned about from here.
//!
//! This records a few seconds from the configured device and reports what each
//! reduction would hand the model, so the choice between Mono / Left / Right /
//! Stereo can be made from numbers rather than from a datasheet.

use std::io::Read;
use std::process::{Command, Stdio};

use crate::cli::Cli;

/// Speed of sound in air at about 15 °C, for turning an inter-channel delay
/// into the path difference that produced it.
const SPEED_OF_SOUND_M_PER_S: f32 = 340.0;

/// Widest inter-channel delay considered, in milliseconds.
///
/// 2 ms is about 68 cm of path difference — far wider than any stereo capsule
/// pair, so a best fit at the edge of this range means the two channels are not
/// a delayed copy of one another at all (independent noise, or one dead
/// channel) rather than a very wide array.
const MAX_LAG_MS: f32 = 2.0;

/// What each way of reducing two channels to one would deliver.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelAnalysis {
    /// RMS of the left channel alone.
    pub left_rms: f32,
    /// RMS of the right channel alone.
    pub right_rms: f32,
    /// RMS of the plain average — what the decoder does today.
    pub average_rms: f32,
    /// RMS of the delay-aligned sum: shift one channel by `best_lag` so the
    /// wavefronts coincide, then add. Coherent signal doubles; noise that is
    /// independent between the capsules does not.
    pub aligned_sum_rms: f32,
    /// Inter-channel delay, in samples, that best aligns the two channels.
    /// Positive means the right capsule heard the sound later than the left.
    pub best_lag_samples: i32,
    /// Normalised correlation at `best_lag_samples`, in `[-1, 1]`.
    pub best_correlation: f32,
    /// Normalised correlation with no shift — what the plain average is
    /// effectively working with.
    pub zero_lag_correlation: f32,
    /// Sample rate the analysis ran at.
    pub sample_rate: u32,
}

impl ChannelAnalysis {
    /// The louder single channel's RMS — the baseline any reduction should beat.
    #[must_use]
    pub const fn best_single_rms(&self) -> f32 {
        self.left_rms.max(self.right_rms)
    }

    /// Which single channel is louder.
    #[must_use]
    pub const fn louder_channel(&self) -> &'static str {
        if self.left_rms >= self.right_rms {
            "left"
        } else {
            "right"
        }
    }

    /// Delay between the capsules in milliseconds.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "sample rates and capsule lags are far inside f32's exact range"
    )]
    pub fn best_lag_ms(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        f32::from(i16::try_from(self.best_lag_samples).unwrap_or(0)) * 1000.0
            / self.sample_rate as f32
    }

    /// Path-length difference implied by the delay, in centimetres.
    #[must_use]
    pub fn path_difference_cm(&self) -> f32 {
        (self.best_lag_ms() / 1000.0 * SPEED_OF_SOUND_M_PER_S * 100.0).abs()
    }

    /// Whether the capsules look spaced rather than coincident.
    ///
    /// Two conditions, because either alone misleads. The channels must
    /// genuinely be a delayed copy of one another (`best_correlation` high —
    /// otherwise they are unrelated noise and no alignment is meaningful), and
    /// aligning them must actually improve on not aligning them, which is what
    /// says the delay is real rather than a spurious best fit on a coincident
    /// pair.
    #[must_use]
    pub fn capsules_look_spaced(&self) -> bool {
        self.best_correlation > 0.5
            && self.best_lag_samples != 0
            && self.best_correlation - self.zero_lag_correlation > 0.05
    }

    /// How much the plain average loses against the louder single channel, in
    /// dB. Negative means the average is quieter — signal thrown away.
    #[must_use]
    pub fn average_vs_best_single_db(&self) -> f32 {
        ratio_db(self.average_rms, self.best_single_rms())
    }

    /// How much a delay-aligned sum would gain over the plain average, in dB.
    #[must_use]
    pub fn aligned_vs_average_db(&self) -> f32 {
        ratio_db(self.aligned_sum_rms, self.average_rms)
    }
}

/// `20 log10(a / b)`, with zero and non-finite inputs collapsing to 0 dB rather
/// than to an infinity that would print as `-inf` in a report.
fn ratio_db(a: f32, b: f32) -> f32 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * (a / b).log10();
    if db.is_finite() { db } else { 0.0 }
}

/// Root mean square of a signal.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "a few seconds of audio is far inside f32's exact integer range"
)]
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    (samples.iter().map(|v| v * v).sum::<f32>() / n).sqrt()
}

/// Normalised cross-correlation of `a` and `b` with `b` shifted by `lag`.
///
/// Normalised by both signals' energy over the overlapping region, so the
/// result is comparable across lags and is not simply maximised by whichever
/// shift happens to overlap the loudest passage.
fn correlation_at(first: &[f32], second: &[f32], lag: i32) -> f32 {
    // `lag` is how far `b` trails `a`: at lag +n, `b[i + n]` is the same
    // wavefront as `a[i]`, so a positive result reads as "the right capsule
    // heard it n samples later", which is the direction an operator expects.
    let (first_start, second_start) = if lag >= 0 {
        (0usize, lag.unsigned_abs() as usize)
    } else {
        (lag.unsigned_abs() as usize, 0usize)
    };
    if first_start >= first.len() || second_start >= second.len() {
        return 0.0;
    }
    let overlap = (first.len() - first_start).min(second.len() - second_start);
    if overlap == 0 {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut energy_first = 0.0_f32;
    let mut energy_second = 0.0_f32;
    for i in 0..overlap {
        let x = first[first_start + i];
        let y = second[second_start + i];
        // `mul_add` keeps the intermediate at full precision, which matters
        // here: these accumulate over hundreds of thousands of samples.
        dot = x.mul_add(y, dot);
        energy_first = x.mul_add(x, energy_first);
        energy_second = y.mul_add(y, energy_second);
    }
    if energy_first <= 0.0 || energy_second <= 0.0 {
        return 0.0;
    }
    dot / (energy_first.sqrt() * energy_second.sqrt())
}

/// Shift `right` by `lag` samples and add it to `left`, over the overlap.
fn aligned_sum(left: &[f32], right: &[f32], lag: i32) -> Vec<f32> {
    // Same convention as `correlation_at`: at lag +n the right channel trails,
    // so it is the one advanced to bring the wavefronts together.
    let (l_start, r_start) = if lag >= 0 {
        (0usize, lag.unsigned_abs() as usize)
    } else {
        (lag.unsigned_abs() as usize, 0usize)
    };
    if l_start >= left.len() || r_start >= right.len() {
        return Vec::new();
    }
    let n = (left.len() - l_start).min(right.len() - r_start);
    (0..n)
        .map(|i| f32::midpoint(left[l_start + i], right[r_start + i]))
        .collect()
}

/// Analyse a stereo pair. Pure, so every number in the report is testable
/// without an audio device.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the lag bound is a small positive count of samples"
)]
pub fn analyse(left: &[f32], right: &[f32], sample_rate: u32) -> ChannelAnalysis {
    let max_lag = ((MAX_LAG_MS / 1000.0) * sample_rate as f32) as i32;
    let mut best_lag = 0_i32;
    let mut best_corr = f32::NEG_INFINITY;
    for lag in -max_lag..=max_lag {
        let c = correlation_at(left, right, lag);
        if c > best_corr {
            best_corr = c;
            best_lag = lag;
        }
    }
    if !best_corr.is_finite() {
        best_corr = 0.0;
    }

    let average: Vec<f32> = left.iter().zip(right).map(|(l, r)| (l + r) / 2.0).collect();

    ChannelAnalysis {
        left_rms: rms(left),
        right_rms: rms(right),
        average_rms: rms(&average),
        aligned_sum_rms: rms(&aligned_sum(left, right, best_lag)),
        best_lag_samples: best_lag,
        best_correlation: best_corr,
        zero_lag_correlation: correlation_at(left, right, 0),
        sample_rate,
    }
}

/// De-interleave S16LE bytes into two f32 channels in `[-1, 1]`.
#[must_use]
pub fn deinterleave_s16le(bytes: &[u8]) -> (Vec<f32>, Vec<f32>) {
    let frames = bytes.len() / 4;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for f in bytes[..frames * 4].chunks_exact(4) {
        left.push(f32::from(i16::from_le_bytes([f[0], f[1]])) / 32768.0);
        right.push(f32::from(i16::from_le_bytes([f[2], f[3]])) / 32768.0);
    }
    (left, right)
}

/// Why a recording attempt produced nothing.
///
/// Separated from a plain string because the two cases call for opposite
/// advice: a missing tool says nothing whatsoever about the microphone, and
/// reporting it as "this is not a stereo source" would send an operator to
/// replace hardware that is fine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// `arecord` is not installed or not on `PATH`.
    ArecordMissing,
    /// `arecord` ran but produced no audio.
    NoAudio(String),
    /// Anything else went wrong while running it.
    Failed(String),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArecordMissing => write!(
                f,
                "arecord not found on PATH — install alsa-utils. This says nothing \
                 about the microphone; the report never got as far as opening it"
            ),
            Self::NoAudio(d) => write!(
                f,
                "arecord produced no audio from {d}.\n\n  \
                 The likeliest cause is that the station is running: an ALSA capture \
                 device is exclusive, so stop it first\n  (sudo systemctl stop \
                 birdnet-behavior), run this again, then start it.\n\n  \
                 Otherwise: check `arecord -l` for the right device name. A device \
                 that genuinely cannot open two\n  channels is not a stereo source, \
                 and nothing in this report applies to it"
            ),
            Self::Failed(e) => write!(f, "{e}"),
        }
    }
}

/// Record `secs` of raw interleaved stereo S16LE from `device` via `arecord`.
fn record_stereo(device: &str, sample_rate: u32, secs: u32) -> Result<Vec<u8>, RecordError> {
    let mut child = Command::new("arecord")
        .args([
            "-D",
            device,
            "-f",
            "S16_LE",
            "-c",
            "2",
            "-r",
            &sample_rate.to_string(),
            "-t",
            "raw",
            "-d",
            &secs.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RecordError::ArecordMissing
            } else {
                RecordError::Failed(format!("could not run arecord: {e}"))
            }
        })?;

    let mut buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut buf)
            .map_err(|e| RecordError::Failed(format!("reading arecord output: {e}")))?;
    }
    let _ = child
        .wait()
        .map_err(|e| RecordError::Failed(format!("waiting for arecord: {e}")))?;
    if buf.is_empty() {
        return Err(RecordError::NoAudio(device.to_owned()));
    }
    Ok(buf)
}

/// Render the human-readable report.
#[must_use]
pub fn render(a: &ChannelAnalysis, device: &str, secs: u32) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Channel report — {device}, {secs}s at {} Hz\n",
        a.sample_rate
    );

    let _ = writeln!(s, "Levels");
    let _ = writeln!(s, "  left                    RMS {:.5}", a.left_rms);
    let _ = writeln!(s, "  right                   RMS {:.5}", a.right_rms);

    let _ = writeln!(s, "\nCapsule geometry");
    let _ = writeln!(
        s,
        "  best-fit delay          {:+} samples ({:+.2} ms, ~{:.1} cm path difference; \
         positive = right heard it later)",
        a.best_lag_samples,
        a.best_lag_ms(),
        a.path_difference_cm()
    );
    let _ = writeln!(
        s,
        "  correlation             {:.3} aligned, {:.3} unaligned",
        a.best_correlation, a.zero_lag_correlation
    );
    let _ = writeln!(
        s,
        "  verdict                 {}",
        if a.capsules_look_spaced() {
            "SPACED — averaging will cancel some frequencies"
        } else if a.best_correlation > 0.5 {
            "coincident (or near enough) — averaging is safe"
        } else {
            "channels are not a delayed copy of one another; check for a dead \
             or unplugged capsule"
        }
    );

    let _ = writeln!(s, "\nWhat each setting hands the model");
    let _ = writeln!(
        s,
        "  Stereo (averaged, today) RMS {:.5}  {:+.1} dB vs the louder channel",
        a.average_rms,
        a.average_vs_best_single_db()
    );
    let _ = writeln!(
        s,
        "  {:<8} (single channel) RMS {:.5}   0.0 dB (baseline)",
        a.louder_channel(),
        a.best_single_rms()
    );
    let _ = writeln!(
        s,
        "  delay-aligned sum        RMS {:.5}  {:+.1} dB vs averaging (not implemented yet)",
        a.aligned_sum_rms,
        a.aligned_vs_average_db()
    );

    let _ = writeln!(s, "\nRecommendation");
    if a.capsules_look_spaced() && a.average_vs_best_single_db() < -1.0 {
        let _ = writeln!(
            s,
            "  Set this source to {} on the Audio page. Averaging is currently\n  \
             throwing away {:.1} dB against the better single capsule.",
            if a.louder_channel() == "left" {
                "Left"
            } else {
                "Right"
            },
            -a.average_vs_best_single_db()
        );
    } else if a.capsules_look_spaced() {
        let _ = writeln!(
            s,
            "  The capsules are spaced, but averaging is not costing much in this\n  \
             sample. Re-run while a bird is actually singing — the cancellation is\n  \
             direction-dependent and ambient noise will not show it."
        );
    } else {
        let _ = writeln!(
            s,
            "  Nothing to change. Averaging is not costing you signal with this mic."
        );
    }
    let _ = writeln!(
        s,
        "\n  One sample of ambient sound is not a detection-rate experiment. This\n  \
         measures signal delivered to the model, not species identified."
    );
    s
}

/// Entry point for `--channel-report`.
///
/// Returns the process exit code: 0 when the report was produced, 2 when the
/// device could not be recorded from.
pub fn run(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> i32 {
    let device = cli
        .alsa_device
        .clone()
        .or_else(|| cli.alsa_devices.first().cloned())
        .or_else(|| config.and_then(|c| c.get("ALSA_CARD").map(ToOwned::to_owned)))
        .unwrap_or_else(|| "plughw:1,0".to_owned());
    let secs = cli.channel_report_secs.max(1);
    let sample_rate = 48_000;

    println!("Recording {secs}s of stereo from {device} …\n");
    let bytes = match record_stereo(&device, sample_rate, secs) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("channel report failed: {e}");
            return 2;
        }
    };

    let (left, right) = deinterleave_s16le(&bytes);
    let analysis = analyse(&left, &right, sample_rate);
    print!("{}", render(&analysis, &device, secs));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tone, optionally delayed, at `freq` Hz.
    #[allow(
        clippy::cast_precision_loss,
        reason = "one second of samples is exact in f32"
    )]
    fn tone(n: usize, freq: f32, sample_rate: f32, delay: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                if i < delay {
                    0.0
                } else {
                    let t = (i - delay) as f32 / sample_rate;
                    (t * 2.0 * std::f32::consts::PI * freq).sin() * amp
                }
            })
            .collect()
    }

    #[test]
    fn coincident_capsules_lose_nothing_to_averaging() {
        let left = tone(48_000, 2000.0, 48_000.0, 0, 0.3);
        let right = left.clone();
        let a = analyse(&left, &right, 48_000);

        assert_eq!(a.best_lag_samples, 0);
        assert!(
            !a.capsules_look_spaced(),
            "identical channels are not spaced"
        );
        assert!(
            a.average_vs_best_single_db().abs() < 0.1,
            "averaging identical channels changes nothing: {} dB",
            a.average_vs_best_single_db()
        );
    }

    /// The case the whole report exists for.
    ///
    /// Half a period of delay is the worst case: the average very nearly
    /// vanishes, while either channel alone is untouched. A station in this
    /// state is feeding BirdNET almost nothing and has no way to notice.
    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a fraction of an audio period is a small positive sample count"
    )]
    fn half_period_delay_is_caught_and_the_loss_quantified() {
        let sr = 48_000.0_f32;
        let freq = 2000.0_f32;
        let half_period = (sr / freq / 2.0) as usize; // 12 samples
        let left = tone(48_000, freq, sr, 0, 0.3);
        let right = tone(48_000, freq, sr, half_period, 0.3);

        let a = analyse(&left, &right, 48_000);
        assert!(
            a.capsules_look_spaced(),
            "a half-period delay is a spaced pair: {a:?}"
        );
        assert!(
            a.average_vs_best_single_db() < -20.0,
            "averaging must be reported as a large loss, got {} dB",
            a.average_vs_best_single_db()
        );
        assert!(
            a.aligned_vs_average_db() > 20.0,
            "aligning must be reported as a large gain, got {} dB",
            a.aligned_vs_average_db()
        );
        // And the recommendation must actually name a channel.
        let out = render(&a, "plughw:1,0", 5);
        assert!(out.contains("SPACED"), "{out}");
        assert!(
            out.contains("Set this source to Left") || out.contains("Set this source to Right"),
            "{out}"
        );
    }

    /// A quarter period is the milder, likelier case: a real but survivable
    /// loss. It must still be reported rather than rounded away.
    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a fraction of an audio period is a small positive sample count"
    )]
    fn quarter_period_delay_reports_a_modest_loss() {
        let sr = 48_000.0_f32;
        let freq = 2000.0_f32;
        let left = tone(48_000, freq, sr, 0, 0.3);
        let right = tone(48_000, freq, sr, (sr / freq / 4.0) as usize, 0.3);

        let a = analyse(&left, &right, 48_000);
        let loss = a.average_vs_best_single_db();
        assert!(
            (-4.0..-1.0).contains(&loss),
            "expected roughly -3 dB, got {loss}"
        );
    }

    #[test]
    fn delay_is_reported_as_a_plausible_capsule_spacing() {
        let sr = 48_000.0_f32;
        let left = tone(48_000, 2000.0, sr, 0, 0.3);
        // 12 samples at 48 kHz is 0.25 ms — about 8.5 cm of path difference.
        let right = tone(48_000, 2000.0, sr, 12, 0.3);
        let a = analyse(&left, &right, 48_000);
        assert!((a.best_lag_ms() - 0.25).abs() < 0.02, "{}", a.best_lag_ms());
        assert!(
            (a.path_difference_cm() - 8.5).abs() < 1.0,
            "{}",
            a.path_difference_cm()
        );
    }

    /// Uncorrelated channels must not be dressed up as a spaced pair — that
    /// reads as a dead capsule or a mis-wired input, and the report says so.
    #[test]
    fn uncorrelated_channels_are_not_called_spaced() {
        let left = tone(48_000, 2000.0, 48_000.0, 0, 0.3);
        let right = tone(48_000, 5300.0, 48_000.0, 0, 0.3);
        let a = analyse(&left, &right, 48_000);
        assert!(!a.capsules_look_spaced(), "{a:?}");
        let out = render(&a, "plughw:1,0", 5);
        assert!(out.contains("dead"), "{out}");
    }

    #[test]
    fn a_silent_channel_does_not_produce_infinities() {
        let left = tone(48_000, 2000.0, 48_000.0, 0, 0.3);
        let right = vec![0.0_f32; 48_000];
        let a = analyse(&left, &right, 48_000);
        assert!(a.average_vs_best_single_db().is_finite());
        assert!(a.aligned_vs_average_db().is_finite());
        let out = render(&a, "plughw:1,0", 5);
        assert!(!out.contains("inf"), "{out}");
    }

    /// A missing tool and a non-stereo device must not read the same.
    ///
    /// Reporting "this is not a stereo source" when `arecord` simply is not
    /// installed would send an operator to replace a microphone that is fine.
    #[test]
    fn a_missing_tool_is_not_reported_as_a_bad_microphone() {
        let missing = RecordError::ArecordMissing.to_string();
        assert!(missing.contains("alsa-utils"), "{missing}");
        assert!(
            !missing.contains("not a stereo source"),
            "a missing tool says nothing about the mic: {missing}"
        );

        let no_audio = RecordError::NoAudio("plughw:1,0".into()).to_string();
        assert!(no_audio.contains("plughw:1,0"), "{no_audio}");
        // The exclusive-device case is the likeliest and must be named first.
        assert!(no_audio.contains("systemctl stop"), "{no_audio}");
    }

    #[test]
    fn deinterleave_splits_the_channels() {
        // frames: (1, -1), (2, -2)
        let bytes: Vec<u8> = [1i16, -1, 2, -2]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let (l, r) = deinterleave_s16le(&bytes);
        assert_eq!(l.len(), 2);
        assert_eq!(r.len(), 2);
        assert!(l[0] > 0.0 && r[0] < 0.0);
        assert!(l[1] > l[0] && r[1] < r[0]);
    }

    #[test]
    fn deinterleave_ignores_a_trailing_partial_frame() {
        let mut bytes: Vec<u8> = [1i16, -1].iter().flat_map(|s| s.to_le_bytes()).collect();
        bytes.push(0x7f); // half a sample of the next frame
        let (l, r) = deinterleave_s16le(&bytes);
        assert_eq!(l.len(), 1);
        assert_eq!(r.len(), 1);
    }
}
