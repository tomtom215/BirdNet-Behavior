//! Third-octave sound level measurement — what the site sounds like, not
//! whether a chunk is worth classifying.
//!
//! # Why this is not the quality module
//!
//! [`crate::audio::quality`] answers "is this three seconds of audio clean
//! enough to hand to the classifier": one broadband signal-to-noise figure, a
//! spectral flatness, an adaptive noise floor, a rain flag. It is a gate, and
//! everything it computes is discarded once the gate has been passed.
//!
//! This module answers a different question, and keeps the answer. A level in
//! each of the 30 ISO 266 third-octave bands, once per interval, stored, is the
//! standard unit of acoustic-ecology fieldwork. It is what shows a road opening
//! two valleys away, a generator running at night, or the dawn chorus rising
//! 12 dB in the 2–4 kHz bands across six weeks of spring — none of which a
//! broadband SNR separates from "a quiet night".
//!
//! It also diagnoses the station itself. A microphone going deaf loses the top
//! bands first; a preamp oscillating shows a single band climbing with nothing
//! either side; wind and a badly-mounted housing live below 200 Hz. All three
//! read as "SNR got worse" to a broadband measure, and all three have different
//! remedies.
//!
//! # What it costs
//!
//! One biquad per band per sample — 30 multiply-accumulate chains over samples
//! already in memory from the capture tee. Memory is constant and under 4 kB;
//! see [`SoundLevelMeter`] for why that is worth stating.
//!
//! # dBFS, and the honesty of the numbers
//!
//! By default the meter reports **dBFS** — decibels relative to digital full
//! scale — which is negative, station-relative, and meaningless to compare
//! against another station or a published figure. That is what an uncalibrated
//! microphone can honestly produce, and it is entirely sufficient for every
//! question above, all of which are about change over time at one place.
//!
//! [`Calibration::SplOffsetDb`] converts to dB SPL once an operator has
//! measured the offset against a known source. The unit travels with the
//! reading ([`Calibration::unit`]) so a chart cannot silently relabel one as
//! the other.

mod bands;
mod filter;
mod meter;

pub use bands::{
    CENTRE_FREQUENCIES_HZ, THIRD_OCTAVE_EDGE_RATIO, a_weighting_db, band_edges, band_label,
    exact_centre_hz, third_octave_q,
};
pub use filter::{
    Biquad, BiquadError, THIRD_OCTAVE_SECTIONS, ThirdOctaveBand, section_bandwidth_octaves,
    section_q,
};
pub use meter::{
    BandLevel, Calibration, FLOOR_DBFS, NYQUIST_MARGIN, SoundLevelMeter, SoundLevelReading,
    db_to_power, label_for, power_to_db,
};

#[cfg(test)]
mod tests;
