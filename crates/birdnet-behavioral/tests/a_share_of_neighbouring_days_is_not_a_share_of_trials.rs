//! N25-02: the reporting-frequency interval treats neighbouring days as
//! independent trials.
//!
//! A species present today is likely present tomorrow. On a simulated
//! two-state Markov series with that persistence, an interval that counts a
//! period's listened days as separate trials (a plain Wilson interval) holds
//! the true share far less often than the 95 % it claims; the interval each
//! period now carries ([`birdnet_behavioral::null_model::SHARE_METHOD`]) must
//! hold it about as often as it says. The series is drawn from a fixed seed.
//!
//! Counterpart: on a series with no persistence, the interval must stay near
//! the plain one, so the gate is not passed by making every interval wide.

use birdnet_behavioral::interval::wilson;
use birdnet_behavioral::null_model::SplitMix64;
use birdnet_behavioral::reporting::period_intervals;

/// `YYYY-MM-DD` of `n` days after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
#[allow(
    clippy::many_single_char_names,
    reason = "Hinnant's own names, so the algorithm reads against its source"
)]
fn date(n: i64) -> String {
    let z = n + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Two years of daily presence from a stationary Markov chain with share `p`
/// and lag-one correlation `rho`.
fn series(rng: &mut SplitMix64, p: f64, rho: f64) -> Vec<(String, bool)> {
    let up = p * (1.0 - rho);
    let down = (1.0 - p) * (1.0 - rho);
    let start = 19_723; // 2024-01-01
    let mut state = rng.next_f64() < p;
    (0..731)
        .map(|i| {
            let r = rng.next_f64();
            state = if state { r >= down } else { r < up };
            (date(start + i), state)
        })
        .collect()
}

struct Coverage {
    new: f64,
    plain: f64,
    width_ratio: f64,
}

fn coverage(p: f64, rho: f64, seed: u64) -> Coverage {
    let mut rng = SplitMix64::new(seed);
    let (mut hits, mut plain_hits, mut total) = (0_u32, 0_u32, 0_u32);
    let (mut w_new, mut w_plain) = (0.0, 0.0);
    for _ in 0..200 {
        let days = series(&mut rng, p, rho);
        let (intervals, _) = period_intervals(&days);
        for iv in intervals.into_iter().flatten() {
            total += 1;
            hits += u32::from(iv.lower <= p && p <= iv.upper);
            let (lo, hi) = wilson(iv.heard, iv.listened).expect("days");
            plain_hits += u32::from(lo <= p && p <= hi);
            w_new += iv.upper - iv.lower;
            w_plain += hi - lo;
        }
    }
    assert!(total > 9000, "the simulation formed its intervals: {total}");
    Coverage {
        new: f64::from(hits) / f64::from(total),
        plain: f64::from(plain_hits) / f64::from(total),
        width_ratio: w_new / w_plain,
    }
}

#[test]
fn a_persistent_species_share_is_covered_as_often_as_its_interval_says() {
    let c = coverage(0.3, 0.7, 0x5EED_0225);
    // The defect, measured: the plain interval misses far more than 1 in 20.
    assert!(c.plain < 0.85, "plain Wilson covered {:.3}", c.plain);
    assert!(
        c.new >= 0.92,
        "the reporting interval covered the true share in {:.3} of periods (plain Wilson {:.3}); \
         a 95 % interval should hold it in about 0.95",
        c.new,
        c.plain
    );
}

#[test]
fn independent_days_keep_an_interval_close_to_the_plain_one() {
    let c = coverage(0.3, 0.0, 0x5EED_0226);
    assert!(c.new >= 0.92 && c.new <= 0.99, "covered {:.3}", c.new);
    assert!(
        c.width_ratio < 1.15,
        "with no persistence the interval is {:.2} times as wide as the plain one",
        c.width_ratio
    );
}
