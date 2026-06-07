//! Free-text search-term parsing for the detections list.
//!
//! Pure string logic (no database access): turns the operator's search box
//! value into an [`SearchTerm`] the read queries format into a SQL `LIKE` /
//! `NOT LIKE` clause.

/// What the operator meant by a free-text search term.
///
/// Public for unit-test access. `Exclude` carries the rest of the term
/// (post-`"NOT "` prefix, trimmed) so the caller can format it directly
/// into a SQL LIKE pattern.
#[derive(Debug, PartialEq, Eq)]
pub enum SearchTerm {
    /// The whole term is a `Com_Name LIKE %term% OR Sci_Name LIKE %term%`
    /// inclusion pattern.
    Include(String),
    /// The term begins case-insensitively with `"NOT "` and has at least
    /// one non-whitespace character after it. The carried string is the
    /// content after the prefix, trimmed; the caller wraps it as
    /// `Com_Name NOT LIKE %term%`.
    Exclude(String),
}

/// Parse an operator-supplied search box value into [`SearchTerm`].
///
/// `None` if the input is `None`, empty, or whitespace-only — the caller
/// should drop the WHERE clause entirely in that case.
///
/// The `"NOT "` prefix is the legacy BirdNET-Pi exclusion syntax. We use
/// `str::strip_prefix_ignore_ascii_case` rather than `s.len() > 4 &&
/// s[..4].eq_ignore_ascii_case("NOT ")` because the second form has an
/// equivalent mutant on the length comparison: with the calling code's
/// up-front `.trim()`, a 4-char input ending in space is unreachable, so
/// `> 4` and `>= 4` produce identical observable behaviour. Eliminating
/// the explicit length comparison eliminates the mutant. Tracked in the
/// `parse_search_term_*` tests below.
#[must_use]
pub fn parse_search_term(raw: Option<&str>) -> Option<SearchTerm> {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if let Some(rest) = strip_not_prefix(trimmed) {
        let rest = rest.trim();
        if !rest.is_empty() {
            return Some(SearchTerm::Exclude(rest.to_string()));
        }
    }
    Some(SearchTerm::Include(trimmed.to_string()))
}

/// Strip a case-insensitive `"NOT "` prefix.
///
/// Returns `Some(&s[4..])` if `s` is at least 4 bytes long and the first
/// four bytes are ASCII-equal-ignore-case to `"NOT "`. Otherwise `None`.
///
/// This uses `s.get(..4)` rather than a length comparison so cargo-mutants
/// has no `>` / `>=` boundary to flip — the existence check is implicit
/// in the `Option` return. The unit test pins every cell of the case-
/// insensitive prefix match table.
fn strip_not_prefix(s: &str) -> Option<&str> {
    let head = s.get(..4)?;
    if head.eq_ignore_ascii_case("NOT ") {
        Some(&s[4..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_term_none_or_empty_returns_none() {
        assert_eq!(parse_search_term(None), None);
        assert_eq!(parse_search_term(Some("")), None);
        assert_eq!(parse_search_term(Some("   ")), None);
        assert_eq!(parse_search_term(Some("\t\n")), None);
    }

    #[test]
    fn parse_search_term_plain_word_is_include() {
        assert_eq!(
            parse_search_term(Some("Robin")),
            Some(SearchTerm::Include("Robin".into()))
        );
        // Leading / trailing whitespace is trimmed before the dispatch.
        assert_eq!(
            parse_search_term(Some("  Robin  ")),
            Some(SearchTerm::Include("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_not_prefix_is_exclude() {
        assert_eq!(
            parse_search_term(Some("NOT Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        // Case-insensitive prefix match: "not", "Not", "NoT" all work.
        assert_eq!(
            parse_search_term(Some("not Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        assert_eq!(
            parse_search_term(Some("Not Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
        assert_eq!(
            parse_search_term(Some("nOt Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_not_prefix_trims_remainder() {
        // The remainder is trimmed too — "NOT   Robin   " excludes
        // "Robin", not "  Robin  ".
        assert_eq!(
            parse_search_term(Some("NOT   Robin")),
            Some(SearchTerm::Exclude("Robin".into()))
        );
    }

    #[test]
    fn parse_search_term_lone_not_degrades_to_include() {
        // The 3-char "NOT" doesn't have the trailing space so doesn't
        // match the prefix → include "NOT".
        assert_eq!(
            parse_search_term(Some("NOT")),
            Some(SearchTerm::Include("NOT".into()))
        );
        // "NOT " with the literal trailing space SHOULD be unreachable
        // in practice because the function trims its input. But even if
        // a caller bypasses the trim, an empty remainder degrades to an
        // inclusion of the original-trimmed string ("NOT") rather than
        // collapsing to an exclude-everything that would return 0
        // rows. The helper assumes its caller has already trimmed, so
        // we pass "NOT" (3 chars) here — the trim invariant is what
        // makes "NOT " (with trailing space) unreachable from the
        // public-API surface.
        assert_eq!(
            parse_search_term(Some("NOT ")),
            // After the helper's own trim, "NOT" — the strip prefix
            // requires at least 4 bytes, so this falls through to
            // include "NOT".
            Some(SearchTerm::Include("NOT".into()))
        );
    }

    #[test]
    fn parse_search_term_short_strings_are_include() {
        // Any string shorter than 4 bytes can't have a "NOT " prefix.
        assert_eq!(
            parse_search_term(Some("a")),
            Some(SearchTerm::Include("a".into()))
        );
        assert_eq!(
            parse_search_term(Some("NOT")),
            Some(SearchTerm::Include("NOT".into()))
        );
        assert_eq!(
            parse_search_term(Some("not")),
            Some(SearchTerm::Include("not".into()))
        );
    }

    #[test]
    fn parse_search_term_notx_is_include_not_exclude() {
        // 4 chars but no trailing space — first 4 chars are "NOTX",
        // which doesn't equal "NOT " ignoring case → include path.
        assert_eq!(
            parse_search_term(Some("NOTX")),
            Some(SearchTerm::Include("NOTX".into()))
        );
        // Same with 5 chars where the 4th is non-space.
        assert_eq!(
            parse_search_term(Some("NOTOK")),
            Some(SearchTerm::Include("NOTOK".into()))
        );
    }

    #[test]
    fn parse_search_term_multibyte_input_does_not_panic() {
        // The helper uses `s.get(..4)` which never panics on a
        // non-char-boundary slice — it just returns None. Pin the
        // contract so a future refactor can't reintroduce the
        // pre-helper `s[..4]` slice that would panic on a 2-byte
        // emoji.
        assert_eq!(
            parse_search_term(Some("∅Owl")), // 4 bytes (∅) + 3 chars = 6 bytes
            Some(SearchTerm::Include("∅Owl".into()))
        );
        // A pure-multibyte string shorter than 4 bytes.
        assert_eq!(
            parse_search_term(Some("ω")), // 2 bytes
            Some(SearchTerm::Include("ω".into()))
        );
    }
}
