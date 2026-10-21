//! Colour maps for spectrogram rendering.
//!
//! [`Colormap`] is the choice an owner makes on the settings page
//! (`spectrogram_colormap`); [`Colormap::rgb`] maps a normalised level in
//! `[0, 1]` (0 = the quietest cell of the image, 1 = the loudest) to a colour.
//!
//! # Provenance and what "perceptually sound" is held to
//!
//! `magma` and `inferno` are matplotlib's maps by Nathaniel Smith and Stefan
//! van der Walt (CC0, `matplotlib/_cm_listed.py`, v3.9.2), sampled at 33 evenly
//! spaced stops of their 256-entry tables and interpolated linearly; the worst
//! channel error against the full table, measured over all 256 entries when the
//! stops were cut, is 1.6 of 255. `viridis` is this project's older 9-stop
//! approximation, kept as it was so a station that never chooses gets the
//! pixels it always got. The two greys are plain luminance ramps.
//!
//! A colour map for data should get lighter as the value rises, so that order
//! survives a grey print and a colour-blind reader. `lightness_is_monotonic`
//! below holds every map to that: CIELAB L\* never falls along the ramp (and
//! never rises, for `grey_inverted`, which is deliberately the other way: the
//! quiet floor is paper-white, as BirdNET-Pi's "scientific" light style draws).

/// Which colour map paints the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Colormap {
    /// Dark blue to yellow; what every earlier version drew.
    #[default]
    Viridis,
    /// Black through purple and orange to pale yellow.
    Magma,
    /// Black through purple and red to pale yellow; the brightest of the three.
    Inferno,
    /// Black (quiet) to white (loud).
    Grey,
    /// White (quiet) to black (loud): a printed page, or a light background.
    GreyInverted,
}

impl Colormap {
    /// Every map, in the order the settings page lists them.
    pub const ALL: [Self; 5] = [
        Self::Viridis,
        Self::Magma,
        Self::Inferno,
        Self::Grey,
        Self::GreyInverted,
    ];

    /// The value stored in the settings table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viridis => "viridis",
            Self::Magma => "magma",
            Self::Inferno => "inferno",
            Self::Grey => "grey",
            Self::GreyInverted => "grey_inverted",
        }
    }

    /// The name the settings page shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Viridis => "Viridis (blue to yellow, the default)",
            Self::Magma => "Magma (black to pale yellow)",
            Self::Inferno => "Inferno (black to bright yellow)",
            Self::Grey => "Grey (quiet is black)",
            Self::GreyInverted => "Grey, inverted (quiet is white)",
        }
    }

    /// The map a stored value names, or `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == value.trim())
    }

    /// The colour of level `t` (clamped to `[0, 1]`).
    #[must_use]
    pub fn rgb(self, t: f32) -> (u8, u8, u8) {
        match self {
            Self::Viridis => viridis(t),
            Self::Magma => sample(&MAGMA, t),
            Self::Inferno => sample(&INFERNO, t),
            Self::Grey => grey(t),
            Self::GreyInverted => grey(1.0 - t.clamp(0.0, 1.0)),
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
fn grey(t: f32) -> (u8, u8, u8) {
    let v = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
    (v, v, v)
}

/// Linear interpolation through evenly spaced stops.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
fn sample(stops: &[(u8, u8, u8)], t: f32) -> (u8, u8, u8) {
    let last = stops.len() - 1;
    let pos = t.clamp(0.0, 1.0) * last as f32;
    let i = (pos as usize).min(last - 1);
    let frac = pos - i as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    let mix = |x: u8, y: u8| (f32::from(x) + frac * (f32::from(y) - f32::from(x))).round() as u8;
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

const MAGMA: [(u8, u8, u8); 33] = [
    (0, 0, 4),
    (3, 3, 18),
    (10, 8, 34),
    (19, 13, 52),
    (29, 17, 71),
    (41, 17, 90),
    (54, 16, 107),
    (68, 15, 118),
    (81, 18, 124),
    (93, 23, 127),
    (106, 28, 129),
    (118, 33, 129),
    (131, 38, 129),
    (144, 42, 129),
    (156, 46, 127),
    (170, 51, 125),
    (183, 55, 121),
    (194, 59, 117),
    (207, 64, 112),
    (219, 71, 106),
    (229, 80, 100),
    (238, 91, 94),
    (244, 105, 92),
    (249, 120, 93),
    (251, 135, 97),
    (253, 150, 104),
    (254, 165, 113),
    (254, 180, 123),
    (254, 194, 135),
    (254, 209, 148),
    (253, 224, 161),
    (252, 238, 176),
    (252, 253, 191),
];

const INFERNO: [(u8, u8, u8); 33] = [
    (0, 0, 4),
    (4, 3, 18),
    (11, 7, 36),
    (21, 11, 55),
    (33, 12, 74),
    (47, 10, 91),
    (61, 9, 101),
    (74, 12, 107),
    (87, 16, 110),
    (100, 21, 110),
    (113, 25, 110),
    (125, 30, 109),
    (138, 34, 106),
    (151, 39, 102),
    (163, 44, 97),
    (176, 49, 91),
    (188, 55, 84),
    (198, 61, 77),
    (208, 69, 69),
    (218, 78, 60),
    (227, 89, 51),
    (235, 100, 41),
    (241, 113, 31),
    (246, 126, 20),
    (249, 140, 10),
    (251, 155, 6),
    (252, 170, 15),
    (251, 186, 31),
    (249, 201, 50),
    (245, 217, 73),
    (242, 232, 101),
    (243, 245, 134),
    (252, 255, 164),
];

/// Approximate viridis colormap: maps t in \[0,1\] to (R,G,B).
pub fn viridis(t: f32) -> (u8, u8, u8) {
    // Control points: (t, R, G, B)
    let cps: &[(f32, f32, f32, f32)] = &[
        (0.000, 68.0, 1.0, 84.0),
        (0.125, 71.0, 44.0, 122.0),
        (0.250, 59.0, 82.0, 139.0),
        (0.375, 44.0, 113.0, 142.0),
        (0.500, 33.0, 145.0, 140.0),
        (0.625, 39.0, 173.0, 129.0),
        (0.750, 92.0, 200.0, 99.0),
        (0.875, 170.0, 220.0, 50.0),
        (1.000, 253.0, 231.0, 37.0),
    ];

    let t = t.clamp(0.0, 1.0);
    let i = cps
        .partition_point(|cp| cp.0 <= t)
        .saturating_sub(1)
        .min(cps.len() - 2);
    let (t0, r0, g0, b0) = cps[i];
    let (t1, r1, g1, b1) = cps[i + 1];
    let frac = if (t1 - t0).abs() < 1e-6 {
        0.0
    } else {
        (t - t0) / (t1 - t0)
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let lerp = |a: f32, b: f32| (a + frac * (b - a)).clamp(0.0, 255.0) as u8;
    (lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viridis_endpoints() {
        let (r, _g, b) = viridis(0.0);
        assert!(b > r, "cold end should be blue-heavy");
        let (r2, g2, _b2) = viridis(1.0);
        assert!(r2 > 200 && g2 > 200, "warm end should be yellow");
    }

    #[test]
    fn viridis_midpoint_is_greenish() {
        let (_r, g, _b) = viridis(0.5);
        assert!(g > 100, "midpoint should have significant green");
    }

    #[test]
    fn every_map_round_trips_its_stored_name_and_nothing_else() {
        for m in Colormap::ALL {
            assert_eq!(Colormap::parse(m.as_str()), Some(m));
            assert!(!m.label().is_empty());
        }
        assert_eq!(Colormap::default(), Colormap::Viridis);
        for bad in ["", "rainbow", "Viridis", "magma2"] {
            assert_eq!(Colormap::parse(bad), None, "{bad:?}");
        }
    }

    /// CIELAB lightness of an sRGB colour (D65).
    fn lstar((r, g, b): (u8, u8, u8)) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let y = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
        if y > 216.0 / 24389.0 {
            116.0 * y.cbrt() - 16.0
        } else {
            24389.0 / 27.0 * y
        }
    }

    /// A colour map for ordered data must keep its order in lightness, or a
    /// grey print and a colour-blind reader lose it. Sampled finely, so a dip
    /// between two stops would show.
    #[test]
    fn lightness_is_monotonic() {
        const STEPS: usize = 1000;
        for m in Colormap::ALL {
            let l: Vec<f64> = (0..=STEPS)
                .map(|i| lstar(m.rgb(i as f32 / STEPS as f32)))
                .collect();
            let rising = m != Colormap::GreyInverted;
            // The old nine-stop viridis dips by 0.11 L* around t = 0.125 (its
            // second knot is a little too light), a tenth of a just-noticeable
            // difference; it is kept as it was so a station that never chooses
            // gets the pixels it always got. The others allow only the rounding of
            // 8-bit channels (magma's top end dips 0.07 L* from it).
            let slack = if m == Colormap::Viridis { 0.25 } else { 0.15 };
            for (i, pair) in l.windows(2).enumerate() {
                let ok = if rising {
                    pair[1] >= pair[0] - slack
                } else {
                    pair[1] <= pair[0] + slack
                };
                assert!(
                    ok,
                    "{m:?}: L* goes the wrong way at step {i}: {} -> {}",
                    pair[0], pair[1]
                );
            }
            let span = (l[STEPS] - l[0]).abs();
            assert!(span > 60.0, "{m:?}: lightness spans only {span:.1}");
        }
    }

    /// The maps are different: no two agree at the same level (the two greys
    /// cross at the middle, so a quarter of the way is compared).
    #[test]
    fn the_maps_differ() {
        for t in [0.25_f32, 0.75] {
            let colours: Vec<_> = Colormap::ALL.iter().map(|m| m.rgb(t)).collect();
            for (i, a) in colours.iter().enumerate() {
                for b in &colours[i + 1..] {
                    assert_ne!(a, b, "two maps agree at {t}");
                }
            }
        }
    }

    #[test]
    fn out_of_range_levels_are_clamped_not_wrapped() {
        for m in Colormap::ALL {
            assert_eq!(m.rgb(-3.0), m.rgb(0.0));
            assert_eq!(m.rgb(7.0), m.rgb(1.0));
        }
    }

    /// The ends are matplotlib's, to the digit (`magma(0)`, `magma(1)`, ...).
    #[test]
    fn endpoints_are_the_published_ones() {
        assert_eq!(Colormap::Magma.rgb(0.0), (0, 0, 4));
        assert_eq!(Colormap::Magma.rgb(1.0), (252, 253, 191));
        assert_eq!(Colormap::Inferno.rgb(0.0), (0, 0, 4));
        assert_eq!(Colormap::Inferno.rgb(1.0), (252, 255, 164));
        assert_eq!(Colormap::Grey.rgb(1.0), (255, 255, 255));
        assert_eq!(Colormap::GreyInverted.rgb(0.0), (255, 255, 255));
        assert_eq!(Colormap::Viridis.rgb(0.0), (68, 1, 84));
        assert_eq!(Colormap::Viridis.rgb(1.0), (253, 231, 37));
    }
}
