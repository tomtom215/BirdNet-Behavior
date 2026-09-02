//! A biquad bandpass, and the arithmetic that makes one stable.
//!
//! Shared, not private to the sound-level meter: the same primitive is what a
//! configurable per-source equaliser needs, and two implementations of a
//! transposed direct-form II biquad in one codebase would be one too many.

use super::bands::THIRD_OCTAVE_EDGE_RATIO;

/// One second-order section, in transposed direct form II.
///
/// Transposed form rather than the textbook direct form I: it needs two state
/// words instead of four, and its rounding behaviour is better at the high `Q`
/// a third-octave band asks for (about 4.3), where the poles sit close enough
/// to the unit circle that accumulated error is a real concern on `f32`. The
/// state is `f64` for the same reason, even though the samples are `f32` —
/// a resonant recursive filter is exactly where the extra mantissa earns its
/// keep, and the cost is one register.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f64,
    s2: f64,
}

/// Why a set of coefficients was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiquadError {
    /// Centre frequency or sample rate was zero or negative.
    NonPositiveParameter,
    /// The band's upper edge lands at or above Nyquist, where a bandpass
    /// designed by the bilinear transform folds instead of rolling off.
    AboveNyquist,
    /// The resulting poles are on or outside the unit circle.
    Unstable,
}

impl core::fmt::Display for BiquadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonPositiveParameter => write!(f, "centre frequency and sample rate must be > 0"),
            Self::AboveNyquist => write!(f, "band upper edge is at or above Nyquist"),
            Self::Unstable => write!(f, "coefficients place a pole on or outside the unit circle"),
        }
    }
}

impl std::error::Error for BiquadError {}

impl Biquad {
    /// A constant-0 dB-peak bandpass centred on `centre_hz`, from the RBJ
    /// audio EQ cookbook.
    ///
    /// "Constant peak gain" is the variant that matters here: the other RBJ
    /// bandpass form has a peak gain of `Q`, so a level measured through it
    /// would carry a per-band offset — about 12.7 dB at a third-octave `Q` —
    /// that varies with the band. Reading that as a sound level would put
    /// every band too loud, uniformly enough to look plausible.
    ///
    /// `bandwidth_octaves` rather than `Q`, because `alpha = sin(ω₀)/2Q` — the
    /// form that reads more naturally and that the reference implementation
    /// uses — only realises the requested `Q` in the low-frequency limit. It
    /// warps towards Nyquist: measured on the 10 kHz band at 48 kHz, the lower
    /// −3 dB edge landed at −4.35 dB instead of −3.01, so the band was
    /// measurably narrower than the one it claimed to be. The `sinh` form
    /// below pre-warps and holds the edges across the whole range. Use
    /// [`Self::bandpass_q`] when a `Q` really is what you have.
    ///
    /// # Errors
    ///
    /// [`BiquadError`] when the parameters are non-positive, when the band
    /// reaches Nyquist, or when the resulting poles are not inside the unit
    /// circle.
    pub fn bandpass(
        centre_hz: f32,
        sample_rate: u32,
        bandwidth_octaves: f64,
    ) -> Result<Self, BiquadError> {
        if centre_hz <= 0.0 || sample_rate == 0 || bandwidth_octaves <= 0.0 {
            return Err(BiquadError::NonPositiveParameter);
        }
        let fs = f64::from(sample_rate);
        let f0 = f64::from(centre_hz);
        if f0 * 2.0 >= fs {
            return Err(BiquadError::AboveNyquist);
        }

        let omega = 2.0 * std::f64::consts::PI * f0 / fs;
        let sin_w = omega.sin();
        let cos_w = omega.cos();
        if sin_w <= 0.0 {
            return Err(BiquadError::AboveNyquist);
        }
        let alpha =
            sin_w * (std::f64::consts::LN_2 / 2.0 * bandwidth_octaves * omega / sin_w).sinh();

        Self::from_alpha(alpha, cos_w)
    }

    /// A constant-0 dB-peak bandpass specified by `Q` rather than bandwidth.
    ///
    /// The un-pre-warped `alpha = sin(ω₀)/2Q` form. Correct where `ω₀` is
    /// small, and the natural parameterisation for a user-facing equaliser
    /// where `Q` is the knob on the panel. [`Self::bandpass`] is what a
    /// measurement wants.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn bandpass_q(centre_hz: f32, sample_rate: u32, q: f32) -> Result<Self, BiquadError> {
        if centre_hz <= 0.0 || sample_rate == 0 || q <= 0.0 {
            return Err(BiquadError::NonPositiveParameter);
        }
        let fs = f64::from(sample_rate);
        let f0 = f64::from(centre_hz);
        if f0 * 2.0 >= fs {
            return Err(BiquadError::AboveNyquist);
        }
        let omega = 2.0 * std::f64::consts::PI * f0 / fs;
        Self::from_alpha(omega.sin() / (2.0 * f64::from(q)), omega.cos())
    }

    /// Build the constant-peak-gain bandpass from its `alpha` and `cos ω₀`.
    fn from_alpha(alpha: f64, cos_w: f64) -> Result<Self, BiquadError> {
        let a0 = 1.0 + alpha;
        let filter = Self {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: (-2.0 * cos_w) / a0,
            a2: (1.0 - alpha) / a0,
            s1: 0.0,
            s2: 0.0,
        };

        // Jury's stability test for a second-order section: both poles lie
        // inside the unit circle exactly when |a2| < 1 and |a1| < 1 + a2.
        if filter.a2.abs() >= 1.0 || filter.a1.abs() >= 1.0 + filter.a2 {
            return Err(BiquadError::Unstable);
        }
        Ok(filter)
    }

    /// Filter one sample.
    #[inline]
    #[must_use]
    pub fn process(&mut self, x: f32) -> f64 {
        let x = f64::from(x);
        let y = self.b0.mul_add(x, self.s1);
        self.s1 = self.b1.mul_add(x, (-self.a1).mul_add(y, self.s2));
        self.s2 = self.b2.mul_add(x, -(self.a2 * y));
        y
    }

    /// Clear the delay line.
    pub const fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    /// Whether the internal state has gone non-finite.
    ///
    /// A stable filter driven by finite input cannot reach this, so it is a
    /// tripwire for a bug rather than an expected condition — but a station
    /// runs for months and a single denormal or a NaN arriving from a broken
    /// decode would otherwise poison every subsequent reading silently.
    #[must_use]
    pub const fn is_diverged(&self) -> bool {
        !self.s1.is_finite() || !self.s2.is_finite()
    }

    /// Run `n` zero samples through the filter to settle its transient.
    ///
    /// A freshly built biquad has zero state, which *is* settled — this exists
    /// for the case where a filter is reused across a discontinuity.
    pub fn warm_up(&mut self, n: usize) {
        for _ in 0..n {
            let _ = self.process(0.0);
        }
    }

    /// Magnitude response at `hz`, as a linear gain.
    ///
    /// Evaluates `|H(e^{jω})|` directly from the coefficients. Used by the
    /// tests to check the designed filter really is a bandpass centred where
    /// it claims, without running any audio through it, and by the equaliser
    /// UI to draw a response curve.
    #[must_use]
    pub fn magnitude_at(&self, hz: f32, sample_rate: u32) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f64::from(hz) / f64::from(sample_rate);
        let (sin1, cos1) = w.sin_cos();
        let (sin2, cos2) = (2.0 * w).sin_cos();
        // Numerator and denominator of H(z) evaluated at z = e^{jw}, written
        // as b0 + b1·e^{-jw} + b2·e^{-2jw} over 1 + a1·e^{-jw} + a2·e^{-2jw}.
        let num_re = self.b1.mul_add(cos1, self.b2.mul_add(cos2, self.b0));
        let num_im = -self.b1.mul_add(sin1, self.b2 * sin2);
        let den_re = self.a1.mul_add(cos1, self.a2.mul_add(cos2, 1.0));
        let den_im = -self.a1.mul_add(sin1, self.a2 * sin2);
        let num = num_re.hypot(num_im);
        let den = den_re.hypot(den_im);
        if den == 0.0 { f64::INFINITY } else { num / den }
    }
}

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
