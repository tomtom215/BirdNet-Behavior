//! "Heard just before and just after", against chance (CHT-12).
//!
//! `sequence_next_node` says which species was heard beside a bird's first detection in each
//! session. Ranked by how often, the answer is the species list: the commonest bird of the hours
//! the trigger sings in "follows" everything. Here each neighbour's share is set beside the share
//! chance gives it (`birdnet_behavioral::null::neighbour_base`: how often it is beside every
//! *other* species' first detection in the trigger's own clock hours), tested with the exact
//! binomial test, and the neighbours of both sides are marked together by Benjamini–Hochberg.
//! The list is then ordered by how far a neighbour stands from chance, not by its count.
//!
//! The same bird calling again is not tested: chance is about other species following one
//! another, and a bird repeating itself is a different fact, labelled as such.

use std::collections::BTreeMap;

use birdnet_behavioral::null::{FDR_Q, Verdict, benjamini_hochberg, binomial_two_sided};

/// One neighbour, rated against chance.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rated {
    /// The stored species name.
    pub species: String,
    /// Its share of the sessions with a bird on this side.
    pub probability: f64,
    /// Sessions in which it was the neighbour.
    pub frequency: u64,
    /// The share chance gives it, `None` for the trigger itself (not tested).
    pub chance: Option<f64>,
    /// Which side of chance it falls on.
    pub verdict: Verdict,
}

impl Rated {
    /// The share over chance's.
    fn lift(&self) -> Option<f64> {
        self.chance
            .filter(|c| *c > 0.0)
            .map(|c| self.probability / c)
    }

    /// How far from chance, for ordering: a marked neighbour by `|ln lift|`, anything else below
    /// every marked one.
    fn distance(&self) -> f64 {
        if self.verdict == Verdict::Chance {
            return -1.0;
        }
        self.lift()
            .map_or(0.0, |l| l.max(f64::MIN_POSITIVE).ln().abs())
    }
}

/// The sessions with a bird on a side, recovered from a row's share and count.
fn sessions_of(probability: f64, frequency: u64) -> u64 {
    if probability > 0.0 {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let n = (frequency as f64 / probability).round() as u64;
        n.max(frequency)
    } else {
        0
    }
}

/// Rate both sides of `trigger` against chance and keep the `keep` furthest from it on each
/// side (then the most frequent): `rows` are `(species, probability, sessions)` as the
/// extension gives them, and `base` the chance shares from `neighbour_base` for that side.
///
/// Returns the two sides and the number of neighbours tested.
pub(super) fn rate_sides(
    trigger: &str,
    before: &[(&str, f64, u64)],
    after: &[(&str, f64, u64)],
    base_before: &BTreeMap<String, f64>,
    base_after: &BTreeMap<String, f64>,
    keep: usize,
) -> (Vec<Rated>, Vec<Rated>, usize) {
    let mut sides: Vec<Vec<Rated>> = Vec::with_capacity(2);
    let mut p = Vec::new();
    let mut tested: Vec<(usize, usize)> = Vec::new();
    for (s, (rows, base)) in [(before, base_before), (after, base_after)]
        .iter()
        .enumerate()
    {
        let mut out = Vec::with_capacity(rows.len());
        for (k, &(species, probability, frequency)) in rows.iter().enumerate() {
            let chance = (species != trigger).then(|| base.get(species).copied().unwrap_or(0.0));
            if let Some(c) = chance {
                let n = sessions_of(probability, frequency);
                p.push(binomial_two_sided(frequency, n, c));
                tested.push((s, k));
            }
            out.push(Rated {
                species: species.to_owned(),
                probability,
                frequency,
                chance,
                verdict: Verdict::Chance,
            });
        }
        sides.push(out);
    }
    let marked = benjamini_hochberg(&p, FDR_Q);
    for (&(s, k), &m) in tested.iter().zip(&marked) {
        let r = &mut sides[s][k];
        r.verdict = if m {
            match r.lift() {
                Some(l) if l > 1.0 => Verdict::More,
                Some(l) if l < 1.0 => Verdict::Less,
                _ => Verdict::Chance,
            }
        } else {
            Verdict::Chance
        };
    }
    for side in &mut sides {
        side.sort_by(|x, y| {
            y.distance()
                .total_cmp(&x.distance())
                .then(y.frequency.cmp(&x.frequency))
                .then_with(|| x.species.cmp(&y.species))
        });
        side.truncate(keep);
    }
    let after_side = sides.pop().unwrap_or_default();
    let before_side = sides.pop().unwrap_or_default();
    (before_side, after_side, tested.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    /// CHT-12: the commonest bird of the hours "follows" the trigger in most sessions, and as
    /// often as it follows everyone: it is not first. A bird that follows the trigger three times
    /// as often as chance is first, though it is heard after it less often.
    #[test]
    fn the_commonest_bird_is_not_first_for_being_common() {
        // 200 sessions with a bird after the trigger.
        let after = [
            ("Robin", 0.60, 120),
            ("Nuthatch", 0.30, 60),
            ("Wren", 0.10, 20),
        ];
        let chance = base(&[("Robin", 0.62), ("Nuthatch", 0.10), ("Wren", 0.12)]);
        let (_, a, tested) = rate_sides("Jay", &[], &after, &BTreeMap::new(), &chance, 5);
        assert_eq!(tested, 3);
        assert_eq!(a[0].species, "Nuthatch", "{a:?}");
        assert_eq!(a[0].verdict, Verdict::More);
        let robin = a.iter().find(|r| r.species == "Robin").unwrap();
        assert_eq!(robin.verdict, Verdict::Chance, "{robin:?}");
    }

    /// Counterpart: when every neighbour is where chance puts it, nothing is marked, and the
    /// list falls back to the most frequent first.
    #[test]
    fn neighbours_where_chance_puts_them_are_all_within_chance() {
        let after = [
            ("Robin", 0.60, 120),
            ("Nuthatch", 0.30, 60),
            ("Wren", 0.10, 20),
        ];
        let chance = base(&[("Robin", 0.6), ("Nuthatch", 0.3), ("Wren", 0.1)]);
        let (_, a, _) = rate_sides("Jay", &[], &after, &BTreeMap::new(), &chance, 5);
        assert!(a.iter().all(|r| r.verdict == Verdict::Chance), "{a:?}");
        let order: Vec<&str> = a.iter().map(|r| r.species.as_str()).collect();
        assert_eq!(order, ["Robin", "Nuthatch", "Wren"]);
    }

    /// The bird calling again is kept and labelled, never tested; the limit cuts after the
    /// ranking.
    #[test]
    fn the_same_bird_again_is_not_tested_and_the_limit_cuts_last() {
        let after = [("Jay", 0.5, 50), ("Robin", 0.3, 30), ("Nuthatch", 0.2, 20)];
        let chance = base(&[("Robin", 0.3), ("Nuthatch", 0.02)]);
        let (_, a, tested) = rate_sides("Jay", &[], &after, &BTreeMap::new(), &chance, 2);
        assert_eq!(tested, 2);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].species, "Nuthatch");
        assert_eq!(a[1].species, "Jay");
        assert_eq!(a[1].chance, None);
    }
}
