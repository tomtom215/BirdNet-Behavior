//! Uncertainty: the interval a share or rate is drawn with, and the one rule for
//! "too few to say" (CHT-23).
//!
//! A share or a rate is drawn with its `n` and its interval, or it is not drawn
//! (honesty rule 10 in `docs/design/charts.md`). Three pieces, the same on every
//! figure that uses them:
//!
//! * [`Interval`]: the arithmetic. A share of trials takes the Wilson score
//!   interval ([`Interval::share`], from `birdnet_behavioral::interval::wilson`);
//!   a rate of events over an exposure takes the exact Poisson interval
//!   ([`Interval::rate`], from the one already written for the effort-corrected
//!   abundance, `birdnet_behavioral::phenology::abundance::poisson_interval_exact`).
//! * [`too_few`]: whether `n` is too small to draw a value at all, from one
//!   documented threshold, [`TOO_FEW_BELOW`].
//! * The marks: [`whisker`] draws an interval quietly behind its estimate, and
//!   [`few_mark`] draws the muted "too few" state; [`interval_words`] and
//!   [`too_few_words`] are what a tooltip, a table cell and a screen reader say
//!   for each, so the picture and its text never disagree.
//!
//! The marks are classes (`ck-ci`, `ck-est`, `ck-few`), painted by the block at the end
//! of `static/css/app.css` from the chart tokens; nothing here names a colour.

use std::fmt::Write as _;

/// Below this many trials a share is "too few to say": nothing is drawn as a value.
///
/// Three, because one or two days cannot tell "never" from "always": the 95 % Wilson
/// interval of 0 of 2 reaches 66 % and that of 2 of 2 falls to 34 %, so either reading
/// is consistent with a bird heard on most days or on few. At three the interval of 0 of
/// 3 stops at 56 % and that of 3 of 3 starts at 44 %: still wide, and drawn as wide, but
/// no longer consistent with everything. The interval carries the rest of the
/// uncertainty; this rule only removes the cases where the interval is the whole answer.
/// The chart review (`docs/audit-2026-11/09-chart-review.md`, CHT-16 and G-01) proposed
/// the same floor for the reporting-frequency figure.
pub const TOO_FEW_BELOW: u32 = 3;

/// Whether `n` trials are too few to say anything: below [`TOO_FEW_BELOW`].
#[must_use]
pub const fn too_few(n: u32) -> bool {
    n < TOO_FEW_BELOW
}

/// A two-sided 95 % interval, `lo..=hi`, in the units of the value it surrounds (a share
/// in `0..=1`, or a rate per unit of exposure).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// The lower bound.
    pub lo: f64,
    /// The upper bound.
    pub hi: f64,
}

impl Interval {
    /// The Wilson interval of `k` of `n`; `None` for no trials (or `k > n`), which is
    /// "not known", never an interval around zero.
    #[must_use]
    pub fn share(k: u32, n: u32) -> Option<Self> {
        birdnet_behavioral::interval::wilson(k, n).map(|(lo, hi)| Self { lo, hi })
    }

    /// The exact Poisson interval of `k` events over `exposure` units (hours of
    /// listening, say), as a rate per unit; `None` for no exposure.
    #[must_use]
    pub fn rate(k: u32, exposure: f64) -> Option<Self> {
        if !(exposure.is_finite() && exposure > 0.0) {
            return None;
        }
        let (lo, hi) = birdnet_behavioral::phenology::abundance::poisson_interval_exact(k);
        Some(Self {
            lo: lo / exposure,
            hi: hi / exposure,
        })
    }
}

/// A share as a whole percentage, as every figure prints it.
#[must_use]
pub fn percent(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
}

/// How an interval on a share is said, everywhere it is said: `95% interval 16% to 75%`.
#[must_use]
pub fn interval_words(i: Interval) -> String {
    format!("95% interval {} to {}", percent(i.lo), percent(i.hi))
}

/// How "too few" is said, with the count and its unit: `2 listened days: too few to say`.
/// `unit` is singular and plural, `("listened day", "listened days")`.
#[must_use]
pub fn too_few_words(n: u32, unit: (&str, &str)) -> String {
    let u = if n == 1 { unit.0 } else { unit.1 };
    format!("{n} {u}: too few to say")
}

/// The interval mark: a quiet vertical bar from `y_lo` to `y_hi`, centred on `cx`.
///
/// The bounds are viewBox units, in either order; the bar is `width` wide with rounded ends.
/// It is drawn under the estimate's mark and carries the mark's index so the two light
/// together. Decorative to a screen reader: what it says is in the mark's `data-tip` and the
/// table ([`interval_words`]).
#[must_use]
pub fn whisker(cx: f64, y_lo: f64, y_hi: f64, width: f64, index: usize) -> String {
    let (top, bottom) = if y_lo <= y_hi {
        (y_lo, y_hi)
    } else {
        (y_hi, y_lo)
    };
    // A zero-height interval is still drawn as a cap, so "certain" is never invisible.
    let height = (bottom - top).max(width.min(2.0));
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<rect class="ck-ci" x="{x0:.1}" y="{top:.1}" width="{width:.1}" height="{height:.1}" rx="{r:.1}" data-ck-i="{index}"/>"#,
        x0 = cx - width / 2.0,
        r = width / 2.0,
    );
    out
}

/// The estimate's mark: a dot of radius `r` at `(x, y)`, index `i`.
#[must_use]
pub fn estimate(x: f64, y: f64, r: f64, i: usize) -> String {
    format!(r#"<circle class="ck-est" cx="{x:.1}" cy="{y:.1}" r="{r:.1}" data-ck-i="{i}"/>"#)
}

/// The "too few to say" mark: a small hollow ring of radius `r` at `(x, y)`, index `i`.
///
/// It is in the muted ink, and sits where a value would but on no scale (the strip under
/// the plot's baseline), so it cannot be read as a value of zero.
#[must_use]
pub fn few_mark(x: f64, y: f64, r: f64, i: usize) -> String {
    format!(r#"<circle class="ck-few" cx="{x:.1}" cy="{y:.1}" r="{r:.1}" data-ck-i="{i}"/>"#)
}

/// The key every figure that draws these marks prints under itself, so the three states
/// are named the same way everywhere. `unit` is the plural of what `n` counts.
#[must_use]
pub fn key(unit_plural: &str) -> String {
    format!(
        r#"<p class="bnb-meta ck-unc-key"><span class="ck-unc-item"><span class="ck-unc-sw ck-unc-sw-est" aria-hidden="true"></span>share heard</span> <span class="ck-unc-item"><span class="ck-unc-sw ck-unc-sw-ci" aria-hidden="true"></span>95% interval</span> <span class="ck-unc-item"><span class="ck-unc-sw ck-unc-sw-few" aria-hidden="true"></span>fewer than {TOO_FEW_BELOW} {unit}: too few to say</span></p>"#,
        unit = super::escape_html(unit_plural),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 5e-7
    }

    /// The Wilson bounds of 3 of 7, worked in `birdnet_behavioral::interval`'s tests:
    /// (0.1582198, 0.7495416), said as whole percentages.
    #[test]
    fn a_share_takes_the_wilson_interval_and_says_it_in_percent() {
        let i = Interval::share(3, 7).expect("seven trials");
        assert!(near(i.lo, 0.158_219_8) && near(i.hi, 0.749_541_6), "{i:?}");
        assert_eq!(interval_words(i), "95% interval 16% to 75%");
        assert_eq!(Interval::share(0, 0), None, "no trials is no interval");
    }

    /// A rate reuses the exact Poisson interval. Closed forms, worked by hand: for 0
    /// events the upper bound on the mean is −ln(0.025) = 3.6888795; for 1 event the
    /// lower bound is −ln(0.975) = 0.0253178, and the upper solves
    /// e^(−μ)(1 + μ) = 0.025, μ = 5.5716434 (bisected on a calculator). Over 10 hours
    /// each is a tenth of that.
    #[test]
    fn a_rate_reuses_the_exact_poisson_interval_over_its_exposure() {
        let zero = Interval::rate(0, 10.0).expect("exposure");
        assert!(near(zero.lo, 0.0) && near(zero.hi, 0.368_888_0), "{zero:?}");
        let one = Interval::rate(1, 10.0).expect("exposure");
        assert!(
            near(one.lo, 0.002_531_8) && near(one.hi, 0.557_164_3),
            "{one:?}"
        );
        assert_eq!(Interval::rate(4, 0.0), None, "no listening is no rate");
    }

    /// The threshold and its counterpart: two is too few, three is not.
    #[test]
    fn too_few_is_below_three_and_three_is_enough() {
        assert!(too_few(0) && too_few(1) && too_few(2));
        assert!(!too_few(3) && !too_few(7));
        assert_eq!(
            too_few_words(2, ("listened day", "listened days")),
            "2 listened days: too few to say"
        );
        assert_eq!(
            too_few_words(1, ("listened day", "listened days")),
            "1 listened day: too few to say"
        );
    }

    /// The interval mark spans exactly its bounds whichever way round they come, and a
    /// zero-width interval is still a visible cap.
    #[test]
    fn the_whisker_spans_its_bounds_and_never_vanishes() {
        let w = whisker(100.0, 80.0, 20.0, 4.0, 3);
        assert!(
            w.contains(r#"x="98.0" y="20.0" width="4.0" height="60.0""#),
            "{w}"
        );
        assert!(w.contains(r#"data-ck-i="3""#) && w.contains("ck-ci"), "{w}");
        let cap = whisker(100.0, 50.0, 50.0, 4.0, 0);
        assert!(cap.contains(r#"height="2.0""#), "{cap}");
    }
}
