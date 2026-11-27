//! Confidence intervals for the shares the figures draw.
//!
//! One formula per kind of number, each checked in its tests against values
//! worked by hand:
//!
//! * a **share** of trials (days heard of days listened, anchors that returned
//!   of anchors that could be followed) takes the Wilson score interval,
//!   [`wilson`];
//! * a **rate** of events over an exposure (detections per hour of listening)
//!   takes the exact Poisson (Garwood) interval, which is already written in
//!   [`crate::phenology::abundance::poisson_interval_exact`] and is not
//!   written again here.
//!
//! Why Wilson and not the textbook `p ± 1.96·√(p(1−p)/n)`: at the small `n`
//! a station has per week (seven listened days, often fewer) that Wald
//! interval collapses to zero width at 0 of `n` and `n` of `n`, which reads
//! as certainty exactly where there is least of it, and it runs below 0 and
//! above 1 in between. Wilson's interval stays inside `[0, 1]`, is never zero
//! width for `n > 0`, and its coverage stays near the nominal 95 % for small
//! `n` (Brown, Cai and `DasGupta`, *Statistical Science* 16(2), 2001, compare
//! the two; that comparison is cited from general knowledge and was not
//! re-read for this module).

/// The 97.5th percentile of the standard normal distribution: the `z` of a
/// two-sided 95 % interval, to double precision.
pub const Z95: f64 = 1.959_963_984_540_054;

/// The two-sided 95 % Wilson score interval for `k` successes in `n` trials.
///
/// It is `(lower, upper)`, shares in `0..=1`; `None` when there were no trials
/// (`n == 0`: nothing is known, which is not a share of zero) or the counts
/// are inconsistent (`k > n`).
///
/// With `p = k/n` and `z = Z95`:
///
/// ```text
/// centre = (p + z²/2n) / (1 + z²/n)
/// half   = z / (1 + z²/n) · √( p(1−p)/n + z²/4n² )
/// ```
///
/// and the interval is `centre ± half`, clamped to `[0, 1]` only against
/// floating-point dust at the ends (at `k = 0` the lower bound is exactly 0
/// in real arithmetic, and at `k = n` the upper bound is exactly 1).
#[must_use]
pub fn wilson(k: u32, n: u32) -> Option<(f64, f64)> {
    if n == 0 || k > n {
        return None;
    }
    wilson_real(f64::from(k), f64::from(n))
}

/// [`wilson`] at real-valued counts: `k` successes of an *effective* `n`
/// trials, as a design effect gives them
/// ([`crate::null_model::dependent_share`]). `None` unless `0 <= k <= n` and
/// `n > 0`, all finite.
#[must_use]
pub fn wilson_real(k: f64, n: f64) -> Option<(f64, f64)> {
    if !(k.is_finite() && n.is_finite() && n > 0.0 && (0.0..=n).contains(&k)) {
        return None;
    }
    let p = k / n;
    let z2 = Z95 * Z95;
    let denom = 1.0 + z2 / n;
    let centre = (p + z2 / (2.0 * n)) / denom;
    let half = Z95 / denom * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    Some(((centre - half).max(0.0), (centre + half).min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 5e-7
    }

    /// 3 of 7. Worked step by step, each step checked on a calculator, with
    /// z² = 3.8414588: p = 0.4285714; z²/n = 0.5487798, so 1 + z²/n = 1.5487798;
    /// centre = (0.4285714 + 0.2743899) / 1.5487798 = 0.4538807;
    /// p(1−p)/n = 0.0349854 and z²/4n² = 0.0195993, whose sum 0.0545847 has
    /// root 0.2336337; half = (1.9599640 / 1.5487798) · 0.2336337
    /// = 1.2654891 · 0.2336337 = 0.2956609. So (0.1582198, 0.7495416).
    #[test]
    fn three_of_seven_matches_the_hand_computation() {
        let (lo, hi) = wilson(3, 7).expect("seven trials");
        assert!(near(lo, 0.158_219_8), "{lo}");
        assert!(near(hi, 0.749_541_6), "{hi}");
    }

    /// 10 of 20: p = 0.5, so the centre is 0.5 exactly; 1 + z²/n = 1.1920729;
    /// p(1−p)/n + z²/4n² = 0.0125 + 0.0024009 = 0.0149009, root 0.1220693;
    /// half = (1.9599640 / 1.1920729) · 0.1220693 = 1.6441645 · 0.1220693
    /// = 0.2007020. So (0.2992980, 0.7007020).
    #[test]
    fn half_of_twenty_is_symmetric_about_a_half() {
        let (lo, hi) = wilson(10, 20).expect("twenty trials");
        assert!(near(lo, 0.299_298_0), "{lo}");
        assert!(near(hi, 0.700_702_0), "{hi}");
    }

    /// The ends have closed forms: at 0 of n the upper bound is z²/(n + z²),
    /// and at n of n the lower bound is n/(n + z²). For n = 7:
    /// 3.8414588 / 10.8414588 = 0.3543304 and 7 / 10.8414588 = 0.6456696.
    /// Neither end collapses to zero width, which is the point of Wilson.
    #[test]
    fn none_and_all_keep_an_interval_of_honest_width() {
        let (lo, hi) = wilson(0, 7).expect("seven trials");
        assert!(near(lo, 0.0) && near(hi, 0.354_330_4), "{lo} {hi}");
        let (lo, hi) = wilson(7, 7).expect("seven trials");
        assert!(near(lo, 0.645_669_6) && near(hi, 1.0), "{lo} {hi}");
    }

    /// Four more fixed values, to four places: 2 of 3 is 0.2077 to 0.9385,
    /// 16 of 24 is 0.4671 to 0.8203, 0 of 5 is 0 to 0.4345 and 5 of 5 is
    /// 0.5655 to 1. They were the reviewer-verdict panel's own check before
    /// it took this function, and came from a second implementation.
    #[test]
    fn four_values_from_a_second_implementation_agree() {
        for (k, n, want_lo, want_hi) in [
            (2_u32, 3_u32, 0.2077, 0.9385),
            (16, 24, 0.4671, 0.8203),
            (0, 5, 0.0, 0.4345),
            (5, 5, 0.5655, 1.0),
        ] {
            let (lo, hi) = wilson(k, n).expect("trials");
            assert!(
                (lo - want_lo).abs() < 5e-5 && (hi - want_hi).abs() < 5e-5,
                "{k} of {n}: {lo:.4} to {hi:.4}"
            );
        }
    }

    /// No trials is no interval, not the interval of a share of zero.
    #[test]
    fn no_trials_is_no_interval() {
        assert_eq!(wilson(0, 0), None);
        assert_eq!(wilson(3, 2), None);
    }

    /// The interval narrows as n grows at a fixed share, and always holds the
    /// share itself: a property, beside the fixed values above.
    #[test]
    fn the_interval_holds_the_share_and_narrows_with_n() {
        let mut last = f64::INFINITY;
        for n in [4_u32, 8, 16, 32, 64, 128] {
            let k = n / 4;
            let (lo, hi) = wilson(k, n).expect("trials");
            let p = f64::from(k) / f64::from(n);
            assert!(lo < p && p < hi, "{k}/{n}: {lo}..{hi}");
            assert!(hi - lo < last, "{k}/{n} is no narrower than before");
            last = hi - lo;
        }
    }
}
