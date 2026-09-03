//! A biquad — one second-order IIR section — and the RBJ cookbook designs.
//!
//! Shared rather than owned by whoever needed it first. Two consumers want the
//! same primitive for different reasons, and two implementations of a
//! transposed direct-form II biquad in one codebase would be one too many:
//!
//! * [`crate::audio::soundlevel`] cascades three of them per band to measure a
//!   third-octave spectrum, where what matters is that the band edges land
//!   where they claim.
//! * [`crate::audio::eq`] chains them to condition a microphone before
//!   inference, where what matters is that an operator's `Q` and gain mean
//!   what a parametric equaliser's knobs normally mean.

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

    /// A one-pole-shaped second-order low-pass, RBJ cookbook.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn low_pass(cutoff_hz: f32, sample_rate: u32, q: f32) -> Result<Self, BiquadError> {
        let (omega, alpha) = Self::omega_alpha_q(cutoff_hz, sample_rate, q)?;
        let cos_w = omega.cos();
        let b = (1.0 - cos_w) / 2.0;
        Self::normalise(b, 1.0 - cos_w, b, alpha, cos_w)
    }

    /// A second-order high-pass, RBJ cookbook.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn high_pass(cutoff_hz: f32, sample_rate: u32, q: f32) -> Result<Self, BiquadError> {
        let (omega, alpha) = Self::omega_alpha_q(cutoff_hz, sample_rate, q)?;
        let cos_w = omega.cos();
        let b = f64::midpoint(1.0, cos_w);
        Self::normalise(b, -(1.0 + cos_w), b, alpha, cos_w)
    }

    /// A notch (band-reject): unity everywhere except a null at `centre_hz`.
    ///
    /// The filter for a hum, which a high-pass cannot remove without also
    /// removing everything below it. Mains hum at 50 or 60 Hz, and its
    /// harmonics, sit squarely in the range a station's wind filter is already
    /// working in.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn notch(centre_hz: f32, sample_rate: u32, q: f32) -> Result<Self, BiquadError> {
        let (omega, alpha) = Self::omega_alpha_q(centre_hz, sample_rate, q)?;
        let cos_w = omega.cos();
        Self::normalise(1.0, -2.0 * cos_w, 1.0, alpha, cos_w)
    }

    /// A peaking (bell) filter: `gain_db` at `centre_hz`, unity far away.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn peaking(
        centre_hz: f32,
        sample_rate: u32,
        q: f32,
        gain_db: f32,
    ) -> Result<Self, BiquadError> {
        let (omega, alpha) = Self::omega_alpha_q(centre_hz, sample_rate, q)?;
        let cos_w = omega.cos();
        // `A` is the *square root* of the linear gain, which is the cookbook's
        // convention for the shelving and peaking forms and the single easiest
        // thing to get wrong here: using the linear gain directly doubles every
        // boost and cut in decibels.
        let amp = 10.0_f64.powf(f64::from(gain_db) / 40.0);
        let a0 = 1.0 + alpha / amp;
        Ok(Self::from_coefficients(
            alpha.mul_add(amp, 1.0) / a0,
            (-2.0 * cos_w) / a0,
            alpha.mul_add(-amp, 1.0) / a0,
            (-2.0 * cos_w) / a0,
            (1.0 - alpha / amp) / a0,
        ))
    }

    /// A low shelf: `gain_db` below `corner_hz`, unity above.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn low_shelf(
        corner_hz: f32,
        sample_rate: u32,
        q: f32,
        gain_db: f32,
    ) -> Result<Self, BiquadError> {
        Self::shelf(corner_hz, sample_rate, q, gain_db, true)
    }

    /// A high shelf: `gain_db` above `corner_hz`, unity below.
    ///
    /// # Errors
    ///
    /// As [`Self::bandpass`].
    pub fn high_shelf(
        corner_hz: f32,
        sample_rate: u32,
        q: f32,
        gain_db: f32,
    ) -> Result<Self, BiquadError> {
        Self::shelf(corner_hz, sample_rate, q, gain_db, false)
    }

    /// The two shelves differ only in three signs, so they share a body.
    fn shelf(
        corner_hz: f32,
        sample_rate: u32,
        q: f32,
        gain_db: f32,
        low: bool,
    ) -> Result<Self, BiquadError> {
        let (omega, _) = Self::omega_alpha_q(corner_hz, sample_rate, q)?;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let amp = 10.0_f64.powf(f64::from(gain_db) / 40.0);
        // The shelf form takes `alpha` from `S`, a slope parameter, rather
        // than from `Q` directly; `S = 1` is the steepest monotonic shelf and
        // corresponds to this expression. Exposed as `q` because that is the
        // control a parametric equaliser gives an operator.
        let alpha = sin_w / 2.0
            * (amp + 1.0 / amp)
                .mul_add(1.0 / f64::from(q) - 1.0, 2.0)
                .max(0.0)
                .sqrt();
        let two_sqrt_a_alpha = 2.0 * amp.sqrt() * alpha;
        let (ap1, am1) = (amp + 1.0, amp - 1.0);

        let (b0, b1, b2, a0, a1, a2) = if low {
            (
                amp * (am1.mul_add(-cos_w, ap1) + two_sqrt_a_alpha),
                2.0 * amp * ap1.mul_add(-cos_w, am1),
                amp * (am1.mul_add(-cos_w, ap1) - two_sqrt_a_alpha),
                am1.mul_add(cos_w, ap1) + two_sqrt_a_alpha,
                -2.0 * ap1.mul_add(cos_w, am1),
                am1.mul_add(cos_w, ap1) - two_sqrt_a_alpha,
            )
        } else {
            (
                amp * (am1.mul_add(cos_w, ap1) + two_sqrt_a_alpha),
                -2.0 * amp * ap1.mul_add(cos_w, am1),
                amp * (am1.mul_add(cos_w, ap1) - two_sqrt_a_alpha),
                am1.mul_add(-cos_w, ap1) + two_sqrt_a_alpha,
                2.0 * ap1.mul_add(-cos_w, am1),
                am1.mul_add(-cos_w, ap1) - two_sqrt_a_alpha,
            )
        };
        if a0 == 0.0 {
            return Err(BiquadError::Unstable);
        }
        let filter = Self::from_coefficients(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
        filter.checked()
    }

    /// `ω₀` and the cookbook `alpha` for a `Q`-parameterised design.
    fn omega_alpha_q(hz: f32, sample_rate: u32, q: f32) -> Result<(f64, f64), BiquadError> {
        if hz <= 0.0 || sample_rate == 0 || q <= 0.0 {
            return Err(BiquadError::NonPositiveParameter);
        }
        let fs = f64::from(sample_rate);
        let f0 = f64::from(hz);
        if f0 * 2.0 >= fs {
            return Err(BiquadError::AboveNyquist);
        }
        let omega = 2.0 * std::f64::consts::PI * f0 / fs;
        Ok((omega, omega.sin() / (2.0 * f64::from(q))))
    }

    /// Divide an unnormalised numerator by `a0 = 1 + alpha` and check stability.
    fn normalise(b0: f64, b1: f64, b2: f64, alpha: f64, cos_w: f64) -> Result<Self, BiquadError> {
        let a0 = 1.0 + alpha;
        Self::from_coefficients(
            b0 / a0,
            b1 / a0,
            b2 / a0,
            (-2.0 * cos_w) / a0,
            (1.0 - alpha) / a0,
        )
        .checked()
    }

    /// A section from already-normalised coefficients, with zero state.
    const fn from_coefficients(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// Jury's stability test for a second-order section: both poles lie inside
    /// the unit circle exactly when `|a2| < 1` and `|a1| < 1 + a2`.
    fn checked(self) -> Result<Self, BiquadError> {
        if !self.a1.is_finite()
            || !self.a2.is_finite()
            || self.a2.abs() >= 1.0
            || self.a1.abs() >= 1.0 + self.a2
        {
            return Err(BiquadError::Unstable);
        }
        Ok(self)
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

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Magnitude response in decibels.
    fn db_at(f: &Biquad, hz: f32) -> f64 {
        20.0 * f.magnitude_at(hz, SR).log10()
    }

    /// A low-pass passes DC, is 3 dB down at its corner, and rolls off.
    ///
    /// Checked against the analytic response rather than by running audio
    /// through it, so a failure points at the coefficients.
    #[test]
    fn a_low_pass_passes_below_and_stops_above() {
        let f = Biquad::low_pass(1000.0, SR, std::f32::consts::FRAC_1_SQRT_2).expect("designs");
        assert!(db_at(&f, 10.0).abs() < 0.1, "DC: {:.2} dB", db_at(&f, 10.0));
        assert!(
            (db_at(&f, 1000.0) + 3.01).abs() < 0.2,
            "corner: {:.2} dB, expected -3.01",
            db_at(&f, 1000.0)
        );
        assert!(
            db_at(&f, 4000.0) < -20.0,
            "two octaves up: {:.2} dB, expected steep roll-off",
            db_at(&f, 4000.0)
        );
    }

    /// A high-pass is the mirror image, and the pair is what makes either
    /// meaningful: a filter that attenuated everything would satisfy half of
    /// each test above on its own.
    #[test]
    fn a_high_pass_stops_below_and_passes_above() {
        let f = Biquad::high_pass(1000.0, SR, std::f32::consts::FRAC_1_SQRT_2).expect("designs");
        assert!(
            db_at(&f, 10_000.0).abs() < 0.1,
            "well above: {:.2} dB",
            db_at(&f, 10_000.0)
        );
        assert!(
            (db_at(&f, 1000.0) + 3.01).abs() < 0.2,
            "corner: {:.2} dB",
            db_at(&f, 1000.0)
        );
        assert!(
            db_at(&f, 250.0) < -20.0,
            "two octaves down: {:.2} dB",
            db_at(&f, 250.0)
        );
    }

    /// A notch nulls its centre and leaves everything else alone.
    ///
    /// The filter for mains hum, which a high-pass cannot remove without also
    /// removing every bittern boom and wood pigeon call below it.
    #[test]
    fn a_notch_nulls_its_centre_and_spares_the_rest() {
        let f = Biquad::notch(50.0, SR, 20.0).expect("designs");
        assert!(
            db_at(&f, 50.0) < -30.0,
            "at the notch: {:.2} dB, expected a deep null",
            db_at(&f, 50.0)
        );
        for hz in [25.0_f32, 100.0, 1000.0, 8000.0] {
            assert!(
                db_at(&f, hz).abs() < 1.0,
                "at {hz} Hz the notch should be transparent, got {:.2} dB",
                db_at(&f, hz)
            );
        }
    }

    /// A peaking filter's gain at its centre is the gain that was asked for.
    ///
    /// The `A = 10^(dB/40)` convention is the single easiest thing to get
    /// wrong in the cookbook: using the linear gain `10^(dB/20)` instead
    /// doubles every boost and cut, so a requested +6 dB arrives as +12.
    #[test]
    fn a_peaking_filter_delivers_the_gain_it_was_asked_for() {
        for gain in [-12.0_f32, -6.0, 3.0, 6.0, 12.0] {
            let f = Biquad::peaking(2000.0, SR, 1.0, gain).expect("designs");
            let got = db_at(&f, 2000.0);
            assert!(
                (got - f64::from(gain)).abs() < 0.1,
                "asked for {gain:+.1} dB at the centre, got {got:+.2} dB"
            );
            assert!(
                db_at(&f, 50.0).abs() < 0.5 && db_at(&f, 18_000.0).abs() < 0.5,
                "a bell must be transparent far from its centre: {:.2} / {:.2} dB",
                db_at(&f, 50.0),
                db_at(&f, 18_000.0)
            );
        }
    }

    /// `Q` sets a bell's width, and a higher `Q` is narrower.
    ///
    /// Without this, a peaking filter that ignored `Q` entirely would pass
    /// every assertion above.
    #[test]
    fn a_higher_q_makes_a_narrower_bell() {
        let wide = Biquad::peaking(2000.0, SR, 0.5, 12.0).expect("designs");
        let narrow = Biquad::peaking(2000.0, SR, 8.0, 12.0).expect("designs");
        assert!(
            (db_at(&wide, 2000.0) - db_at(&narrow, 2000.0)).abs() < 0.1,
            "both must reach +12 dB at the centre"
        );
        assert!(
            db_at(&wide, 4000.0) > db_at(&narrow, 4000.0) + 3.0,
            "an octave up the wide bell should still be lifted ({:.2} dB) and the narrow one \
             should have returned to unity ({:.2} dB)",
            db_at(&wide, 4000.0),
            db_at(&narrow, 4000.0)
        );
    }

    /// A low shelf lifts or cuts everything below its corner and nothing above.
    #[test]
    fn a_low_shelf_acts_below_its_corner_only() {
        let f = Biquad::low_shelf(300.0, SR, 0.707, -9.0).expect("designs");
        assert!(
            (db_at(&f, 20.0) + 9.0).abs() < 0.5,
            "well below the corner the shelf should reach -9 dB, got {:.2}",
            db_at(&f, 20.0)
        );
        assert!(
            db_at(&f, 8000.0).abs() < 0.5,
            "well above it should be transparent, got {:.2} dB",
            db_at(&f, 8000.0)
        );
    }

    /// And a high shelf is its mirror. Asserted as a pair for the same reason
    /// as the low/high pass: either alone is satisfied by a filter that does
    /// nothing in the half being ignored.
    #[test]
    fn a_high_shelf_acts_above_its_corner_only() {
        let f = Biquad::high_shelf(4000.0, SR, 0.707, 6.0).expect("designs");
        assert!(
            (db_at(&f, 18_000.0) - 6.0).abs() < 0.5,
            "well above the corner: {:.2} dB, expected +6",
            db_at(&f, 18_000.0)
        );
        assert!(
            db_at(&f, 100.0).abs() < 0.5,
            "well below it: {:.2} dB, expected transparent",
            db_at(&f, 100.0)
        );
    }

    /// A shelf's corner sits at exactly half its gain, in decibels.
    ///
    /// The defining midpoint of the RBJ shelf parameterisation: a −9 dB shelf
    /// reads −4.50 dB at its corner frequency, whatever `Q` is. Worth pinning
    /// because it is what makes "corner frequency" mean something an operator
    /// can predict — but note it does *not* constrain the slope, which is what
    /// the next test is for.
    #[test]
    fn the_corner_of_a_shelf_is_half_its_gain() {
        for (gain, q) in [
            (-9.0_f32, 0.4_f32),
            (-9.0, 0.707),
            (-9.0, 1.5),
            (6.0, 0.707),
        ] {
            let low = Biquad::low_shelf(300.0, SR, q, gain).expect("designs");
            assert!(
                (db_at(&low, 300.0) - f64::from(gain) / 2.0).abs() < 0.1,
                "low shelf {gain:+.1} dB at Q={q}: corner reads {:.2} dB, expected {:+.2}",
                db_at(&low, 300.0),
                gain / 2.0
            );
            let high = Biquad::high_shelf(4000.0, SR, q, gain).expect("designs");
            assert!(
                (db_at(&high, 4000.0) - f64::from(gain) / 2.0).abs() < 0.1,
                "high shelf {gain:+.1} dB at Q={q}: corner reads {:.2} dB",
                db_at(&high, 4000.0)
            );
        }
    }

    /// `Q` sets how abruptly a shelf transitions.
    ///
    /// Written because a mutant that dropped the gain and `Q` terms from the
    /// shelf's `alpha` — leaving `alpha = sin(ω₀)/2` — passed every other
    /// shelf test here. The asymptotes and the corner midpoint are all
    /// independent of the slope, so nothing constrained it.
    ///
    /// Measured on a −9 dB shelf at 300 Hz, one octave below the corner:
    ///
    /// ```text
    ///   Q = 0.4   -> −6.68 dB   gentle
    ///   Q = 0.707 -> −7.69 dB
    ///   Q = 1.5   -> −9.25 dB   steep, and overshooting
    /// ```
    ///
    /// The overshoot at high `Q` is real and not a defect: the same resonance
    /// a high-`Q` shelf has in any parametric equaliser. At Q = 1.5 this one
    /// reaches −9.25 dB below the corner (past the −9 dB it is heading for)
    /// and +0.25 dB above it.
    #[test]
    fn the_q_of_a_shelf_sets_its_steepness() {
        let at =
            |q: f32, hz: f32| db_at(&Biquad::low_shelf(300.0, SR, q, -9.0).expect("designs"), hz);
        let gentle = at(0.4, 150.0);
        let middle = at(0.707, 150.0);
        let steep = at(1.5, 150.0);

        assert!(
            gentle > middle && middle > steep,
            "an octave below the corner a higher Q must have travelled further towards the \
             shelf gain: Q=0.4 {gentle:.2} dB, Q=0.707 {middle:.2} dB, Q=1.5 {steep:.2} dB"
        );
        assert!(
            gentle - steep > 2.0,
            "the spread across that Q range is only {:.2} dB, so Q barely reaches the design",
            gentle - steep
        );
        assert!(
            at(1.5, 600.0) > 0.0,
            "a Q of 1.5 should overshoot above the corner; got {:.2} dB",
            at(1.5, 600.0)
        );
        assert!(
            at(0.4, 600.0) < 0.0,
            "and a gentle Q should not overshoot; got {:.2} dB",
            at(0.4, 600.0)
        );
    }

    /// A shelf or bell at 0 dB is a wire.
    ///
    /// A station whose operator adds a stage and leaves its gain alone must
    /// hear exactly what it heard before.
    #[test]
    fn zero_gain_is_transparent() {
        for f in [
            Biquad::peaking(2000.0, SR, 1.0, 0.0).expect("peaking"),
            Biquad::low_shelf(300.0, SR, 0.707, 0.0).expect("low shelf"),
            Biquad::high_shelf(4000.0, SR, 0.707, 0.0).expect("high shelf"),
        ] {
            for hz in [20.0_f32, 300.0, 2000.0, 12_000.0] {
                assert!(
                    db_at(&f, hz).abs() < 0.01,
                    "a 0 dB stage moved {hz} Hz by {:.4} dB",
                    db_at(&f, hz)
                );
            }
        }
    }

    /// The analytic response and the actual filtered signal agree.
    ///
    /// `magnitude_at` is used by every test above and by the equaliser's
    /// response curve, and it is derived from the coefficients rather than
    /// measured — so if it were wrong, every one of those tests would be
    /// consistent and wrong together. This is the one that runs audio.
    #[test]
    fn the_analytic_response_matches_the_filtered_signal() {
        for (name, mut f, hz) in [
            (
                "peaking +12 dB",
                Biquad::peaking(2000.0, SR, 1.0, 12.0).expect("designs"),
                2000.0_f32,
            ),
            (
                "low-pass stopband",
                Biquad::low_pass(1000.0, SR, 0.707).expect("designs"),
                4000.0,
            ),
            (
                "high shelf +6 dB",
                Biquad::high_shelf(4000.0, SR, 0.707, 6.0).expect("designs"),
                12_000.0,
            ),
        ] {
            // Two seconds of tone; the first second is discarded so the
            // measurement is of the steady state, not the transient.
            let n = (SR * 2) as usize;
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                out.push(f.process((std::f32::consts::TAU * hz * t).sin()));
            }
            let tail = &out[n / 2..];
            let rms = (tail.iter().map(|s| s * s).sum::<f64>()
                / f64::from(u32::try_from(tail.len()).expect("tail fits")))
            .sqrt();
            // A unit sine has an RMS of 1/√2, so the gain is rms·√2.
            let measured_db = 20.0 * (rms * std::f64::consts::SQRT_2).log10();
            let predicted_db = db_at(&f, hz);
            assert!(
                (measured_db - predicted_db).abs() < 0.2,
                "{name} at {hz} Hz: the response curve says {predicted_db:+.2} dB and the \
                 filtered signal measures {measured_db:+.2} dB"
            );
        }
    }

    /// Impossible parameters are refused rather than producing a filter that
    /// misbehaves later.
    #[test]
    fn impossible_parameters_are_refused() {
        let err = |r: Result<Biquad, BiquadError>| r.err();
        assert_eq!(
            err(Biquad::low_pass(0.0, SR, 1.0)),
            Some(BiquadError::NonPositiveParameter)
        );
        assert_eq!(
            err(Biquad::peaking(1000.0, SR, 0.0, 3.0)),
            Some(BiquadError::NonPositiveParameter)
        );
        assert_eq!(
            err(Biquad::high_pass(1000.0, 0, 1.0)),
            Some(BiquadError::NonPositiveParameter)
        );
        assert_eq!(
            err(Biquad::notch(30_000.0, SR, 1.0)),
            Some(BiquadError::AboveNyquist),
            "a filter at or above Nyquist folds instead of filtering"
        );
        assert_eq!(
            err(Biquad::low_pass(24_000.0, SR, 1.0)),
            Some(BiquadError::AboveNyquist),
            "exactly Nyquist is out too"
        );
    }
}
