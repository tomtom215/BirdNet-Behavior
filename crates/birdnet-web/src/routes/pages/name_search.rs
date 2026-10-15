//! Finding species by the name a person would type.
//!
//! The database matches a typed term against the classifier's common name and
//! the scientific name, which is every name there is while the station shows
//! the classifier's own. A station set to German shows *Kohlmeise*, and a reader
//! who types "kohl" expects to find it; the stored name is *Great Tit*, so the
//! database finds nothing. This adds the species whose name in the station's
//! language contains the term, in the web layer, so the database's search — a
//! grammar with its own `NOT` form and its own tests — is untouched.

use std::collections::HashSet;

use birdnet_core::i18n::NameLocalizer;
use birdnet_db::sqlite::{
    DateRange, DbError, DetectionFilter, SearchTerm, SpeciesCount, TodayFilter, parse_search_term,
};
use rusqlite::Connection;

/// The most species a typed term is allowed to add to a search. A term that names
/// more than this (one letter, say) means nothing as a search, and an `IN` list
/// of thousands only slows the query that was going to match nearly everything.
const MOST_SPECIES_ADDED: usize = 500;

/// The scientific names of the species whose name in the station's language
/// contains the typed `search` term, for the detection searches to cover as well
/// as the names the database holds. Empty for an English station, for no term,
/// and for a term nothing matches — where a search behaves exactly as it always
/// has.
///
/// Both forms of the term count: `Elster` finds the magpie, and `NOT Elster`
/// leaves it out.
pub fn also_species(names: &NameLocalizer, search: Option<&str>) -> Vec<String> {
    let Some(SearchTerm::Include(term) | SearchTerm::Exclude(term)) = parse_search_term(search)
    else {
        return Vec::new();
    };
    names
        .scientific_names_matching(&term)
        .into_iter()
        .take(MOST_SPECIES_ADDED)
        .map(str::to_owned)
        .collect()
}

/// The Today log's filter — one date, a category shortcut, an optional search —
/// with the typed search also covering the station's own language.
///
/// The same filter `todays_detections` builds, and no other: the log, its count
/// and its "load more" have to agree about what they are counting.
pub fn todays_filter(
    date: &str,
    search: Option<&str>,
    category: TodayFilter,
    names: &NameLocalizer,
) -> DetectionFilter {
    DetectionFilter {
        text: search.map(str::to_owned),
        text_also_species: also_species(names, search),
        dates: DateRange::On(date.to_owned()),
        category,
        ..DetectionFilter::default()
    }
}

/// The stored common name for a name a person typed, when they typed it in the
/// station's language.
///
/// For the lookups that take one species by name (the companion lookup): the
/// database has only the classifier's names, so *Amsel* has to be turned back
/// into *Common Blackbird* before it can be asked about. A typed name the
/// database already knows (as a common or scientific name, any case) is left as
/// typed — it wins over a translation that happens to spell the same. One that
/// is the name of no species, or of two, is also left as typed, and finds what
/// it always found.
pub fn stored_name_for_typed(conn: &Connection, names: &NameLocalizer, typed: &str) -> String {
    let known = conn
        .query_row(
            "SELECT 1 FROM detections WHERE Com_Name = ?1 COLLATE NOCASE \
             OR Sci_Name = ?1 COLLATE NOCASE LIMIT 1",
            [typed],
            |_| Ok(()),
        )
        .is_ok();
    if known {
        return typed.to_owned();
    }
    names
        .scientific_name_for(typed)
        .and_then(|sci| {
            conn.query_row(
                "SELECT Com_Name FROM detections WHERE Sci_Name = ?1 LIMIT 1",
                [sci],
                |r| r.get::<_, String>(0),
            )
            .ok()
        })
        .unwrap_or_else(|| typed.to_owned())
}

/// Species matching `term` by stored name, scientific name, or — when `names`
/// is not English — by their name in the station's language, commonest first.
///
/// For an English station this is `search_species` and nothing else, so the
/// default behaviour is the one that has always been tested.
///
/// # Errors
///
/// A failed query is returned, not defaulted to "no species".
pub fn search_species(
    conn: &Connection,
    names: &NameLocalizer,
    term: &str,
    limit: u32,
) -> Result<Vec<SpeciesCount>, DbError> {
    let mut found = birdnet_db::sqlite::search_species(conn, term, limit)?;
    let in_language = names.scientific_names_matching(term);
    if in_language.is_empty() {
        return Ok(found);
    }
    let have: HashSet<&str> = found.iter().map(|s| s.sci_name.as_str()).collect();
    let wanted: HashSet<&str> = in_language
        .into_iter()
        .filter(|sci| !have.contains(sci))
        .collect();
    if wanted.is_empty() {
        return Ok(found);
    }
    // 10 000 is more species than any station has heard; the list is already
    // aggregated in `species_summary`, so this is one small scan.
    let all = birdnet_db::sqlite::top_species(conn, 10_000)?;
    found.extend(
        all.into_iter()
            .filter(|s| wanted.contains(s.sci_name.as_str())),
    );
    found.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.com_name.cmp(&b.com_name))
    });
    found.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(found)
}
