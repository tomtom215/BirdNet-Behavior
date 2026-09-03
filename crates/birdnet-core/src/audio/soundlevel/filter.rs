//! The third-octave band filter: three biquads in series.

use super::bands::THIRD_OCTAVE_EDGE_RATIO;
use crate::audio::biquad::{Biquad, BiquadError};

// ---------------------------------------------------------------------------
// Third-octave cascade
// ---------------------------------------------------------------------------

/// Second-order sections cascaded to make one third-octave band.
///
/// One section is not enough. A single biquad at the third-octave `Q` of 4.318
/// rejects only 12.7 dB two bands away, so a loud tone shows up across half the
/// spectrum and a band's reading is not a measurement of that band. Three
/// sections take that to 22.5 dB for three times the arithmetic; a fourth buys
/// a further 3.1 dB for another third again, which is the point at which the
/// return stops paying on a Raspberry Pi.
///
/// | Sections | Section `Q` | 1 band away | 2 bands away | 1 octave away |
/// |---|---|---|---|---|
/// | 1 | 4.318 | −7.0 dB | −12.7 dB | −16.3 dB |
/// | 2 | 2.779 | −8.6 dB | −18.4 dB | −25.3 dB |
/// | **3** | **2.202** | **−9.4 dB** | **−22.5 dB** | **−32.3 dB** |
/// | 4 | 1.878 | −9.9 dB | −25.6 dB | −38.1 dB |
///
/// These are the analytic figures for the cascade, and
/// `the_cascade_rejection_table_is_accurate` checks the shipped filters
/// against them rather than letting the table become decoration.
///
/// This is a synchronously-tuned cascade, not a Butterworth: the sections are
/// identical, so the passband is rounder than a maximally-flat design of the
/// same order. It is not, and does not claim to be, an IEC 61260 class 1
/// filter — those masks are stricter than any of the rows above. What it does
/// guarantee is the property a level measurement needs: the composite is 3 dB
/// down at the nominal band edges, so the band measures the band it names.
pub const THIRD_OCTAVE_SECTIONS: usize = 3;

/// `x = f/f₀ − f₀/f` evaluated at a third-octave band edge.
///
/// The bandpass magnitude is `1/√(1 + Q²x²)`, so this is the quantity that
/// turns a target attenuation at the band edge into a section `Q`.
fn edge_detuning() -> f64 {
    let r = f64::from(THIRD_OCTAVE_EDGE_RATIO);
    r - 1.0 / r
}

/// The `Q` each section needs so that `sections` of them cascade to −3 dB at
/// the third-octave band edges.
///
/// The composite is `|H|^n`, so each section must be `2^(−1/2n)` at the edge.
/// Substituting into `|H|² = 1/(1 + Q²x²)` gives `Q = √(2^(1/n) − 1) / x`.
///
/// At `sections = 1` this returns 4.318 — the textbook third-octave `Q` —
/// which is the arithmetic checking itself: a one-section cascade must be the
/// ordinary third-octave filter and nothing else.
#[must_use]
pub fn section_q(sections: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = sections.max(1) as f64;
    ((1.0 / n).exp2() - 1.0).sqrt() / edge_detuning()
}

/// The −3 dB bandwidth, in octaves, of one section of an `n`-section cascade.
///
/// Inverts `Q = 1 / (2^(b/2) − 2^(−b/2))` for `b`, which is what
/// [`Biquad::bandpass`] wants. Solving `u − 1/u = 1/Q` for `u = 2^(b/2)` gives
/// one positive root.
#[must_use]
pub fn section_bandwidth_octaves(sections: usize) -> f64 {
    let inv_q = 1.0 / section_q(sections);
    let u = f64::midpoint(inv_q, inv_q.mul_add(inv_q, 4.0).sqrt());
    2.0 * u.log2()
}

/// A third-octave band filter: [`THIRD_OCTAVE_SECTIONS`] biquads in series.
#[derive(Debug, Clone)]
pub struct ThirdOctaveBand {
    sections: [Biquad; THIRD_OCTAVE_SECTIONS],
}

impl ThirdOctaveBand {
    /// Design the cascade for `centre_hz` at `sample_rate`.
    ///
    /// # Errors
    ///
    /// [`BiquadError`] as [`Biquad::bandpass`].
    pub fn new(centre_hz: f32, sample_rate: u32) -> Result<Self, BiquadError> {
        let bw = section_bandwidth_octaves(THIRD_OCTAVE_SECTIONS);
        let section = Biquad::bandpass(centre_hz, sample_rate, bw)?;
        Ok(Self {
            sections: [section; THIRD_OCTAVE_SECTIONS],
        })
    }

    /// Filter one sample through every section.
    #[inline]
    #[must_use]
    pub fn process(&mut self, x: f32) -> f64 {
        let mut y = f64::from(x);
        for section in &mut self.sections {
            #[allow(clippy::cast_possible_truncation)]
            {
                y = section.process(y as f32);
            }
        }
        y
    }

    /// Clear every section's delay line.
    pub const fn reset(&mut self) {
        let mut i = 0;
        while i < THIRD_OCTAVE_SECTIONS {
            self.sections[i].reset();
            i += 1;
        }
    }

    /// Whether any section's state has gone non-finite.
    #[must_use]
    pub fn is_diverged(&self) -> bool {
        self.sections.iter().any(Biquad::is_diverged)
    }

    /// Composite magnitude response at `hz`, as a linear gain.
    #[must_use]
    pub fn magnitude_at(&self, hz: f32, sample_rate: u32) -> f64 {
        self.sections
            .iter()
            .map(|s| s.magnitude_at(hz, sample_rate))
            .product()
    }
}
