//! Asking a capture device what sample rates it actually supports.
//!
//! # The gap
//!
//! Autodetected microphones were configured at 48 kHz because that is what most
//! USB capture devices do. A device that does not — a 44.1 kHz-only interface, a
//! 16 kHz conference microphone, a cheap dongle that offers 8 kHz and nothing
//! else — is handed `-r 48000` and either fails to start (so the supervisor
//! restarts it forever, backing off, with an ALSA error nobody reads) or is
//! silently plug-converted by ALSA from a rate it does have, which is worse:
//! capture works, the station records, and every spectrogram has been resampled
//! from something narrower than it claims.
//!
//! # What this does
//!
//! Runs `arecord --dump-hw-params`, which reports a device's capabilities and
//! exits without recording, and parses the `RATE:` line out of it.
//!
//! # What it does not do, deliberately
//!
//! Every failure — no `arecord`, an unreadable device, a line in a shape this
//! does not recognise — yields [`RateSupport::Unknown`], and the caller keeps
//! the rate it would have used anyway. The probe can improve a station's
//! configuration; it can never stop one starting. That matters more than usual
//! here because the parser below was written against `arecord`'s documented
//! output format rather than against a device: this project's CI has no sound
//! card, so the fixtures are constructed, not captured. A constructed fixture
//! can be wrong about a driver's exact spacing in a way no test here would
//! catch — so the code is arranged such that being wrong costs nothing.

use std::process::Command;

/// What rates a device reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateSupport {
    /// A specific set, as `arecord` prints for a device with a fixed list.
    Discrete(Vec<u32>),
    /// A continuous range, inclusive at both ends.
    Range {
        /// Lowest supported rate.
        min: u32,
        /// Highest supported rate.
        max: u32,
    },
    /// Nothing could be determined. The caller keeps its configured rate.
    Unknown,
}

/// Parse the `RATE:` line out of `arecord --dump-hw-params` output.
///
/// Three shapes are recognised, all of which the tool emits depending on the
/// driver:
///
/// ```text
/// RATE: 48000              -> Discrete([48000])
/// RATE: 44100 48000        -> Discrete([44100, 48000])
/// RATE: [44100 48000]      -> Range { min: 44100, max: 48000 }
/// ```
///
/// Anything else is [`RateSupport::Unknown`]. The tool's output is not a
/// stable interface, so an unrecognised shape has to be survivable rather than
/// an error.
#[must_use]
pub fn parse_hw_params_rates(output: &str) -> RateSupport {
    for line in output.lines() {
        let trimmed = line.trim();
        // `RATE:` and not `RATE_NUM:` or a substring of another key: the dump
        // carries several lines whose names start the same way.
        let Some(rest) = trimmed.strip_prefix("RATE:") else {
            continue;
        };
        let rest = rest.trim();

        let bracketed = rest.starts_with('[') && rest.ends_with(']');
        let body = if bracketed {
            &rest[1..rest.len() - 1]
        } else {
            rest
        };

        let values: Vec<u32> = body
            .split_whitespace()
            .filter_map(|t| t.parse::<u32>().ok())
            .filter(|r| *r > 0)
            .collect();

        if values.is_empty() {
            // A `RATE:` line whose values could not be read is not evidence of
            // anything; keep scanning in case a later line is readable.
            continue;
        }
        if bracketed {
            // A single-valued bracket (`[48000]`) is a range of one, which is
            // the same claim as a discrete list of one.
            let min = *values.iter().min().unwrap_or(&0);
            let max = *values.iter().max().unwrap_or(&0);
            return RateSupport::Range { min, max };
        }
        return RateSupport::Discrete(values);
    }
    RateSupport::Unknown
}

/// Choose the rate to capture at.
///
/// `preferred` is what the model wants — 48 kHz for V2.4, 32 kHz for V3.0.
///
/// `None` means "keep what you were going to use": either the device reported
/// nothing readable, or it reported exactly the preferred rate and there is
/// nothing to change.
///
/// When the preferred rate is unavailable, the **lowest supported rate above
/// it** wins, and only if there is none does the highest below it. Bird song
/// runs to about 10 kHz, so a rate below the target throws away signal the
/// model was trained to use, while one above it costs only CPU on the resample
/// the pipeline already performs.
#[must_use]
pub fn pick_rate(support: &RateSupport, preferred: u32) -> Option<u32> {
    match support {
        RateSupport::Unknown => None,
        RateSupport::Range { min, max } => {
            if preferred >= *min && preferred <= *max {
                None
            } else if preferred < *min {
                Some(*min)
            } else {
                Some(*max)
            }
        }
        RateSupport::Discrete(rates) => {
            if rates.contains(&preferred) {
                return None;
            }
            rates
                .iter()
                .filter(|r| **r > preferred)
                .min()
                .or_else(|| rates.iter().max())
                .copied()
        }
    }
}

/// Ask `arecord` what `device` supports.
///
/// The only impure step; every decision about the answer is in
/// [`parse_hw_params_rates`] and [`pick_rate`], which are tested without a
/// sound card.
///
/// `arecord` writes the dump to **stderr** and exits non-zero, because from its
/// point of view the recording was refused. Both are expected, so neither is
/// treated as failure — only an absent binary or unreadable output is.
#[must_use]
pub fn probe_alsa_rates(device: &str) -> RateSupport {
    let output = Command::new("arecord")
        .args(["-D", device, "--dump-hw-params", "-d", "1"])
        .arg("/dev/null")
        .output();

    match output {
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            );
            parse_hw_params_rates(&text)
        }
        Err(e) => {
            tracing::debug!(device, error = %e, "could not probe capture rates");
            RateSupport::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RateSupport, parse_hw_params_rates, pick_rate};

    /// A dump in `arecord`'s documented shape, with `rate_line` substituted.
    ///
    /// Constructed rather than captured: this project's CI has no sound card.
    /// The surrounding lines are here because the parser has to pick `RATE:`
    /// out of a dump that contains several similarly named keys — which is the
    /// part a one-line fixture would not exercise.
    fn dump(rate_line: &str) -> String {
        format!(
            "Recording WAVE '/dev/null' : Signed 16 bit Little Endian, Rate 48000 Hz, Mono\n\
             HW Params of device \"hw:1,0\":\n\
             --------------------\n\
             ACCESS:  MMAP_INTERLEAVED RW_INTERLEAVED\n\
             FORMAT:  S16_LE S32_LE\n\
             SUBFORMAT:  STD\n\
             SAMPLE_BITS: [16 32]\n\
             FRAME_BITS: [16 64]\n\
             CHANNELS: [1 2]\n\
             RATE_NUM: 48000\n\
             RATE_DEN: 1\n\
             {rate_line}\n\
             PERIOD_TIME: [125 8000000]\n\
             PERIOD_SIZE: [6 384000]\n\
             BUFFER_TIME: [125 16000000]\n\
             --------------------\n\
             arecord: set_params:1416: Sample format non available\n"
        )
    }

    // ── parsing ─────────────────────────────────────────────────────────

    #[test]
    fn a_single_rate_is_read_as_a_one_element_list() {
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: 48000")),
            RateSupport::Discrete(vec![48_000])
        );
    }

    #[test]
    fn a_space_separated_list_is_read_in_full() {
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: 8000 16000 44100 48000")),
            RateSupport::Discrete(vec![8_000, 16_000, 44_100, 48_000])
        );
    }

    #[test]
    fn a_bracketed_pair_is_read_as_a_range() {
        // The distinction matters: a range claims everything between the ends,
        // a list claims only what it names. Reading `[8000 192000]` as a
        // two-element list would make 48 kHz look unsupported on a device that
        // plainly supports it.
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: [44100 48000]")),
            RateSupport::Range {
                min: 44_100,
                max: 48_000
            }
        );
    }

    #[test]
    fn a_bracketed_single_value_is_a_range_of_one() {
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: [48000]")),
            RateSupport::Range {
                min: 48_000,
                max: 48_000
            }
        );
    }

    #[test]
    fn rate_num_and_rate_den_are_not_mistaken_for_the_rate() {
        // The dump carries `RATE_NUM:` and `RATE_DEN:` — the rate as a
        // fraction — immediately before `RATE:`. A matcher looking for "RATE"
        // anywhere in the line, rather than the exact `RATE:` prefix, takes
        // `RATE_NUM: 48000` first and reports a discrete 48 kHz for a device
        // that actually offers a range.
        //
        // This test did not catch that at first: the fixture had no line a
        // loose matcher would trip on, so the guard it claimed to check was
        // never exercised. These two lines are in the real format and are the
        // ones that do it.
        let got = parse_hw_params_rates(&dump("RATE: [44100 48000]"));
        assert_eq!(
            got,
            RateSupport::Range {
                min: 44_100,
                max: 48_000
            },
            "the parser picked up a line that is not RATE"
        );
    }

    #[test]
    fn output_with_no_rate_line_is_unknown() {
        // `arecord` absent, a device that refused to open, a future version
        // that renames the key. Unknown is survivable; a wrong answer is not.
        assert_eq!(parse_hw_params_rates(""), RateSupport::Unknown);
        assert_eq!(
            parse_hw_params_rates("arecord: main:831: audio open error: No such file or directory"),
            RateSupport::Unknown
        );
    }

    #[test]
    fn an_unparseable_rate_line_is_unknown_rather_than_empty() {
        // A `RATE:` line whose values cannot be read is not evidence that the
        // device supports nothing — which is what an empty `Discrete` would
        // claim, and `pick_rate` would then have no answer to give.
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: all")),
            RateSupport::Unknown
        );
        assert_eq!(parse_hw_params_rates(&dump("RATE:")), RateSupport::Unknown);
        assert_eq!(
            parse_hw_params_rates(&dump("RATE: 0")),
            RateSupport::Unknown
        );
    }

    // ── choosing ────────────────────────────────────────────────────────

    #[test]
    fn a_device_that_supports_the_preferred_rate_is_left_alone() {
        // `None` is "keep what you were going to use". Returning `Some(48000)`
        // would be the same number but a different meaning, and the caller
        // logs the change it makes.
        assert_eq!(
            pick_rate(&RateSupport::Discrete(vec![44_100, 48_000]), 48_000),
            None
        );
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 8_000,
                    max: 192_000
                },
                48_000
            ),
            None
        );
    }

    #[test]
    fn an_unknown_device_is_left_alone() {
        // The whole failure path: a probe that learned nothing must not change
        // a station's configuration.
        assert_eq!(pick_rate(&RateSupport::Unknown, 48_000), None);
    }

    #[test]
    fn the_lowest_rate_above_the_target_wins() {
        // Bird song runs to about 10 kHz. A rate below the target throws away
        // signal the model was trained on; one above costs only CPU in a
        // resample the pipeline performs anyway.
        assert_eq!(
            pick_rate(
                &RateSupport::Discrete(vec![8_000, 16_000, 96_000, 192_000]),
                48_000
            ),
            Some(96_000),
            "the highest rate was taken when a nearer one was above the target"
        );
    }

    #[test]
    fn a_device_that_cannot_reach_the_target_gets_its_best() {
        // Counterpart: when nothing is above the target, the most bandwidth
        // available is the right answer — not a failure, and not the lowest.
        assert_eq!(
            pick_rate(&RateSupport::Discrete(vec![8_000, 16_000, 44_100]), 48_000),
            Some(44_100)
        );
    }

    #[test]
    fn a_range_is_clamped_to_the_nearer_end() {
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 8_000,
                    max: 16_000
                },
                48_000
            ),
            Some(16_000),
            "a device topping out below the target should give its maximum"
        );
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 96_000,
                    max: 192_000
                },
                48_000
            ),
            Some(96_000),
            "a device starting above the target should give its minimum"
        );
    }

    #[test]
    fn the_range_ends_are_inclusive() {
        // A device whose only rate is exactly the target must be left alone,
        // not nudged to an end it is already at.
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 48_000,
                    max: 48_000
                },
                48_000
            ),
            None
        );
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 16_000,
                    max: 48_000
                },
                48_000
            ),
            None
        );
        assert_eq!(
            pick_rate(
                &RateSupport::Range {
                    min: 48_000,
                    max: 192_000
                },
                48_000
            ),
            None
        );
    }

    #[test]
    fn a_v3_model_target_is_honoured_too() {
        // The preferred rate is the model's, not a constant: V3.0 wants 32 kHz.
        // A device offering 44.1/48 keeps 48; one offering only 16 keeps 16.
        assert_eq!(
            pick_rate(&RateSupport::Discrete(vec![44_100, 48_000]), 32_000),
            Some(44_100)
        );
        assert_eq!(
            pick_rate(&RateSupport::Discrete(vec![16_000]), 32_000),
            Some(16_000)
        );
        assert_eq!(
            pick_rate(&RateSupport::Discrete(vec![32_000]), 32_000),
            None
        );
    }
}
