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
///
/// # What the rounder passband costs
///
/// A tone at a band's centre reads true. White noise does not: it reads the
/// area under the squared response, and for this cascade that is 1.155 times
/// the nominal band — **+0.63 dB** in mid-spectrum, +0.38 dB in the 20 kHz band
/// at 48 kHz, where the zero at Nyquist trims the upper skirt. The sum of all
/// the bands on white noise is therefore +0.53 to +0.55 dB above the broadband
/// power of the part of the spectrum the bands cover (measured at 48, 44.1, 32
/// and 22.05 kHz; the same figure from integrating the responses). The same
/// three biquads staggered as a Butterworth bandpass would read +0.20 dB on
/// noise and fall 18.2 dB at the neighbouring band centre, against 9.3 dB
/// here — computed from the ideal analogue responses, not built. This
/// design is kept because the rejection table above is pinned by tests and
/// because a tone, which is what the station's questions are about, reads
/// exactly; but the figure is a property of the shape, not of a defect, and a
/// reading of broadband noise should be taken as about half a decibel high.
pub const THIRD_OCTAVE_SECTIONS: usize = 3;

/// `x = f/f₀ − f₀/f` evaluated at a third-octave band edge.
///
/// The bandpass magnitude is `1/√(1 + Q²x²)`, so this is the quantity that
/// turns a target attenuation at the band edge into a section `Q`.
fn edge_detuning() -> f64 {
    let r = f64::from(THIRD_OCTAVE_EDGE_RATIO);
    r - 1.0 / r
}

/// `√(2^(1/n) − 1)`: the value `Q · x` must take at a band edge for `n`
/// identical sections to cascade to −3.01 dB there.
///
/// Each section must be `2^(−1/2n)` in magnitude at the edge; with
/// `|H|² = 1/(1 + Q²x²)` that gives `Q²x² = 2^(1/n) − 1`.
fn section_edge_k(sections: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = sections.max(1) as f64;
    ((1.0 / n).exp2() - 1.0).sqrt()
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
    section_edge_k(sections) / edge_detuning()
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
///
/// # Where the edges land
///
/// The sections are designed so the composite is 3.01 dB down at the nominal
/// edges `f₀ · 2^(±1/6)` **in the digital domain**, at every band and every
/// sample rate. That takes more than choosing the section `Q`: the bilinear
/// transform maps analogue frequency `Ω` to `tan(πf/fs)`, so the two edges are
/// not symmetric about the centre once the band is a sizeable fraction of
/// Nyquist, and a filter that is symmetric about the geometric centre puts one
/// edge too low and the other too high. The previous design, which pre-warped
/// only the *bandwidth* ([`Biquad::bandpass`]), read a unit sine at the 20 kHz
/// band's edges at 48 kHz as −4.38 dB and −8.91 dB against the −6.02 it should
/// show: +1.6 and −2.9 dB. The other rates are the same shape, further down
/// the spectrum (16 kHz at 44.1 kHz: +0.9 / −1.0 dB; 12.5 kHz at 32 kHz:
/// +1.3 / −1.7 dB; 8 kHz at 22.05 kHz: +0.9 / −1.0 dB).
///
/// So the design takes the two edges through `tan`, centres the analogue
/// prototype on their geometric mean in the warped domain, and sets `Q` from
/// their separation there. Both edges land on −3.01 dB (measured to 0.003 dB
/// in the top bands of all four rates); the peak sits at the warped mean,
/// which is a little above the nominal centre where the band nears Nyquist.
/// The price is paid by a tone at the nominal centre: −0.34 dB for the 20 kHz
/// band at 48 kHz (the peak is 4 % high), −0.19 dB for 12.5 kHz at 32 kHz,
/// −0.09 dB for 16 kHz at 44.1 kHz and 8 kHz at 22.05 kHz, and under 0.02 dB
/// for every band in the lowest fifth of the spectrum. `tests/soundlevel_accuracy.rs`
/// measures all of these.
#[derive(Debug, Clone)]
pub struct ThirdOctaveBand {
    sections: [Biquad; THIRD_OCTAVE_SECTIONS],
}

impl ThirdOctaveBand {
    /// Design the cascade for `centre_hz` at `sample_rate`.
    ///
    /// # Errors
    ///
    /// * [`BiquadError::NonPositiveParameter`] for a zero or negative centre
    ///   or sample rate.
    /// * [`BiquadError::AboveNyquist`] when the band's **upper edge**
    ///   (`centre_hz · 2^(1/6)`) is at or above Nyquist. The centre being
    ///   below Nyquist is not enough: the band at 16 kHz sampled at 32 kHz has
    ///   its centre at 15.85 kHz and its upper edge at 17.79 kHz, and a filter
    ///   designed there folds back instead of rolling off.
    /// * [`BiquadError::Unstable`] as [`Biquad::bandpass_q`].
    pub fn new(centre_hz: f32, sample_rate: u32) -> Result<Self, BiquadError> {
        if centre_hz <= 0.0 || sample_rate == 0 {
            return Err(BiquadError::NonPositiveParameter);
        }
        let fs = f64::from(sample_rate);
        let ratio = f64::from(THIRD_OCTAVE_EDGE_RATIO);
        let centre = f64::from(centre_hz);
        let (lower, upper) = (centre / ratio, centre * ratio);
        if upper * 2.0 >= fs {
            return Err(BiquadError::AboveNyquist);
        }

        // Analogue (warped) frequencies of the two edges: Ω = tan(π f / fs).
        let warp = |hz: f64| (std::f64::consts::PI * hz / fs).tan();
        let (omega_lower, omega_upper) = (warp(lower), warp(upper));
        // The prototype is symmetric in Ω/Ω_c − Ω_c/Ω, so it is centred on
        // the geometric mean of the warped edges, and its Q follows from the
        // attenuation each section must show there (see `edge_detuning` for
        // the unwarped version of the same arithmetic).
        let omega_c = (omega_lower * omega_upper).sqrt();
        let q = section_edge_k(THIRD_OCTAVE_SECTIONS) * omega_c / (omega_upper - omega_lower);
        // The digital frequency whose warped value is Ω_c.
        let peak_hz = omega_c.atan() * fs / std::f64::consts::PI;

        #[allow(clippy::cast_possible_truncation)]
        let section = Biquad::bandpass_q(peak_hz as f32, sample_rate, q as f32)?;
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
