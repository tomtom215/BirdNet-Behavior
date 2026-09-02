//! The ISO 266 third-octave centre frequencies, and the A-weighting curve.

/// The 30 preferred third-octave centre frequencies of ISO 266, in hertz,
/// spanning 25 Hz to 20 kHz.
///
/// These are the *preferred* (rounded) numbers the standard publishes. They
/// are **labels**: every acoustics instrument reports "31.5 Hz", and a reading
/// keyed on anything else is harder to compare. The band a label names is
/// defined by the exact base-10 series `1000 · 10^(n/10)` — see
/// [`exact_centre_hz`] — and that is what the filter and the weighting curve
/// are built from.
///
/// The distinction is worth 0.16 dB, which sounds like nothing and is not.
/// Evaluating the A-weighting curve at the rounded labels disagrees with the
/// standard'"'"'s own published A-weighting table by up to 0.157 dB (worst case
/// the 160 Hz band, whose exact centre is 158.489 Hz); evaluating it at the
/// exact centres agrees to within 0.050 dB, which is the table'"'"'s own rounding.
/// Measured, not assumed — `a_weighting_matches_the_published_table` pins both
/// halves so this comment cannot quietly become false.
pub const CENTRE_FREQUENCIES_HZ: [f32; 30] = [
    25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
    630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0,
    10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// Exponent, in the base-10 series, of the first entry of
/// [`CENTRE_FREQUENCIES_HZ`].
///
/// `1000 · 10^(-16/10)` is 25.119 Hz, which the standard labels 25 Hz.
const LOWEST_EXPONENT: i32 = -16;

/// The exact centre frequency of the band whose label is
/// `CENTRE_FREQUENCIES_HZ[index]`.
///
/// IEC 61260 defines third-octave bands by the base-10 ratio, so the exact
/// centre of the *n*-th band is `1000 · 10^(n/10)`. This is the frequency the
/// filter is designed at and the frequency the weighting curve is evaluated
/// at; the rounded value is only ever a name.
///
/// # Panics
///
/// Never for `index` in range; an out-of-range index is a programming error
/// and panics rather than silently returning a frequency for a band that does
/// not exist.
#[must_use]
pub fn exact_centre_hz(index: usize) -> f32 {
    assert!(
        index < CENTRE_FREQUENCIES_HZ.len(),
        "band index {index} is out of range"
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let n = index as i32 + LOWEST_EXPONENT;
    #[allow(clippy::cast_precision_loss)]
    let exponent = n as f32 / 10.0;
    1000.0 * 10.0_f32.powf(exponent)
}

/// Half-bandwidth ratio for a third-octave band: `2^(1/6)`.
///
/// A third-octave band spans a frequency *ratio* of `2^(1/3)`, so its edges sit
/// at the centre times and divided by the square root of that.
pub const THIRD_OCTAVE_EDGE_RATIO: f32 = 1.122_462_1; // 2^(1/6)

/// The lower and upper edge of the third-octave band centred on `centre_hz`.
#[must_use]
pub fn band_edges(centre_hz: f32) -> (f32, f32) {
    (
        centre_hz / THIRD_OCTAVE_EDGE_RATIO,
        centre_hz * THIRD_OCTAVE_EDGE_RATIO,
    )
}

/// Quality factor of an ideal third-octave bandpass: `centre / (upper - lower)`.
///
/// Constant across bands — that is what "constant relative bandwidth" means —
/// and works out at about 4.318.
#[must_use]
pub fn third_octave_q(centre_hz: f32) -> f32 {
    let (lo, hi) = band_edges(centre_hz);
    centre_hz / (hi - lo)
}

/// Pole frequencies of the IEC 61672-1 A-weighting transfer function, in hertz.
///
/// Two real poles at 20.6 Hz and 12194 Hz set the overall band, and the pair at
/// 107.7 Hz and 737.9 Hz shapes the mid-range rise. These are the standard's
/// own values, not a fit.
const A_POLE_1: f64 = 20.598_997;
/// See [`A_POLE_1`].
const A_POLE_2: f64 = 107.652_65;
/// See [`A_POLE_1`].
const A_POLE_3: f64 = 737.862_23;
/// See [`A_POLE_1`].
const A_POLE_4: f64 = 12_194.217;

/// Offset that makes the A-weighting exactly 0 dB at 1 kHz.
const A_NORMALISATION_DB: f64 = 2.0;

/// The A-weighting offset at `hz`, in decibels, per IEC 61672-1.
///
/// A-weighting approximates the ear's sensitivity at moderate levels: it
/// discards most of what is below 500 Hz and adds a little around 2–4 kHz. It
/// is the weighting behind every "dB(A)" figure quoted in an environmental
/// noise measurement, which is why a soundscape series that does not carry it
/// cannot be compared with anything published.
///
/// Computed from the standard's transfer function rather than interpolated
/// from its published table of third-octave values:
///
/// ```text
///                        12194² · f⁴
/// R_A(f) = ───────────────────────────────────────────────────────
///          (f² + 20.6²) · √((f² + 107.7²)(f² + 737.9²)) · (f² + 12194²)
///
/// A(f) = 20·log₁₀(R_A(f)) + 2.00
/// ```
///
/// The `+2.00` is the normalisation that makes A(1 kHz) exactly 0 dB. Using the
/// formula means the curve is right at any frequency, including the band edges
/// and any centre frequency added later — and [`super::tests`] pins it against
/// the standard's own tabulated values so a transcription error in the
/// constants above cannot pass unnoticed.
#[must_use]
pub fn a_weighting_db(hz: f32) -> f32 {
    let f = f64::from(hz);
    let f2 = f * f;

    let num = A_POLE_4 * A_POLE_4 * f2 * f2;
    let den = f2.mul_add(1.0, A_POLE_1 * A_POLE_1)
        * (f2.mul_add(1.0, A_POLE_2 * A_POLE_2) * f2.mul_add(1.0, A_POLE_3 * A_POLE_3)).sqrt()
        * f2.mul_add(1.0, A_POLE_4 * A_POLE_4);
    if den == 0.0 {
        return f32::NEG_INFINITY;
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        (num / den).log10().mul_add(20.0, A_NORMALISATION_DB) as f32
    }
}

/// A band's label, as it appears in API output and metric series.
///
/// `31.5` and `12.5k` rather than `31.5_Hz` and `12500`: the reading is a
/// series key a person reads off a chart axis, and this is the form an
/// acoustics tool prints. Trailing zeros are dropped, so `25` and not `25.00`,
/// but significant decimals are kept — the first draft of this used one
/// decimal place and turned the 1250 Hz band into `1.2k`, because `{:.1}`
/// rounds 1.25 to even. Two bands in the table need two decimals (`1.25k` and
/// `3.15k`) and neither of them may be rounded away, because the label is the
/// series key: `1.2k` is a band that does not exist, and a chart legend that
/// says it is lying about which band it drew.
#[must_use]
pub fn band_label(centre_hz: f32) -> String {
    let (value, suffix) = if centre_hz >= 1000.0 {
        (centre_hz / 1000.0, "k")
    } else {
        (centre_hz, "")
    };
    let mut text = format!("{value:.2}");
    while text.contains('.') && (text.ends_with('0') || text.ends_with('.')) {
        text.pop();
    }
    text.push_str(suffix);
    text
}
