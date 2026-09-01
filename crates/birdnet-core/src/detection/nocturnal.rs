//! Which birds are plausibly heard in the middle of the night.
//!
//! # Why this exists
//!
//! A blue tit "detected" at 02:30 is almost always the classifier hearing
//! something else — a car door, a bat, rain on a leaf — and reaching for the
//! nearest bird. Those detections accumulate quietly, and because they are
//! spread across the whole night they distort exactly the analytics people
//! build stations to look at: the dawn-chorus curve, per-species activity
//! histograms, sessionisation.
//!
//! A blanket "no birds at night" rule would be worse than the problem. Owls,
//! nightjars, rails, bitterns and thick-knees call at night on purpose, and
//! they are the detections an operator most wants. So the question is not
//! *when* but *who*.
//!
//! # What this can and cannot know
//!
//! Classification is by **genus**, parsed from the first word of the
//! scientific name. That is the finest unit a binomial gives without a
//! taxonomy table, and it is a real limit: it cannot express "this genus is
//! nocturnal only on migration", and a genus split differently by a later
//! checklist will not match.
//!
//! The list below is therefore not, and cannot be, complete. Two things follow,
//! and both are load-bearing:
//!
//! * **The caller quarantines rather than drops.** Every verdict here is
//!   recoverable by an operator looking at the review queue. A filter that
//!   deleted on this evidence would be trading one silent error for another.
//! * **[`NightVerdict::Unknown`] is not a synonym for "diurnal".** A genus
//!   nobody has classified is reported as such, so the caller can decide —
//!   and so this file cannot quietly become the authority on 11 000 species
//!   it has never been checked against.
//!
//! # Nocturnal flight calls
//!
//! Thrushes, warblers, sparrows and waders migrate at night and call while
//! they do. Recording those calls is an entire field of the hobby, and on a
//! station doing it this filter is exactly wrong: it would quarantine the
//! whole point of the recording. Such a station should leave the filter off,
//! or name the genera it is listening for in the operator's own allow-list.

/// What is known about a species' night-time activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightVerdict {
    /// The genus is expected to be heard at night — owls, nightjars, rails.
    Nocturnal,
    /// The genus is not known to be nocturnal here.
    ///
    /// Deliberately not called `Diurnal`: this file has not been checked
    /// against the model's full label set, and a name that claimed more than
    /// is known would invite a caller to act more strongly than the evidence
    /// supports.
    Unknown,
}

/// Genera whose members are expected to be heard after dark.
///
/// Grouped by what they are, so a reviewer can check a block against a
/// checklist rather than a flat list of 150 words. Genus names only — the
/// species part of a binomial is never consulted.
const NOCTURNAL_GENERA: &[&str] = &[
    // ── Owls (Strigiformes) ──
    "tyto",
    "phodilus",
    "otus",
    "megascops",
    "psiloscops",
    "gymnoglaux",
    "ptilopsis",
    "bubo",
    "ketupa",
    "scotopelia",
    "pulsatrix",
    "strix",
    "ciccaba",
    "jubula",
    "lophostrix",
    "surnia",
    "glaucidium",
    "xenoglaux",
    "micrathene",
    "athene",
    "aegolius",
    "ninox",
    "uroglaux",
    "sceloglaux",
    "nesasio",
    "pseudoscops",
    "asio",
    "margarobyas",
    "taenioptynx",
    "heteroglaux",
    // ── Nightjars and allies (Caprimulgiformes) ──
    "caprimulgus",
    "antrostomus",
    "chordeiles",
    "nyctidromus",
    "phalaenoptilus",
    "nyctiphrynus",
    "siphonorhis",
    "hydropsalis",
    "uropsalis",
    "macropsalis",
    "eleothreptus",
    "lurocalis",
    "nyctipolus",
    "systellura",
    "setopagis",
    "nyctiprogne",
    "eurostopodus",
    "lyncornis",
    "gactornis",
    "podager",
    "veles",
    // Frogmouths, potoos, oilbird, owlet-nightjars
    "batrachostomus",
    "podargus",
    "rigidipenna",
    "nyctibius",
    "steatornis",
    "aegotheles",
    // ── Rails and crakes (Rallidae) — many are far more vocal at night ──
    "rallus",
    "crex",
    "porzana",
    "zapornia",
    "coturnicops",
    "laterallus",
    "micropygia",
    "rallina",
    "gallirallus",
    "hypotaenidia",
    "lewinia",
    "amaurornis",
    "aramides",
    "pardirallus",
    "mustelirallus",
    "atlantisia",
    "sarothrura",
    "porphyrio",
    "gallinula",
    "fulica",
    "anurolimnas",
    // ── Bitterns and night-herons ──
    "botaurus",
    "ixobrychus",
    "nycticorax",
    "nyctanassa",
    "gorsachius",
    "cochlearius",
    // ── Thick-knees ──
    "burhinus",
    "esacus",
    // ── Woodcock and snipe: crepuscular display flights running well into the night ──
    "scolopax",
    "gallinago",
    "lymnocryptes",
    "coenocorypha",
    // ── Petrels, shearwaters and storm-petrels: colony visits are nocturnal ──
    "hydrobates",
    "oceanodroma",
    "oceanites",
    "pelagodroma",
    "fregetta",
    "garrodia",
    "nesofregetta",
    "puffinus",
    "ardenna",
    "calonectris",
    "pterodroma",
    "bulweria",
    "pelecanoides",
    "procellaria",
    // ── Kiwi ──
    "apteryx",
    // ── Others long established as night-callers ──
    "aramus",     // Limpkin
    "nyctanassa", // Yellow-crowned Night Heron (also above; harmless duplicate)
    "hydrobates", // (also above)
    "megapodius", // Megapodes call through the night
    "leipoa",
    "alectura",
];

/// The genus part of a scientific name, lowercased.
///
/// `None` for anything that is not `Genus species`. The label set is not all
/// binomials — the V3.0 CSV carries entries whose `sci_name` repeats a common
/// name, and the non-bird classes (`Dog`, `Siren`) are single words — and
/// treating a one-word label as a genus would silently classify it.
#[must_use]
pub fn genus_of(scientific_name: &str) -> Option<String> {
    let mut parts = scientific_name.split_whitespace();
    let genus = parts.next()?;
    // A binomial has a species epithet. Without one this is not a species name.
    parts.next()?;
    if !genus.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(genus.to_ascii_lowercase())
}

/// Whether this species is expected to be heard in the middle of the night.
///
/// `extra_nocturnal` is the operator's own list, matched against the genus or
/// the full scientific name, case-insensitively. It exists because the genus
/// table cannot be complete and a station that hears a particular bird at
/// night every week should not have to keep approving it.
#[must_use]
pub fn night_verdict(scientific_name: &str, extra_nocturnal: &[String]) -> NightVerdict {
    let name = scientific_name.trim();
    let genus = genus_of(name);

    for entry in extra_nocturnal {
        let entry = entry.trim();
        // A blank entry cannot match a real name by equality, so this guard
        // looks redundant and is not: `name` is trimmed, so a label that is
        // only whitespace becomes `""` too, and a blank line in the operator's
        // list would then make every such detection nocturnal.
        if entry.is_empty() {
            continue;
        }
        if entry.eq_ignore_ascii_case(name)
            || genus
                .as_deref()
                .is_some_and(|g| entry.eq_ignore_ascii_case(g))
        {
            return NightVerdict::Nocturnal;
        }
    }

    match genus {
        Some(g) if NOCTURNAL_GENERA.contains(&g.as_str()) => NightVerdict::Nocturnal,
        _ => NightVerdict::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{NOCTURNAL_GENERA, NightVerdict, genus_of, night_verdict};

    /// The verdict with no operator exemptions.
    fn verdict(sci: &str) -> NightVerdict {
        night_verdict(sci, &[])
    }

    // ── genus parsing ───────────────────────────────────────────────────

    #[test]
    fn a_binomial_yields_its_genus_lowercased() {
        assert_eq!(genus_of("Strix aluco").as_deref(), Some("strix"));
        assert_eq!(genus_of("  strix   aluco  ").as_deref(), Some("strix"));
        assert_eq!(genus_of("Tyto alba javanica").as_deref(), Some("tyto"));
    }

    #[test]
    fn a_label_that_is_not_a_binomial_has_no_genus() {
        // The label set is not all binomials: the non-bird classes are single
        // words (`Dog`, `Siren`), and the V3.0 CSV carries entries whose
        // `sci_name` repeats a common name. Treating a one-word label as a
        // genus would classify it — and `Dog` is one letter from no genus at
        // all, so the failure would be quiet.
        for label in ["Dog", "Siren", "Noise", "", "   "] {
            assert_eq!(genus_of(label), None, "{label:?} was read as a genus");
        }
    }

    #[test]
    fn a_genus_with_non_alphabetic_characters_is_rejected() {
        // Guards against a stray identifier or a numbered class name being
        // matched against the table.
        assert_eq!(genus_of("Strix2 aluco"), None);
        assert_eq!(genus_of("sp. indet"), None);
    }

    // ── the verdict ─────────────────────────────────────────────────────

    #[test]
    fn the_night_callers_are_classified_as_nocturnal() {
        // One per block of the table, so a block deleted wholesale is caught.
        for sci in [
            "Strix aluco",           // owl
            "Tyto alba",             // barn owl
            "Asio otus",             // long-eared owl
            "Caprimulgus europaeus", // nightjar
            "Chordeiles minor",      // nighthawk
            "Nyctibius griseus",     // potoo
            "Rallus aquaticus",      // rail
            "Crex crex",             // corncrake
            "Botaurus stellaris",    // bittern
            "Nycticorax nycticorax", // night heron
            "Burhinus oedicnemus",   // thick-knee
            "Scolopax rusticola",    // woodcock
            "Puffinus puffinus",     // shearwater
            "Apteryx australis",     // kiwi
            "Aramus guarauna",       // limpkin
        ] {
            assert_eq!(
                verdict(sci),
                NightVerdict::Nocturnal,
                "{sci} was not classified as a night caller"
            );
        }
    }

    #[test]
    fn an_ordinary_day_bird_is_not_classified_as_nocturnal() {
        // Counterpart: a table that answered `Nocturnal` to everything would
        // satisfy the gate above and make the whole filter a no-op.
        for sci in [
            "Cyanistes caeruleus",
            "Turdus merula",
            "Parus major",
            "Erithacus rubecula",
            "Passer domesticus",
        ] {
            assert_eq!(
                verdict(sci),
                NightVerdict::Unknown,
                "{sci} was classified as a night caller"
            );
        }
    }

    #[test]
    fn the_species_epithet_is_never_consulted() {
        // Matching on the whole binomial, or on any word in it, would classify
        // by coincidence: `Otus` is an owl genus, but `otus` is also the
        // epithet of `Asio otus` — and of nothing diurnal that would show it.
        // A clearer case: an invented species in a diurnal genus whose epithet
        // happens to be an owl genus name must not be admitted.
        assert_eq!(verdict("Turdus strix"), NightVerdict::Unknown);
        assert_eq!(verdict("Parus tyto"), NightVerdict::Unknown);
    }

    #[test]
    fn classification_ignores_case_and_surrounding_space() {
        assert_eq!(verdict("  STRIX ALUCO  "), NightVerdict::Nocturnal);
        assert_eq!(verdict("strix aluco"), NightVerdict::Nocturnal);
    }

    // ── the operator's own list ─────────────────────────────────────────

    #[test]
    fn an_operator_entry_can_name_a_species_or_a_genus() {
        let by_species = ["Cyanistes caeruleus".to_owned()];
        assert_eq!(
            night_verdict("Cyanistes caeruleus", &by_species),
            NightVerdict::Nocturnal
        );
        // ...and does not spill onto the rest of the genus.
        assert_eq!(
            night_verdict("Cyanistes cyanus", &by_species),
            NightVerdict::Unknown
        );

        let by_genus = ["Catharus".to_owned()];
        assert_eq!(
            night_verdict("Catharus ustulatus", &by_genus),
            NightVerdict::Nocturnal,
            "a genus entry did not cover its species"
        );
        assert_eq!(
            night_verdict("Turdus merula", &by_genus),
            NightVerdict::Unknown
        );
    }

    #[test]
    fn operator_entries_ignore_case_and_blank_lines() {
        let list = [String::new(), "  ".to_owned(), "  cyanistes  ".to_owned()];
        assert_eq!(
            night_verdict("Cyanistes caeruleus", &list),
            NightVerdict::Nocturnal
        );
        assert_eq!(night_verdict("Turdus merula", &list), NightVerdict::Unknown);
        // A label that trims to nothing must not be matched *by* the blank
        // entries. `name` is trimmed, so without the guard `"" == ""` holds
        // and every nameless label would be admitted at night.
        assert_eq!(night_verdict("   ", &list), NightVerdict::Unknown);
        assert_eq!(night_verdict("", &list), NightVerdict::Unknown);
    }

    // ── the table itself ────────────────────────────────────────────────

    #[test]
    fn the_genus_table_is_lowercase_and_alphabetic() {
        // Lookup lowercases the parsed genus, so an entry with a capital can
        // never match — silently, since the only symptom is an owl being
        // quarantined for hooting.
        for g in NOCTURNAL_GENERA {
            assert!(
                g.chars().all(|c| c.is_ascii_lowercase()),
                "{g:?} cannot ever match: the lookup lowercases its input"
            );
        }
    }
}
