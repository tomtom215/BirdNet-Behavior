//! Chance levels and dependence-aware intervals for the exploratory analyses
//! (RES-16, N25-02, UIR-22, CHT-24).
//!
//! Every analysis on the Patterns pages is **exploratory and associational**:
//! what is computed here says whether a figure differs from what chance would
//! draw from the same data, not why, and never that one species causes,
//! answers or follows another.
//!
//! Three tools, each named by a method string a response carries:
//!
//! * [`SplitMix64`], a small seeded generator. It is written here rather than
//!   taken from a crate so that a recorded seed reproduces the same draws in
//!   every later version of this code: a crate's stream may change between its
//!   releases, and then an old answer could no longer be re-derived.
//! * [`dependent_share`] ([`SHARE_METHOD`]): a share of days (days heard of
//!   days listened) whose interval allows for neighbouring days being alike, by
//!   a design effect from a first-order Markov chain fitted to the species'
//!   whole daily series (N25-02).
//! * [`shift_null`] ([`SHIFT_NULL_METHOD`]): the chance level of a count of
//!   shared occasions between two species, by shifting one species' days
//!   circularly within strata of consecutive days, which keeps each species'
//!   prevalence, its own time-of-day pattern, its season (to within a
//!   stratum) and its run-to-run persistence, and breaks only their alignment
//!   (RES-16). [`benjamini_hochberg`] adjusts the resulting p-values for the
//!   number of pairs looked at.

/// The method a [`DependentShare`] names.
pub const SHARE_METHOD: &str = "wilson-markov-design-effect-1";

/// The method a [`PairNull`] names.
pub const SHIFT_NULL_METHOD: &str = "stratified-circular-day-shift-1";

/// The seed every analysis uses unless asked for another. Recorded in each
/// response, so an answer can be re-derived.
pub const DEFAULT_SEED: u64 = 0x2026_1125_0000_0001;

/// The false-discovery rate below which a pair is said to stand out.
pub const FDR_ALPHA: f64 = 0.05;

/// The `SplitMix64` generator (Steele, Lea and Flood, 2014): 64 bits of state,
/// one multiply-xorshift mix per output. Not for secrets.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform draw from `0..n`; `0` when `n` is 0. Unbiased: draws in the
    /// short top slice of the 64-bit range are rejected.
    pub const fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let x = self.next_u64();
            if x < zone {
                return x % n;
            }
        }
    }

    /// A uniform draw from `[0, 1)` with 53 bits.
    pub fn next_f64(&mut self) -> f64 {
        // 53 bits fit an f64's mantissa exactly.
        #[allow(clippy::cast_precision_loss)]
        let (x, scale) = ((self.next_u64() >> 11) as f64, (1_u64 << 53) as f64);
        x / scale
    }
}

/// A seed for one item of an analysis (a pair, a species), from the
/// analysis' seed and the item's name: the item's draws do not depend on
/// which other items were analysed or in what order. 64-bit FNV-1a.
#[must_use]
pub fn item_seed(seed: u64, item: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
    for b in item.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// A share of dependent days
// ---------------------------------------------------------------------------

/// A share of days with an interval that allows for neighbouring days being
/// alike.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct DependentShare {
    /// Days the species was heard.
    pub heard: u32,
    /// Days listened.
    pub listened: u32,
    /// `heard / listened`.
    pub share: f64,
    /// Lower bound of the two-sided 95 % interval.
    pub lower: f64,
    /// Upper bound.
    pub upper: f64,
    /// The day-to-day persistence used (the Markov chain's lag-one
    /// correlation), in `[0, MAX_RHO]`.
    pub rho: f64,
    /// The variance of this share over what independent days would give.
    pub design_effect: f64,
    /// `listened / design_effect`: how many independent days the listened
    /// days are worth.
    pub effective_days: f64,
    /// [`SHARE_METHOD`].
    pub method: &'static str,
}

/// The largest persistence used: at 1 a run of days is one trial however
/// long, and the effective count would be one day per run.
pub const MAX_RHO: f64 = 0.99;

/// The lag-one correlation of a two-state Markov chain fitted to `runs`, each
/// a run of consecutive calendar days in date order (`true`: heard).
///
/// With `a = P(heard | not heard the day before)` and `b = P(not heard |
/// heard the day before)` counted over consecutive days, the chain's
/// correlation at lag one is `1 − a − b`. `None` when either transition
/// probability has no day to be counted from (the species was heard on every
/// day, or on none): there is then nothing to estimate persistence from.
///
/// A species whose presence is seasonal makes the chain look more persistent
/// than its day-to-day behaviour is, because the season is a run too. That
/// errs towards a wider interval, never a narrower one.
#[must_use]
pub fn markov_rho(runs: &[Vec<bool>]) -> Option<f64> {
    let (mut from0, mut up, mut from1, mut down) = (0_u64, 0_u64, 0_u64, 0_u64);
    for run in runs {
        for w in run.windows(2) {
            match (w[0], w[1]) {
                (false, next) => {
                    from0 += 1;
                    up += u64::from(next);
                }
                (true, next) => {
                    from1 += 1;
                    down += u64::from(!next);
                }
            }
        }
    }
    if from0 == 0 || from1 == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let (a, b) = (up as f64 / from0 as f64, down as f64 / from1 as f64);
    Some(1.0 - a - b)
}

/// The design effect of the mean of `m` consecutive days of a chain.
///
/// With lag-one correlation `rho` (correlation `rho^k` at lag `k`) it is
/// `1 + 2 Σ_{k=1}^{m−1} (1 − k/m) rho^k`. A negative `rho` is taken as 0, so
/// an alternating chain is never credited with more days than it has.
#[must_use]
pub fn run_design_effect(m: usize, rho: f64) -> f64 {
    let rho = rho.clamp(0.0, MAX_RHO);
    let mut sum = 1.0;
    let mut pow = 1.0;
    #[allow(clippy::cast_precision_loss)]
    let mf = m as f64;
    for k in 1..m {
        pow *= rho;
        #[allow(clippy::cast_precision_loss)]
        let lag = k as f64;
        sum += 2.0 * (1.0 - lag / mf) * pow;
    }
    sum
}

/// The share of days heard, with an interval at the days' effective number.
///
/// `runs` are the consecutive listened days of one period, each run in
/// date order; runs from different years or split by an unlistened day are
/// separate) that are `true`, with a Wilson interval at the effective number
/// of days the design effect leaves (`rho`, from [`markov_rho`] over the
/// species' whole series; `None` reads as independent days).
///
/// `None` when there are no days.
#[must_use]
pub fn dependent_share(runs: &[Vec<bool>], rho: Option<f64>) -> Option<DependentShare> {
    let n: usize = runs.iter().map(Vec::len).sum();
    if n == 0 {
        return None;
    }
    let k: usize = runs.iter().flatten().filter(|d| **d).count();
    let rho = rho.unwrap_or(0.0).clamp(0.0, MAX_RHO);
    #[allow(clippy::cast_precision_loss)]
    let weighted: f64 = runs
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| r.len() as f64 * run_design_effect(r.len(), rho))
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let (kf, nf) = (k as f64, n as f64);
    let design_effect = weighted / nf;
    let effective = nf / design_effect;
    let share = kf / nf;
    let (lower, upper) = crate::interval::wilson_real(share * effective, effective)?;
    Some(DependentShare {
        heard: u32::try_from(k).unwrap_or(u32::MAX),
        listened: u32::try_from(n).unwrap_or(u32::MAX),
        share,
        lower,
        upper,
        rho,
        design_effect,
        effective_days: effective,
        method: SHARE_METHOD,
    })
}

// ---------------------------------------------------------------------------
// The chance level of shared occasions
// ---------------------------------------------------------------------------

/// Five-minute slots in a day.
pub const SLOTS_PER_DAY: usize = 288;

/// One day's occasions of one species: a bit per five-minute slot.
pub type DaySlots = [u64; 5];

/// The days a stratum holds: shifts happen within a run of this many eligible
/// days, so a species is only ever moved to a day within about four weeks of
/// its own, which keeps its season.
pub const STRATUM_DAYS: usize = 28;

/// Random shifts drawn per pair when they are not enumerated.
pub const DEFAULT_DRAWS: usize = 999;

/// Set slot `slot` (`0..288`) of `day`.
pub const fn set_slot(day: &mut DaySlots, slot: usize) {
    if slot < SLOTS_PER_DAY {
        day[slot / 64] |= 1 << (slot % 64);
    }
}

fn shared(a: &DaySlots, b: &DaySlots) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x & y).count_ones()).sum()
}

/// The strata `days` eligible days split into: consecutive runs of
/// [`STRATUM_DAYS`], the last joined to the one before when shorter than half
/// a stratum. Each is `(start, len)`.
#[must_use]
pub fn strata(days: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start < days {
        let len = STRATUM_DAYS.min(days - start);
        out.push((start, len));
        start += len;
    }
    if out.len() > 1
        && out.last().is_some_and(|l| l.1 < STRATUM_DAYS / 2)
        && let Some((_, tail)) = out.pop()
        && let Some(prev) = out.last_mut()
    {
        prev.1 += tail;
    }
    out
}

/// What chance gives for one pair.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct PairNull {
    /// Shared occasions observed.
    pub observed: u32,
    /// The mean shared occasions over the shifts: the chance level.
    pub expected: f64,
    /// `(1 + shifts sharing at least as many) / (1 + shifts)`: one-sided, for
    /// "more together than chance".
    pub p_value: f64,
    /// The shifts compared against.
    pub draws: usize,
    /// Whether every shift was taken (one stratum, fewer shifts than draws):
    /// the p-value is then exact for this null and no seed was used.
    pub exhaustive: bool,
    /// The smallest p-value this many shifts can give.
    pub min_p: f64,
}

/// The chance level of the occasions `a` and `b` share, by shifting `b`'s
/// days circularly within each stratum of the eligible days ([`strata`]),
/// every stratum by its own offset.
///
/// `a` and `b` are the two species' occasions on the same eligible days, in
/// date order. With one stratum whose `len − 1` shifts are no more than
/// `draws`, every shift is taken; otherwise `draws` sets of offsets are drawn
/// from `seed`. A stratum of one day cannot move.
///
/// # Panics
///
/// Never: `a` and `b` of unequal lengths are read to the shorter.
#[must_use]
pub fn shift_null(a: &[DaySlots], b: &[DaySlots], draws: usize, seed: u64) -> PairNull {
    let days = a.len().min(b.len());
    let (a, b) = (&a[..days], &b[..days]);
    let observed: u32 = a.iter().zip(b).map(|(x, y)| shared(x, y)).sum();
    let st = strata(days);
    // Only the days `a` was heard on can share anything: (stratum, offset in
    // it) of each.
    let busy: Vec<(usize, usize)> = st
        .iter()
        .enumerate()
        .flat_map(|(j, &(start, len))| {
            (0..len)
                .filter(move |&i| a[start + i].iter().any(|w| *w != 0))
                .map(move |i| (j, i))
        })
        .collect();
    let at = |offsets: &[usize]| -> u32 {
        busy.iter()
            .map(|&(j, i)| {
                let (start, len) = st[j];
                shared(&a[start + i], &b[start + (i + offsets[j]) % len])
            })
            .sum()
    };
    let mut null: Vec<u32> = Vec::new();
    let exhaustive = st.len() == 1 && st[0].1.saturating_sub(1) <= draws;
    if exhaustive {
        for s in 1..st[0].1 {
            null.push(at(&[s]));
        }
    } else {
        let mut rng = SplitMix64::new(seed);
        let mut offsets = vec![0_usize; st.len()];
        for _ in 0..draws {
            for (o, &(_, len)) in offsets.iter_mut().zip(&st) {
                // A shift of 0 would be the observed alignment.
                let len = len as u64;
                *o = if len > 1 {
                    usize::try_from(1 + rng.below(len - 1)).unwrap_or(1)
                } else {
                    0
                };
            }
            null.push(at(&offsets));
        }
    }
    let n = null.len();
    #[allow(clippy::cast_precision_loss)]
    let expected = if n == 0 {
        f64::from(observed)
    } else {
        null.iter().map(|&x| f64::from(x)).sum::<f64>() / n as f64
    };
    let at_least = null.iter().filter(|&&x| x >= observed).count();
    #[allow(clippy::cast_precision_loss)]
    let (p_value, min_p) = ((1 + at_least) as f64 / (1 + n) as f64, 1.0 / (1 + n) as f64);
    PairNull {
        observed,
        expected,
        p_value,
        draws: n,
        exhaustive,
        min_p,
    }
}

/// Benjamini–Hochberg adjusted p-values (q-values), in the order given:
/// `q_(i) = min_{j >= i} p_(j) · m / j` over the p-values sorted ascending,
/// capped at 1. A NaN p-value is taken as 1.
#[must_use]
pub fn benjamini_hochberg(p: &[f64]) -> Vec<f64> {
    let m = p.len();
    let mut order: Vec<usize> = (0..m).collect();
    let pv = |i: usize| if p[i].is_nan() { 1.0 } else { p[i] };
    order.sort_by(|&x, &y| pv(x).total_cmp(&pv(y)).then(x.cmp(&y)));
    let mut q = vec![1.0; m];
    let mut running = 1.0_f64;
    for (rank, &i) in order.iter().enumerate().rev() {
        #[allow(clippy::cast_precision_loss)]
        let adj = pv(i) * m as f64 / (rank + 1) as f64;
        running = running.min(adj).min(1.0);
        q[i] = running;
    }
    q
}

/// What a relational figure may say about one pair. The default, for a pair
/// no null was computed for, is [`Verdict::TooLittleData`]: nothing is claimed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// More together than chance, after the false-discovery adjustment.
    StandsOut,
    /// Not distinguishable from chance with these data.
    WithinChance,
    /// Too few days for any pair to stand out: even the most extreme
    /// alignment could not reach the adjusted threshold.
    #[default]
    TooLittleData,
}

/// Whether shifts whose smallest attainable p-value is `min_p` can show anything.
///
/// The best adjusted q-value any pair can reach is `min_p`
/// (when every pair tested is as extreme as it can be), so above
/// [`FDR_ALPHA`] nothing can stand out however strong it is.
#[must_use]
pub fn too_little_data(min_p: f64) -> bool {
    min_p > FDR_ALPHA
}

/// The verdict for a pair with adjusted `q`, `observed` shared occasions
/// against its `expected` chance level, when the smallest p-value the null
/// could give is `min_p`.
#[must_use]
pub fn verdict(q: f64, observed: u32, expected: f64, min_p: f64) -> Verdict {
    if too_little_data(min_p) {
        Verdict::TooLittleData
    } else if q <= FDR_ALPHA && f64::from(observed) > expected {
        Verdict::StandsOut
    } else {
        Verdict::WithinChance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_is_fixed_by_its_seed() {
        // SplitMix64's published first output for seed 0.
        let mut g = SplitMix64::new(0);
        assert_eq!(g.next_u64(), 0xe220_a839_7b1d_cdaf);
        let (mut a, mut b) = (SplitMix64::new(7), SplitMix64::new(7));
        for _ in 0..100 {
            assert_eq!(a.below(10), b.below(10));
        }
        assert_eq!(SplitMix64::new(1).below(0), 0);
    }

    #[test]
    fn a_draw_below_n_is_below_n_and_reaches_every_value() {
        let mut g = SplitMix64::new(3);
        let mut seen = [0_u32; 6];
        for _ in 0..600 {
            let x = g.below(6);
            seen[usize::try_from(x).unwrap()] += 1;
        }
        assert!(seen.iter().all(|&c| c > 50), "{seen:?}");
    }

    #[test]
    fn independent_days_have_no_design_effect() {
        assert!((run_design_effect(7, 0.0) - 1.0).abs() < 1e-12);
        // m = 3, rho = 0.5: 1 + 2(2/3 · 0.5 + 1/3 · 0.25) = 1 + 2 · 5/12.
        assert!((run_design_effect(3, 0.5) - (1.0 + 10.0 / 12.0)).abs() < 1e-12);
        // A single day is one trial whatever the persistence.
        assert!((run_design_effect(1, 0.9) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_markov_estimate_reads_persistence() {
        // Long runs: 0000011111 repeated.
        let run: Vec<bool> = (0..100).map(|i| (i / 5) % 2 == 1).collect();
        let rho = markov_rho(&[run]).expect("both states");
        // a = P(up) = 10/50ish, b similar: rho ~ 0.6.
        assert!((0.55..0.65).contains(&rho), "{rho}");
        assert_eq!(markov_rho(&[vec![true; 10]]), None);
    }

    #[test]
    fn bh_matches_a_hand_computation() {
        // p = .01 .04 .03 .20, m = 4: sorted .01 .03 .04 .20 → .04 .06 .0533 .20,
        // running minimum from the top: .04 .0533 .0533 .20.
        let q = benjamini_hochberg(&[0.01, 0.04, 0.03, 0.20]);
        let want = [0.04, 0.16 / 3.0, 0.16 / 3.0, 0.20];
        for (g, w) in q.iter().zip(want) {
            assert!((g - w).abs() < 1e-12, "{q:?}");
        }
    }

    #[test]
    fn strata_join_a_short_tail() {
        assert_eq!(strata(30), vec![(0, 30)]);
        assert_eq!(strata(56), vec![(0, 28), (28, 28)]);
        assert_eq!(strata(70), vec![(0, 28), (28, 28), (56, 14)]);
        assert_eq!(strata(0), Vec::<(usize, usize)>::new());
    }

    fn day_with(slots: &[usize]) -> DaySlots {
        let mut d = [0; 5];
        for &s in slots {
            set_slot(&mut d, s);
        }
        d
    }

    #[test]
    fn a_short_window_is_shifted_exhaustively_and_says_so() {
        let a: Vec<DaySlots> = (0..10).map(|_| day_with(&[84])).collect();
        let b = a.clone();
        let n = shift_null(&a, &b, DEFAULT_DRAWS, 1);
        assert!(n.exhaustive);
        assert_eq!(n.draws, 9);
        // Both every day at 07:00: every shift aligns them too. Chance.
        assert_eq!(n.observed, 10);
        assert!((n.expected - 10.0).abs() < 1e-12);
        assert!((n.p_value - 1.0).abs() < 1e-12);
    }
}
