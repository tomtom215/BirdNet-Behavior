//! Chance levels for the relational figures: a permutation null with a recorded seed.
//!
//! A figure that says two birds are "together", or that one "follows" or "answers" another,
//! has to say how often that would happen by chance, or it draws the species list (CHT-08,
//! CHT-12, CHT-14 in `docs/audit-2026-11/09-chart-review.md`). Every such figure takes its
//! chance level from here, so the method, the number of shuffles and the seed are one thing,
//! printed under each figure in the same words.
//!
//! * [`Rng`] is a small, seeded generator (`SplitMix64`). The figures are cached and shared
//!   by link, so the same data must give the same answer on every request: the seed is a
//!   constant ([`NULL_SEED`]), printed in each figure's method line, and every test is
//!   given its own stream derived from it and the test's name ([`Rng::for_key`]).
//! * [`stratified_shuffle`] is the co-occurrence null: within each stratum (an hour of the
//!   day), one species' blocks are put back at random among the blocks the station listened
//!   to in that stratum, and the shared blocks are counted again. That keeps each bird's
//!   amount of time and its daily rhythm and breaks only which blocks the two have in common,
//!   so two dawn singers are not "together" for both singing at dawn.
//! * [`Verdict::from_null`] turns a statistic and its shuffled copies into a two-sided
//!   permutation p-value, and [`benjamini_hochberg`] decides which of the tests made at once
//!   stand out, controlling the expected share of false discoveries among them at `q`.
//!
//! Pure arithmetic, no I/O, no `DuckDB`: available in the slim build.

/// The seed every chance level is drawn from. Printed in the method line of each figure that
/// uses one, so a reader can reproduce the shuffles.
pub const NULL_SEED: u64 = 20_261_127;

/// Shuffles per test: 999, so the smallest p-value a test can reach is 1/1000, small enough
/// to survive [`benjamini_hochberg`] over the 45 pairs of ten species (the strictest rank
/// needs 0.05/45 ≈ 0.0011).
pub const NULL_ROUNDS: u32 = 999;

/// The false-discovery rate the figures control: of the cells marked as standing out, at most
/// this share are expected to be chance, on average.
pub const FDR_Q: f64 = 0.05;

/// A seeded `SplitMix64` generator (Steele, Lea and Flood 2014; the constants are the
/// published ones, cited from general knowledge).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A generator for one named test: [`NULL_SEED`] mixed with an FNV-1a hash of `key`, so
    /// each pair has its own stream and adding a species does not change another pair's draws.
    #[must_use]
    pub fn for_key(key: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(NULL_SEED ^ h)
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform draw from `0..n` (`n > 0`), by Lemire's multiply-shift with rejection, so
    /// no value is favoured.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let threshold = n.wrapping_neg() % n;
        loop {
            let m = u128::from(self.next_u64()) * u128::from(n);
            #[allow(clippy::cast_possible_truncation)]
            let low = m as u64;
            if low >= threshold {
                #[allow(clippy::cast_possible_truncation)]
                return (m >> 64) as u64;
            }
        }
    }
}

/// One stratum of a co-occurrence test: `listened` blocks, of which species A was heard in
/// `a`, species B in `b`, and both in `shared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stratum {
    /// Blocks the station listened to in this stratum.
    pub listened: u64,
    /// Blocks species A was heard in.
    pub a: u64,
    /// Blocks species B was heard in.
    pub b: u64,
    /// Blocks both were heard in.
    pub shared: u64,
}

/// How many of `draws` blocks, put at random among `population`, land on one of the
/// `marked` ones: one hypergeometric draw, by sequential sampling.
///
/// The work is the smaller of the two sizes (the distribution is symmetric in `marked` and
/// `draws`), so a species heard in thousands of blocks does not make every shuffle slow.
fn hypergeometric(rng: &mut Rng, population: u64, marked: u64, draws: u64) -> u64 {
    let marked = marked.min(population);
    let draws = draws.min(population);
    let (small, other) = if draws <= marked {
        (draws, marked)
    } else {
        (marked, draws)
    };
    let mut hits = 0;
    let mut left = population;
    let mut other_left = other;
    for _ in 0..small {
        if other_left > 0 && rng.below(left) < other_left {
            hits += 1;
            other_left -= 1;
        }
        left -= 1;
    }
    hits
}

/// What a test found: the observed statistic beside what chance gives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NullResult {
    /// The statistic as observed.
    pub observed: f64,
    /// Its expected value under the null (exact where the null has a closed form, otherwise
    /// the mean of the shuffles).
    pub expected: f64,
    /// Shuffled values at or above the observed one.
    pub at_or_above: u32,
    /// Shuffled values at or below the observed one.
    pub at_or_below: u32,
    /// Shuffles made.
    pub rounds: u32,
}

impl NullResult {
    /// `observed / expected`: how many times chance. `None` when chance gives nothing (an
    /// expected value of zero says nothing about how unusual any count is).
    #[must_use]
    pub fn lift(&self) -> Option<f64> {
        (self.expected > 0.0).then(|| self.observed / self.expected)
    }

    /// The two-sided permutation p-value, `min(1, 2 · min(p_above, p_below))`, where each
    /// one-sided p counts the observed value among the shuffles (`(1 + k) / (1 + rounds)`),
    /// so it is never zero.
    #[must_use]
    pub fn p_two_sided(&self) -> f64 {
        let r = f64::from(self.rounds) + 1.0;
        let above = (f64::from(self.at_or_above) + 1.0) / r;
        let below = (f64::from(self.at_or_below) + 1.0) / r;
        (2.0 * above.min(below)).min(1.0)
    }
}

/// The co-occurrence null for one pair: species B's blocks put back at random among the
/// listened blocks of each stratum, `rounds` times, from `rng`.
///
/// The expected count is exact, `Σ a·b / listened`, the mean of the shuffles' distribution.
#[must_use]
pub fn stratified_shuffle(strata: &[Stratum], rounds: u32, rng: &mut Rng) -> NullResult {
    let observed: u64 = strata.iter().map(|s| s.shared).sum();
    #[allow(clippy::cast_precision_loss)]
    let expected: f64 = strata
        .iter()
        .filter(|s| s.listened > 0)
        .map(|s| s.a as f64 * s.b as f64 / s.listened as f64)
        .sum();
    let mut at_or_above = 0;
    let mut at_or_below = 0;
    for _ in 0..rounds {
        let t: u64 = strata
            .iter()
            .filter(|s| s.listened > 0)
            .map(|s| hypergeometric(rng, s.listened, s.a, s.b))
            .sum();
        if t >= observed {
            at_or_above += 1;
        }
        if t <= observed {
            at_or_below += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    NullResult {
        observed: observed as f64,
        expected,
        at_or_above,
        at_or_below,
        rounds,
    }
}

/// The time shifts of the "who answers whom" null (CHT-14): `count` offsets in seconds, each
/// between `min_secs` and `max_secs` in size, either sign, drawn from the stream named "answers".
///
/// Shifting one bird's times by minutes keeps its amount of song and its daily shape and breaks
/// only the timing under a minute that a reply is: if the second bird is heard within a minute
/// after the first as often with its clock moved by twenty minutes, the "replies" were the two
/// birds singing in the same part of the morning.
#[must_use]
pub fn answer_shifts(count: usize, min_secs: u64, max_secs: u64) -> Vec<i64> {
    let mut rng = Rng::for_key("answers");
    let span = max_secs.saturating_sub(min_secs) + 1;
    (0..count)
        .map(|_| {
            let size = i64::try_from(min_secs + rng.below(span)).unwrap_or(i64::MAX);
            if rng.below(2) == 0 { size } else { -size }
        })
        .collect()
}

/// The upper tail of the standard normal, `P(Z ≥ z)`.
///
/// From the complementary error function (the Chebyshev fit of Numerical Recipes' `erfcc`, fractional error under 1.2e-7 everywhere;
/// cited from general knowledge, checked in the tests against three tabled values).
#[must_use]
pub fn normal_upper(z: f64) -> f64 {
    let x = z / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.5 * x.abs());
    let poly = -x * x - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let erfc = t * poly.exp();
    let erfc = if x >= 0.0 { erfc } else { 2.0 - erfc };
    erfc / 2.0
}

/// A count against its shifted copies: `(expected, p)`.
///
/// The mean of the shifts is what chance gives, and the one-sided p-value that the count is above it takes the shifts' spread, never narrower than a
/// Poisson count's (`√mean`, and at least 1), on a normal approximation.
///
/// Returns `(expected, p)`. With a handful of shifts a rank alone cannot be small enough to
/// survive [`benjamini_hochberg`] over a grid of pairs (19 shifts give at best 1/20), so the
/// shifts give the level and the spread, and the normal tail the p-value.
#[must_use]
pub fn above_shifts(observed: f64, shifted: &[f64]) -> (f64, f64) {
    if shifted.is_empty() {
        return (0.0, 1.0);
    }
    #[allow(clippy::cast_precision_loss)]
    let k = shifted.len() as f64;
    let mean = shifted.iter().sum::<f64>() / k;
    let var = if shifted.len() > 1 {
        shifted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (k - 1.0)
    } else {
        0.0
    };
    let sd = var.sqrt().max(mean.sqrt()).max(1.0);
    (mean, normal_upper((observed - mean) / sd))
}

/// One row of [`crate::queries::neighbours_by_hour_sql`]: in `n` sessions, `anchor`'s first
/// detection fell in clock hour `hour` and `neighbour` was the species beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourNeighbour {
    /// Clock hour of the anchor's first detection in the session.
    pub hour: u8,
    /// The species whose first detection it was.
    pub anchor: String,
    /// The species heard next to it (just after, or just before).
    pub neighbour: String,
    /// Sessions.
    pub n: u64,
}

/// What chance gives `trigger` (CHT-12): the share of sessions in which each species would be its
/// neighbour if the trigger were followed (or preceded) as every *other* species is in the same
/// clock hours.
///
/// For each hour `h`, `w_h` is the share of the trigger's sessions whose first detection fell in
/// `h`, and `base_h(x)` the share of the other species' first detections in `h` that had `x`
/// beside them; the expected share of `x` is `Σ_h w_h · base_h(x)`. The commonest bird of the
/// trigger's hours is therefore expected to be its neighbour often, and is not a finding for
/// being so. Hours in which no other species had a neighbour carry no expectation and are left
/// out of the weights, so the shares are over the hours that can say something.
#[must_use]
pub fn neighbour_base(
    trigger: &str,
    rows: &[HourNeighbour],
) -> std::collections::BTreeMap<String, f64> {
    use std::collections::BTreeMap;
    let mut mine: BTreeMap<u8, u64> = BTreeMap::new();
    let mut others: BTreeMap<u8, BTreeMap<&str, u64>> = BTreeMap::new();
    for r in rows {
        if r.anchor == trigger {
            *mine.entry(r.hour).or_default() += r.n;
        } else {
            *others
                .entry(r.hour)
                .or_default()
                .entry(r.neighbour.as_str())
                .or_default() += r.n;
        }
    }
    let usable: u64 = mine
        .iter()
        .filter(|(h, _)| others.contains_key(h))
        .map(|(_, n)| n)
        .sum();
    let mut out = BTreeMap::new();
    if usable == 0 {
        return out;
    }
    for (h, n) in &mine {
        let Some(base) = others.get(h) else { continue };
        let total: u64 = base.values().sum();
        #[allow(clippy::cast_precision_loss)]
        let w = *n as f64 / usable as f64;
        for (x, k) in base {
            #[allow(clippy::cast_precision_loss)]
            let share = *k as f64 / total as f64;
            *out.entry((*x).to_owned()).or_insert(0.0) += w * share;
        }
    }
    out
}

/// The exact two-sided binomial test of `k` successes in `n` trials at share `p`.
///
/// It is the probability of every outcome no likelier than `k` (the sum of the mass at or below `k`'s).
/// `1.0` when `n` is zero or `p` is outside `(0, 1)` (nothing to test against).
#[must_use]
pub fn binomial_two_sided(k: u64, n: u64, p: f64) -> f64 {
    if n == 0 || !(p > 0.0 && p < 1.0) || k > n {
        return 1.0;
    }
    // Log masses by the recurrence pmf(i + 1) = pmf(i) · (n − i)/(i + 1) · p/(1 − p).
    let (lp, lq) = (p.ln(), (1.0 - p).ln());
    #[allow(clippy::cast_precision_loss)]
    let mut log = vec![0.0; usize::try_from(n).unwrap_or(usize::MAX - 1) + 1];
    #[allow(clippy::cast_precision_loss)]
    {
        log[0] = n as f64 * lq;
        for i in 0..n {
            let iu = usize::try_from(i).unwrap_or(0);
            log[iu + 1] = log[iu] + ((n - i) as f64).ln() - ((i + 1) as f64).ln() + lp - lq;
        }
    }
    let at = log[usize::try_from(k).unwrap_or(0)];
    let tol = 1e-7;
    log.iter()
        .filter(|&&l| l <= at + tol)
        .map(|l| l.exp())
        .sum::<f64>()
        .min(1.0)
}

/// Which side of chance a test fell on, once the tests made with it have been allowed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// More than chance gives, and it stands out among the tests made at once.
    More,
    /// Less than chance gives, and it stands out.
    Less,
    /// Within what chance gives: nothing to say about this one.
    Chance,
}

impl Verdict {
    /// The verdict for a result that [`benjamini_hochberg`] did (`stands_out`) or did not mark.
    #[must_use]
    pub fn from_null(r: &NullResult, stands_out: bool) -> Self {
        if !stands_out {
            return Self::Chance;
        }
        match r.lift() {
            Some(l) if l > 1.0 => Self::More,
            Some(l) if l < 1.0 => Self::Less,
            _ => Self::Chance,
        }
    }
}

/// Which of `p` stand out at false-discovery rate `q` (Benjamini and Hochberg 1995, step-up):
/// sort ascending, find the largest rank `k` with `p(k) ≤ k·q/m`, and mark the `k` smallest.
///
/// The result is in the order of `p`. An empty input marks nothing; a non-finite p-value is
/// never marked.
#[must_use]
pub fn benjamini_hochberg(p: &[f64], q: f64) -> Vec<bool> {
    let m = p.len();
    let mut order: Vec<usize> = (0..m).filter(|&i| p[i].is_finite()).collect();
    order.sort_by(|&x, &y| p[x].total_cmp(&p[y]).then(x.cmp(&y)));
    let mut cut = 0;
    #[allow(clippy::cast_precision_loss)]
    for (rank, &i) in order.iter().enumerate() {
        if p[i] <= (rank + 1) as f64 * q / m as f64 {
            cut = rank + 1;
        }
    }
    let mut out = vec![false; m];
    for &i in order.iter().take(cut) {
        out[i] = true;
    }
    out
}

/// The method line every figure with a chance level prints, so the words are the same
/// everywhere: what was shuffled, how often, the seed and the multiple-test rule.
#[must_use]
pub fn method_words(shuffled: &str, tests: usize) -> String {
    format!(
        "Chance is worked out by shuffling: {shuffled}, {NULL_ROUNDS} times (seed {NULL_SEED}). A result is marked only when it falls outside the shuffles after allowing for the {tests} test{s} made at once (Benjamini–Hochberg, false-discovery rate {q:.0}%). Exploratory and associational: it says what was heard together, not why.",
        s = if tests == 1 { "" } else { "s" },
        q = FDR_Q * 100.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_is_reproducible_and_uniform_enough() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Rng::new(1);
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            counts[usize::try_from(r.below(6)).unwrap()] += 1;
        }
        for c in counts {
            assert!((9_400..=10_600).contains(&c), "{counts:?}");
        }
        assert_ne!(
            Rng::for_key("A|B").next_u64(),
            Rng::for_key("A|C").next_u64()
        );
    }

    /// The draw's mean is `marked · draws / population`, whichever of the two is smaller.
    #[test]
    fn a_hypergeometric_draw_has_the_right_mean_on_both_sides_of_the_symmetry() {
        for (n, k, d) in [(100, 10, 30), (100, 30, 10), (50, 50, 7), (40, 0, 9)] {
            let mut r = Rng::new(3);
            let total: u64 = (0..20_000).map(|_| hypergeometric(&mut r, n, k, d)).sum();
            #[allow(clippy::cast_precision_loss)]
            let mean = total as f64 / 20_000.0;
            #[allow(clippy::cast_precision_loss)]
            let want = k as f64 * d as f64 / n as f64;
            assert!(
                (mean - want).abs() < 0.05 + want * 0.02,
                "{n} {k} {d}: {mean} vs {want}"
            );
        }
    }

    /// Two birds that each sing in the same hours, independently, share about what chance
    /// gives: the test does not mark them. A pair placed in the same blocks far more often
    /// than its hours explain is marked, and on the right side.
    #[test]
    fn independence_is_chance_and_a_planted_pair_is_more() {
        // Twelve dawn hours of 30 days: 360 blocks each; both birds in 90 of them.
        // Independent: expected shared = 90·90/360 = 22.5; observed 23.
        let s = [Stratum {
            listened: 360,
            a: 90,
            b: 90,
            shared: 23,
        }];
        let r = stratified_shuffle(&s, NULL_ROUNDS, &mut Rng::new(NULL_SEED));
        assert!((r.expected - 22.5).abs() < 1e-9);
        assert!(r.p_two_sided() > 0.5, "{r:?}");
        let planted = [Stratum { shared: 60, ..s[0] }];
        let r = stratified_shuffle(&planted, NULL_ROUNDS, &mut Rng::new(NULL_SEED));
        assert!(r.p_two_sided() <= 0.002, "{r:?}");
        assert_eq!(Verdict::from_null(&r, true), Verdict::More);
        let apart = [Stratum { shared: 2, ..s[0] }];
        let r = stratified_shuffle(&apart, NULL_ROUNDS, &mut Rng::new(NULL_SEED));
        assert!(r.p_two_sided() <= 0.002, "{r:?}");
        assert_eq!(Verdict::from_null(&r, true), Verdict::Less);
    }

    /// Strata are what remove the daily cycle: two birds that both sing only at dawn share
    /// many blocks pooled over the day, and none more than their hour gives.
    #[test]
    fn the_daily_cycle_alone_is_not_togetherness() {
        // Dawn: 360 listened blocks, each bird in 180, sharing 90 (independent within dawn).
        // The other 23 hours: 8280 listened blocks, neither bird.
        let strata = [
            Stratum {
                listened: 360,
                a: 180,
                b: 180,
                shared: 90,
            },
            Stratum {
                listened: 8_280,
                a: 0,
                b: 0,
                shared: 0,
            },
        ];
        let r = stratified_shuffle(&strata, NULL_ROUNDS, &mut Rng::new(NULL_SEED));
        assert!((r.lift().unwrap() - 1.0).abs() < 1e-9, "{r:?}");
        assert!(r.p_two_sided() > 0.5);
        // Counterpart: pooled into one stratum, the same counts are a strong "together".
        let pooled = [Stratum {
            listened: 8_640,
            a: 180,
            b: 180,
            shared: 90,
        }];
        let r = stratified_shuffle(&pooled, NULL_ROUNDS, &mut Rng::new(NULL_SEED));
        assert!(
            r.lift().unwrap() > 20.0 && r.p_two_sided() <= 0.002,
            "{r:?}"
        );
    }

    #[test]
    fn benjamini_hochberg_marks_the_step_up_set() {
        // m = 5, q = 0.05: thresholds 0.01, 0.02, 0.03, 0.04, 0.05.
        let p = [0.03, 0.001, 0.2, 0.039, 0.5];
        // Sorted 0.001, 0.03, 0.039, 0.2, 0.5.
        // rank 1: 0.001 ≤ 0.01 yes; rank 2: 0.03 ≤ 0.02 no; rank 3: 0.039 ≤ 0.03 no;
        // rank 4: 0.2 ≤ 0.04 no; rank 5: 0.5 ≤ 0.05 no. So only the first.
        assert_eq!(
            benjamini_hochberg(&p, 0.05),
            [false, true, false, false, false]
        );
        // Step-up: a later rank passing carries the earlier ones with it.
        let p = [0.011, 0.012, 0.013, 0.5];
        // thresholds 0.0125, 0.025, 0.0375, 0.05: rank 3 passes, so all three are marked.
        assert_eq!(benjamini_hochberg(&p, 0.05), [true, true, true, false]);
        assert!(benjamini_hochberg(&[], 0.05).is_empty());
        assert_eq!(benjamini_hochberg(&[f64::NAN, 0.0001], 0.05), [false, true]);
    }

    #[test]
    fn the_p_value_is_never_zero_and_never_above_one() {
        let r = NullResult {
            observed: 9.0,
            expected: 1.0,
            at_or_above: 0,
            at_or_below: 999,
            rounds: 999,
        };
        assert!((r.p_two_sided() - 0.002).abs() < 1e-12);
        let mid = NullResult {
            at_or_above: 999,
            ..r
        };
        assert!((mid.p_two_sided() - 1.0).abs() < 1e-12);
        let none = NullResult { expected: 0.0, ..r };
        assert_eq!(none.lift(), None);
        assert_eq!(Verdict::from_null(&none, true), Verdict::Chance);
    }

    fn row(hour: u8, anchor: &str, neighbour: &str, n: u64) -> HourNeighbour {
        HourNeighbour {
            hour,
            anchor: anchor.to_owned(),
            neighbour: neighbour.to_owned(),
            n,
        }
    }

    /// CHT-12: the commonest bird of the trigger's hours is expected beside it as often as it is
    /// beside everyone in those hours; hours the trigger is not heard in do not count.
    #[test]
    fn chance_is_the_neighbours_of_everyone_else_in_the_triggers_hours() {
        let rows = [
            // Dawn: the trigger in 30 sessions; the others are followed by the Robin 3 times in 4.
            row(5, "Jay", "Robin", 20),
            row(5, "Jay", "Wren", 10),
            row(5, "Wren", "Robin", 30),
            row(5, "Tit", "Robin", 15),
            row(5, "Tit", "Wren", 15),
            // Dusk: the trigger in 10 sessions; the others are followed by the Owl only.
            row(20, "Jay", "Owl", 10),
            row(20, "Robin", "Owl", 50),
            // An hour the trigger never opens: ignored.
            row(12, "Robin", "Gull", 500),
        ];
        let base = neighbour_base("Jay", &rows);
        assert!((base["Robin"] - 0.75 * 0.75).abs() < 1e-12, "{base:?}");
        assert!((base["Wren"] - 0.75 * 0.25).abs() < 1e-12, "{base:?}");
        assert!((base["Owl"] - 0.25).abs() < 1e-12, "{base:?}");
        assert!(!base.contains_key("Gull"));
        assert!((base.values().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_binomial_test_against_hand_values() {
        // 0 of 10 at p = 0.5: P = 2 · 0.5^10.
        assert!((binomial_two_sided(0, 10, 0.5) - 2.0 / 1024.0).abs() < 1e-12);
        // 5 of 10 at 0.5 is the mode: P = 1.
        assert!((binomial_two_sided(5, 10, 0.5) - 1.0).abs() < 1e-9);
        // 9 of 10 at 0.5: masses at 0, 1, 9, 10 = 22/1024.
        assert!((binomial_two_sided(9, 10, 0.5) - 22.0 / 1024.0).abs() < 1e-12);
        assert!((binomial_two_sided(3, 0, 0.5) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_normal_tail_matches_the_table() {
        assert!((normal_upper(0.0) - 0.5).abs() < 1e-7);
        assert!((normal_upper(1.959_964) - 0.025).abs() < 1e-6);
        assert!((normal_upper(3.090_232) - 0.001).abs() < 1e-6);
        assert!((normal_upper(-1.644_854) - 0.95).abs() < 1e-6);
    }

    #[test]
    fn the_shifts_are_reproducible_minutes_either_way() {
        let s = answer_shifts(19, 300, 1800);
        assert_eq!(s, answer_shifts(19, 300, 1800));
        assert_eq!(s.len(), 19);
        assert!(s.iter().all(|v| (300..=1800).contains(&v.abs())), "{s:?}");
        assert!(
            s.iter().any(|&v| v > 0) && s.iter().any(|&v| v < 0),
            "{s:?}"
        );
    }

    /// A count like its shifts is chance; a count far above them is not; a spread of zero
    /// among the shifts does not make a count of one above a mean of zero a finding.
    #[test]
    fn a_count_is_read_against_its_shifts() {
        let shifts = [10.0, 12.0, 9.0, 11.0, 13.0, 8.0, 10.0, 12.0];
        let (mean, p) = above_shifts(11.0, &shifts);
        assert!((mean - 10.625).abs() < 1e-9);
        assert!(p > 0.3, "{p}");
        let (_, p) = above_shifts(40.0, &shifts);
        assert!(p < 1e-6, "{p}");
        let (_, p) = above_shifts(1.0, &[0.0; 8]);
        assert!(
            p > 0.1,
            "a single reply where shifts give none is not a finding: {p}"
        );
        assert_eq!(above_shifts(5.0, &[]), (0.0, 1.0));
    }

    #[test]
    fn the_method_words_carry_the_seed_and_the_rule() {
        let w = method_words("each species' blocks within their hour", 45);
        assert!(w.contains("999 times (seed 20261127)"), "{w}");
        assert!(w.contains("45 tests"), "{w}");
        assert!(w.contains("Benjamini–Hochberg"), "{w}");
        assert!(w.contains("Exploratory and associational"), "{w}");
    }
}
