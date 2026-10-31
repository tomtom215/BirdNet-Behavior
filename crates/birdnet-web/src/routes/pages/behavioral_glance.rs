//! One sentence for each of the Visits and Sequences figures that had none: what the figure found,
//! the first layer of an analytic card.
//!
//! Each is a pure function of numbers the card's partial already holds, so what a page claims is a
//! function with tests and not a string typed into a template (the same rule `insight.rs` keeps for
//! the charts on the other tabs). Each says its basis, is silent when there is too little to say
//! anything, and escapes every name it prints.

use std::collections::BTreeMap;

use birdnet_behavioral::types::{ResidencyType, SpeciesRetention};

use super::escape_html;
use super::viz::flow::Neighbour;

/// Fewest bursts before the figure is described: two are a coincidence.
pub(super) const MIN_BURSTS: usize = 3;
/// Fewest species before the return-rate ledger is described.
pub(super) const MIN_SPECIES: usize = 5;

/// One burst of singing: whose it was and how long it lasted, in seconds.
pub(super) struct Burst<'a> {
    pub species: &'a str,
    pub secs: u64,
}

/// What the bursts figure found: how many, whose were the most, and which ran the longest.
///
/// `scope` says which sessions they were found among ("the 100 newest sessions"), `duration` writes
/// a length of time. `None` for fewer than [`MIN_BURSTS`].
#[must_use]
pub(super) fn bursts_sentence(
    bursts: &[Burst<'_>],
    scope: &str,
    duration: impl Fn(u64) -> String,
) -> Option<String> {
    if bursts.len() < MIN_BURSTS {
        return None;
    }
    let n = bursts.len();
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for b in bursts {
        *tally.entry(b.species).or_insert(0) += 1;
    }
    // Most bursts; a tie goes to the name that sorts first, so the sentence does not change with
    // the order the sessions arrived in.
    let (busiest, k) = tally
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(s, k)| (*s, *k))?;
    let longest = bursts
        .iter()
        .max_by(|a, b| a.secs.cmp(&b.secs).then_with(|| b.species.cmp(a.species)))?;
    Some(if tally.len() == 1 {
        format!(
            "{n} bursts among {}, all {}; the longest ran {}.",
            escape_html(scope),
            escape_html(busiest),
            duration(longest.secs),
        )
    } else {
        format!(
            "{n} bursts among {}: {} had the most ({k}), and the longest, {}, was {}\u{2019}s.",
            escape_html(scope),
            escape_html(busiest),
            duration(longest.secs),
            escape_html(longest.species),
        )
    })
}

/// How many of `retention` are each kind of visitor, as `[residents, regulars, migrants,
/// rarities]`: the order [`retention_sentence`] reads them in.
#[must_use]
pub(super) fn kind_counts(retention: &[SpeciesRetention]) -> [usize; 4] {
    let mut counts = [0; 4];
    for r in retention {
        counts[match r.classification {
            ResidencyType::Resident => 0,
            ResidencyType::Regular => 1,
            ResidencyType::Migrant => 2,
            ResidencyType::Rarity => 3,
        }] += 1;
    }
    counts
}

/// What the return-rate ledger found: how many species fall in each kind of visitor, as
/// `[residents, regulars, migrants, rarities]`, among those heard at least `min_detections` times
/// (the ledger lists no others). `None` for fewer than [`MIN_SPECIES`] in all.
#[must_use]
pub(super) fn retention_sentence(counts: [usize; 4], min_detections: u32) -> Option<String> {
    let total: usize = counts.iter().sum();
    if total < MIN_SPECIES {
        return None;
    }
    let kinds = [
        ("resident", "residents"),
        ("regular", "regulars"),
        ("migrant", "migrants"),
        ("rarity", "rarities"),
    ];
    let parts: Vec<String> = counts
        .iter()
        .zip(kinds)
        .filter(|(n, _)| **n > 0)
        .map(|(n, (one, many))| format!("{n} {}", if *n == 1 { one } else { many }))
        .collect();
    let list = match parts.as_slice() {
        [] => return None,
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    };
    Some(format!(
        "Of {total} species heard at least {min_detections} times: {list}."
    ))
}

/// What the neighbouring-species figure found about `trigger` (as the station names it): the bird
/// most often heard right after it and the bird most often heard right before it, each with the
/// number of sessions it was and its share.
///
/// The share is of the sessions that had a bird on that side at all, which is what the engine's
/// probability is (a session the bird opens has no bird before it, and one it closes none after),
/// not of every session the bird was heard in, so the sentence says so. `None` when there is
/// neither side.
#[must_use]
pub(super) fn neighbour_sentence(
    trigger: &str,
    before: &[Neighbour],
    after: &[Neighbour],
) -> Option<String> {
    let top = |rows: &[Neighbour], side: &str| -> Option<String> {
        let n = rows.iter().max_by(|a, b| {
            a.probability
                .total_cmp(&b.probability)
                .then_with(|| b.name.cmp(&a.name))
        })?;
        let who = if n.is_trigger {
            "the same bird again".to_owned()
        } else {
            escape_html(&n.name)
        };
        let sessions = if n.frequency == 1 {
            "1 session".to_owned()
        } else {
            format!("{} sessions", n.frequency)
        };
        Some(format!(
            "{who} ({sessions}, {:.0}% of those with a bird {side})",
            n.probability * 100.0
        ))
    };
    let next = top(after, "after");
    let previous = top(before, "before");
    let me = escape_html(trigger);
    match (next, previous) {
        (Some(n), Some(p)) => Some(format!(
            "Right after {me}, the next bird is most often {n}; right before it, {p}."
        )),
        (Some(n), None) => Some(format!(
            "Right after {me}, the next bird is most often {n}."
        )),
        (None, Some(p)) => Some(format!(
            "Right before {me}, the bird heard is most often {p}."
        )),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> String {
        format!("{s}s")
    }

    fn burst(species: &str, secs: u64) -> Burst<'_> {
        Burst { species, secs }
    }

    fn species(kind: ResidencyType) -> SpeciesRetention {
        SpeciesRetention {
            species: "x".to_owned(),
            retention_rates: Vec::new(),
            weeks_present: 1,
            station_weeks: 1,
            classification: kind,
        }
    }

    /// Four distinct counts, so a count filed under the wrong kind cannot go unseen: the sentence
    /// reads `[residents, regulars, migrants, rarities]` and so must the counting.
    #[test]
    fn each_kind_of_visitor_is_counted_in_its_own_place() {
        use ResidencyType::{Migrant, Rarity, Regular, Resident};
        let rows: Vec<SpeciesRetention> = [
            vec![Resident; 4],
            vec![Regular; 3],
            vec![Migrant; 2],
            vec![Rarity; 1],
        ]
        .concat()
        .into_iter()
        .map(species)
        .collect();
        assert_eq!(kind_counts(&rows), [4, 3, 2, 1]);
        assert_eq!(kind_counts(&[]), [0, 0, 0, 0]);
    }

    fn nb(name: &str, probability: f64, is_trigger: bool) -> Neighbour {
        nbf(name, probability, 1, is_trigger)
    }

    fn nbf(name: &str, probability: f64, frequency: u64, is_trigger: bool) -> Neighbour {
        Neighbour {
            name: name.to_owned(),
            probability,
            frequency,
            is_trigger,
        }
    }

    /// The most bursts and the longest are different questions with different answers, and the
    /// sentence says both, with its basis.
    #[test]
    fn the_bursts_sentence_names_the_busiest_and_the_longest() {
        let b = [
            burst("Robin", 60),
            burst("Robin", 120),
            burst("Wren", 900),
            burst("Robin", 30),
        ];
        assert_eq!(
            bursts_sentence(&b, "the 100 newest sessions", secs).as_deref(),
            Some(
                "4 bursts among the 100 newest sessions: Robin had the most (3), and the longest, 900s, was Wren\u{2019}s."
            )
        );
    }

    /// A tie is decided by name, whichever order the bursts arrive in; a single species is said
    /// as that, not as "had the most".
    #[test]
    fn a_tie_goes_to_the_first_name_and_one_species_is_said_once() {
        let a = [
            burst("Wren", 10),
            burst("Robin", 10),
            burst("Wren", 5),
            burst("Robin", 5),
        ];
        let b = [
            burst("Robin", 5),
            burst("Wren", 5),
            burst("Robin", 10),
            burst("Wren", 10),
        ];
        let (sa, sb) = (
            bursts_sentence(&a, "all 4 sessions", secs),
            bursts_sentence(&b, "all 4 sessions", secs),
        );
        assert_eq!(sa, sb, "the order the bursts arrived in does not matter");
        let sa = sa.expect("a sentence");
        assert!(
            sa.contains("Robin had the most (2)") && sa.contains("was Robin\u{2019}s"),
            "{sa}"
        );
        let one = [burst("Wren", 10), burst("Wren", 70), burst("Wren", 5)];
        assert_eq!(
            bursts_sentence(&one, "all 3 sessions", secs).as_deref(),
            Some("3 bursts among all 3 sessions, all Wren; the longest ran 70s.")
        );
    }

    #[test]
    fn two_bursts_are_a_coincidence_and_say_nothing() {
        assert_eq!(
            bursts_sentence(&[burst("Robin", 60), burst("Wren", 5)], "x", secs),
            None
        );
        assert_eq!(bursts_sentence(&[], "x", secs), None);
        assert!(
            bursts_sentence(&[burst("a", 1), burst("b", 2), burst("c", 3)], "x", secs).is_some(),
            "three are enough"
        );
    }

    #[test]
    fn names_in_the_sentences_are_escaped() {
        let b = [burst("<b>x</b>", 60), burst("<b>x</b>", 5), burst("y", 1)];
        let s = bursts_sentence(&b, "<s>", secs).expect("a sentence");
        assert!(!s.contains("<b>") && !s.contains("<s>"), "{s}");
        let n = neighbour_sentence("<t>", &[nb("<u>", 0.5, false)], &[nb("<v>", 0.5, false)])
            .expect("a sentence");
        assert!(
            !n.contains("<t>") && !n.contains("<u>") && !n.contains("<v>"),
            "{n}"
        );
    }

    /// Kinds with none are left out, one is not a plural, the last is joined with "and", and the
    /// sentence says which species the ledger lists.
    #[test]
    fn the_ledger_sentence_counts_each_kind_of_visitor() {
        assert_eq!(
            retention_sentence([9, 6, 5, 4], 5).as_deref(),
            Some(
                "Of 24 species heard at least 5 times: 9 residents, 6 regulars, 5 migrants and 4 rarities."
            )
        );
        assert_eq!(
            retention_sentence([4, 0, 1, 0], 5).as_deref(),
            Some("Of 5 species heard at least 5 times: 4 residents and 1 migrant.")
        );
        assert_eq!(
            retention_sentence([5, 0, 0, 0], 3).as_deref(),
            Some("Of 5 species heard at least 3 times: 5 residents.")
        );
        assert_eq!(
            retention_sentence([2, 1, 1, 0], 5),
            None,
            "four species are too few to describe"
        );
    }

    /// The bird most often next and most often before, with the sessions and the share, and the
    /// basis of the share (sessions with a bird on that side, not every session); the trigger
    /// following itself is said as that; and either side may be missing.
    #[test]
    fn the_neighbour_sentence_names_who_comes_next_and_who_came_before() {
        let before = [nbf("Wren", 0.1, 1, false), nbf("Finch", 0.25, 4, false)];
        let after = [nbf("Jay", 0.2, 1, false), nbf("Robin", 0.4, 2, true)];
        assert_eq!(
            neighbour_sentence("Robin", &before, &after).as_deref(),
            Some(
                "Right after Robin, the next bird is most often the same bird again (2 sessions, 40% of those with a bird after); right before it, Finch (4 sessions, 25% of those with a bird before)."
            )
        );
        let after = [nbf("Jay", 0.2, 1, false), nbf("Robin", 0.4, 2, false)];
        assert_eq!(
            neighbour_sentence("Dove", &before, &after).as_deref(),
            Some(
                "Right after Dove, the next bird is most often Robin (2 sessions, 40% of those with a bird after); right before it, Finch (4 sessions, 25% of those with a bird before)."
            )
        );
        assert_eq!(
            neighbour_sentence("Dove", &[], &after).as_deref(),
            Some(
                "Right after Dove, the next bird is most often Robin (2 sessions, 40% of those with a bird after)."
            )
        );
        assert_eq!(
            neighbour_sentence("Dove", &before, &[]).as_deref(),
            Some(
                "Right before Dove, the bird heard is most often Finch (4 sessions, 25% of those with a bird before)."
            )
        );
        assert_eq!(neighbour_sentence("Dove", &[], &[]), None);
        // One session is not a plural.
        assert!(
            neighbour_sentence("Dove", &[], &[nbf("Jay", 1.0, 1, false)])
                .is_some_and(|s| s.contains("(1 session, 100% of those"))
        );
        // The order of the rows does not matter, and a tie goes to the first name.
        assert_eq!(
            neighbour_sentence("T", &[], &[nb("b", 0.3, false), nb("a", 0.3, false)]),
            neighbour_sentence("T", &[], &[nb("a", 0.3, false), nb("b", 0.3, false)])
        );
        let tie = neighbour_sentence("T", &[], &[nb("b", 0.3, false), nb("a", 0.3, false)])
            .expect("a sentence");
        assert!(tie.contains("most often a ("), "{tie}");
    }
}
