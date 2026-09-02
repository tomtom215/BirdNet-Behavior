//! The manual quotes a live `/api/v2/health` response, and that sample carries
//! a version string. Nothing regenerates it, so it drifts silently — which is
//! exactly what happened: it sat at `0.13.1` through both the `0.14.0` and the
//! `0.15.0` version bumps, even though `RELEASING.md`'s pre-release checklist
//! names the line explicitly ("docs/book/reference/api.md: the sample
//! /api/v2/health response"). A checklist item that is only ever checked by a
//! human is one that gets missed twice without anyone noticing.
//!
//! `crates/birdnet-web/src/routes/openapi.rs` already gates `openapi.json`'s
//! `info.version` against `CARGO_PKG_VERSION` the same way. This is that gate
//! for the prose copy.
//!
//! Observed failing against the stale file before `api.md` was corrected:
//!
//! ```text
//! ---- the_manuals_health_sample_reports_this_builds_version stdout ----
//! assertion `left == right` failed: docs/book/reference/api.md's
//! /api/v2/health sample reports version "0.13.1", but this build is
//! "0.15.0" — RELEASING.md's checklist covers this line; update the sample
//!   left: "0.13.1"
//!  right: "0.15.0"
//! ```

use std::path::{Path, PathBuf};

/// The manual, compiled in — so the test moves with the file rather than
/// depending on the working directory a runner happens to use.
const API_DOC: &str = include_str!("../docs/book/reference/api.md");

/// The `"version": "..."` literal from the documented `/api/v2/health` body.
///
/// Returns `None` rather than panicking so the caller can tell "the sample
/// changed shape" apart from "the sample is stale" — a matcher that quietly
/// finds nothing would let this test pass for free, which is the failure mode
/// the whole file exists to prevent.
fn documented_health_version() -> Option<&'static str> {
    let after_path = API_DOC.split("/api/v2/health").nth(1)?;
    let field = after_path.find("\"version\":")?;
    let rest = &after_path[field + "\"version\":".len()..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(&rest[open..close])
}

#[test]
fn the_manuals_health_sample_reports_this_builds_version() {
    let documented = documented_health_version().expect(
        "docs/book/reference/api.md must still show a \"version\" field in its \
         /api/v2/health sample — if the sample moved or was reshaped, retarget \
         this gate rather than deleting it",
    );

    assert_eq!(
        documented,
        env!("CARGO_PKG_VERSION"),
        "docs/book/reference/api.md's /api/v2/health sample reports version \
         {documented:?}, but this build is {:?} — RELEASING.md's checklist \
         covers this line; update the sample",
        env!("CARGO_PKG_VERSION")
    );
}

/// The counterpart. The gate above can only be trusted if its matcher actually
/// locates the sample; an extractor that returns `None` on everything would
/// turn the real assertion into an `expect` that never runs and a test that is
/// green because it tested nothing.
///
/// Kept green rather than red: it asserts the matcher finds *a* plausible
/// version, without asserting *which* — so it stays passing across every future
/// bump while still failing the day the sample stops being found.
#[test]
fn the_version_matcher_actually_finds_the_sample() {
    let found = documented_health_version();
    assert!(
        found.is_some(),
        "the matcher found no version field in api.md's /api/v2/health sample, \
         so the gate beside it would pass vacuously"
    );
    let found = found.unwrap_or_default();
    assert!(
        found.split('.').count() == 3 && found.split('.').all(|p| !p.is_empty()),
        "expected a dotted version triple from the sample, got {found:?} — the \
         matcher is picking up the wrong string"
    );
}

// ── The tuning guide's confirmation-level table ──────────────────────────────
//
// `docs/book/guides/tuning.md` prints, for each confirmation level, the
// fraction it demands, the span it looks across, and the smallest overlap at
// which it demands anything at all. Every one of those numbers is derived —
// `ConfirmationLevel::minimum_overlap` searches for it by asking the filter the
// same question the filter will ask at run time — so the table is a transcript
// of an answer that can change without anyone editing the prose.
//
// That is the failure this repository has had twice: a doc comment on
// `load_icu()` asserting a linkage that did not exist, and `aligned_sum`
// documented as summing when it averaged. Both misled the next reader. Here the
// next reader is an operator deciding what to put in their config, and a stale
// row would tell them to set an overlap that does nothing — the exact defect
// the whole feature warns about.

/// The tuning guide, compiled in.
const TUNING_DOC: &str = include_str!("../docs/book/guides/tuning.md");

/// One parsed row of the confirmation-level table.
struct LevelRow<'a> {
    level: &'a str,
    percent: u32,
    span_secs: u32,
    /// `None` for "any" — the level bites at every overlap.
    minimum_overlap: Option<f32>,
    /// `(needs, of)` from an "already N of M" note, when the row carries one.
    already: Option<(usize, usize)>,
}

/// Pull the rows out of the table under the confirmation-level heading.
///
/// Returns an empty vector if the table moved or was reshaped, which the caller
/// treats as a failure rather than a pass — a matcher that quietly finds
/// nothing is how a gate like this becomes decorative.
fn documented_levels() -> Vec<LevelRow<'static>> {
    let Some(section) = TUNING_DOC.split("BIRDNET_CONFIRMATION_LEVEL=").nth(1) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 {
            continue;
        }
        let level = cells[0].trim_matches('`');
        if !matches!(level, "lenient" | "moderate" | "balanced" | "strict") {
            continue;
        }
        let Ok(percent) = cells[1].trim_end_matches('%').parse::<u32>() else {
            continue;
        };
        let Ok(span_secs) = cells[2].trim_end_matches(" s").parse::<u32>() else {
            continue;
        };
        let last = cells[3];
        let minimum_overlap = if last.starts_with("any") {
            None
        } else {
            match last.trim_matches('`').parse::<f32>() {
                Ok(v) => Some(v),
                Err(_) => continue,
            }
        };
        let already = last.split("already ").nth(1).and_then(|rest| {
            let mut parts = rest.trim_end_matches(')').split(" of ");
            Some((
                parts.next()?.trim().parse().ok()?,
                parts.next()?.trim().parse().ok()?,
            ))
        });
        out.push(LevelRow {
            level,
            percent,
            span_secs,
            minimum_overlap,
            already,
        });
    }
    out
}

#[test]
fn the_tuning_guides_confirmation_table_matches_the_filter() {
    use birdnet_core::detection::corroboration::{
        ConfirmationLevel, REFERENCE_SPAN, required_confirmations,
    };

    let rows = documented_levels();
    assert_eq!(
        rows.len(),
        4,
        "docs/book/guides/tuning.md must still carry a four-row table of \
         confirmation levels under the BIRDNET_CONFIRMATION_LEVEL example — if \
         the table moved or was reshaped, retarget this gate rather than \
         deleting it. Parsed {} row(s).",
        rows.len()
    );

    // The window length the table's numbers are written for. Not operator
    // tunable at the config layer, so a literal here is honest; it is the same
    // value `check_confirmation_filter` reports against.
    let chunk_secs =
        birdnet_core::detection::pipeline::PipelineConfig::default().chunk_duration_secs;

    for row in rows {
        let level = ConfirmationLevel::parse(row.level).expect("filtered to the four names above");

        #[allow(clippy::cast_precision_loss)]
        let documented_fraction = row.percent as f32 / 100.0;
        assert!(
            (documented_fraction - level.required_fraction()).abs() < 1e-6,
            "the guide says `{}` demands {}%, the filter demands {}%",
            row.level,
            row.percent,
            level.required_fraction() * 100.0
        );

        #[allow(clippy::cast_precision_loss)]
        let documented_span = row.span_secs as f32;
        assert!(
            (documented_span - REFERENCE_SPAN).abs() < 1e-6,
            "the guide says `{}` looks across {} s, the filter uses {REFERENCE_SPAN} s",
            row.level,
            row.span_secs
        );

        let actual = level
            .minimum_overlap(chunk_secs)
            .expect("an enabled level has one");
        match row.minimum_overlap {
            Some(documented) => assert!(
                (documented - actual).abs() < 1e-6,
                "the guide tells operators `{}` needs at least {documented}s of \
                 overlap; the filter starts demanding a second opinion at {actual}s",
                row.level
            ),
            None => assert!(
                actual == 0.0,
                "the guide says `{}` bites at any overlap; the filter needs {actual}s",
                row.level
            ),
        }

        if let Some((needs, of)) = row.already {
            assert_eq!(
                level.required_confirmations_at(0.0, chunk_secs),
                needs,
                "the guide says `{}` is already {needs} of {of} at zero overlap",
                row.level
            );
            // And the other half of "N of M": M is the number of windows the
            // filter actually sees at zero overlap, so feeding M back through
            // the same arithmetic must reproduce N. A row claiming "2 of 5"
            // would pass the check above and fail this one.
            assert_eq!(
                required_confirmations(level, of),
                needs,
                "the guide says `{}` is {needs} of {of} at zero overlap, but {of} \
                 windows at that level is {}",
                row.level,
                required_confirmations(level, of)
            );
        }
    }
}

// ── The offsite-backup manual against the planner ────────────────────────────
//
// `docs/book/admin/backups.md` is where an operator learns what to put in their
// config, and every key it names is one `helpers::offsite::plan` has to read.
// The two drift in the obvious direction: a key gets renamed in code and the
// manual keeps telling people the old name, which reads as "offsite backups do
// not work" rather than as "the manual is stale".
//
// Before this feature existed the same page said, in bold, that the station has
// no built-in upload to S3 or a NAS. That sentence was true when it was written
// and wrong the day the code landed, and nothing would have caught it.

/// The backups page, compiled in.
const BACKUPS_DOC: &str = include_str!("../docs/book/admin/backups.md");

/// Every `OFFSITE_*` key the manual mentions.
fn documented_offsite_keys() -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (idx, _) in BACKUPS_DOC.match_indices("OFFSITE_") {
        let rest = &BACKUPS_DOC[idx..];
        let end = rest
            .char_indices()
            .position(|(_, c)| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        out.insert(rest[..end].trim_end_matches('_').to_string());
    }
    out
}

/// Every `OFFSITE_*` key the planner and the decrypt path actually read.
///
/// Two things the first version of this got wrong, both of which made it report
/// dozens of "undocumented keys" that were not keys at all:
///
/// * it took everything up to the next quote, so `"OFFSITE_BACKUP=s3"` from a
///   test fixture and `"OFFSITE_SFTP_PORT is `{v}`, which is not a port"` from
///   an error message both counted. A key literal ends *at* the closing quote,
///   so that is what is required here.
/// * it scanned the whole file, test module included. Only the production half
///   describes what the binary reads.
fn implemented_offsite_keys() -> std::collections::BTreeSet<String> {
    let src = include_str!("../src/helpers/offsite.rs");
    let production = src.split_once("#[cfg(test)]").map_or(src, |(head, _)| head);
    let mut out = std::collections::BTreeSet::new();
    for (idx, _) in production.match_indices("\"OFFSITE_") {
        let rest = &production[idx + 1..];
        let end = rest
            .char_indices()
            .position(|(_, c)| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(rest.len());
        // A literal is `"OFFSITE_X"`; anything else is prose that happens to
        // begin with the prefix.
        if rest[end..].starts_with('"') {
            out.insert(rest[..end].to_string());
        }
    }
    out
}

#[test]
fn the_backups_page_documents_every_offsite_key_and_no_others() {
    let documented = documented_offsite_keys();
    let implemented = implemented_offsite_keys();

    assert!(
        implemented.len() >= 10,
        "the key scanner found only {} keys in src/helpers/offsite.rs — it has \
         stopped matching, and this whole gate is now vacuous: {implemented:?}",
        implemented.len()
    );

    let undocumented: Vec<&String> = implemented.difference(&documented).collect();
    assert!(
        undocumented.is_empty(),
        "these offsite keys exist but the backups page never names them, so an \
         operator has no way to discover them: {undocumented:?}"
    );

    let stale: Vec<&String> = documented.difference(&implemented).collect();
    assert!(
        stale.is_empty(),
        "the backups page tells operators to set keys nothing reads: {stale:?}"
    );
}

// ── Claims the manual makes that shipping something made false ──────────────
//
// Twice in one release cycle a page said, in plain words, that a feature did
// not exist — and was right when it was written:
//
//   * `admin/backups.md`: "the station has no built-in upload to S3, a NAS, or
//     email", until `OFFSITE_BACKUP` landed.
//   * `guides/recipes.md`: "the built-in server is plain HTTP", until
//     `--tls-mode` landed.
//
// Neither was caught by anything. A test that names the *sentence* is the only
// kind that can catch this class, because there is nothing structural about a
// paragraph that says a feature is absent — it reads exactly like a paragraph
// that says one is present.
//
// So: a table of retired claims, checked across the whole manual and the
// README, each paired with the flag or key that retired it. The pairing is not
// decoration — [`retired_claims_name_something_that_actually_ships`] checks
// that the retiring feature still exists, so an entry cannot outlive the thing
// that made it stale and quietly forbid a sentence that has become true again.

/// A sentence the manual must no longer contain, and what made it false.
struct RetiredClaim {
    /// Lowercase phrase to search for.
    phrase: &'static str,
    /// The `--flag` or `CONFIG_KEY` whose arrival retired it.
    retired_by: &'static str,
    /// Where it was found, for the failure message.
    was_in: &'static str,
}

const RETIRED_CLAIMS: &[RetiredClaim] = &[
    RetiredClaim {
        phrase: "no built-in upload",
        retired_by: "OFFSITE_BACKUP",
        was_in: "admin/backups.md",
    },
    RetiredClaim {
        phrase: "snapshots are not off-site",
        retired_by: "OFFSITE_BACKUP",
        was_in: "admin/backups.md",
    },
    RetiredClaim {
        phrase: "the built-in server is plain http",
        retired_by: "--tls-mode",
        was_in: "guides/recipes.md",
    },
];

/// Every markdown file in the manual, plus the README, as (path, text).
fn manual_pages() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Ok(readme) = std::fs::read_to_string(root.join("README.md")) {
        out.push((PathBuf::from("README.md"), readme));
    }
    collect_markdown(&root.join("docs/book"), &root, &mut out);
    out
}

fn collect_markdown(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // `_generated/` is machine-written from the binary itself, so it
            // cannot contain a stale human claim.
            if path.file_name().is_some_and(|n| n == "_generated") {
                continue;
            }
            collect_markdown(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push((rel, text));
        }
    }
}

#[test]
fn the_manual_does_not_still_say_a_shipped_feature_is_missing() {
    let pages = manual_pages();
    assert!(
        pages.len() > 20,
        "the page walker found only {} markdown files — it has stopped working \
         and this gate is vacuous",
        pages.len()
    );

    let mut found = Vec::new();
    for (path, text) in &pages {
        let lowered = text.to_lowercase();
        for claim in RETIRED_CLAIMS {
            if lowered.contains(claim.phrase) {
                found.push(format!(
                    "{}: {:?} — retired by `{}` (was in {})",
                    path.display(),
                    claim.phrase,
                    claim.retired_by,
                    claim.was_in
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "the manual tells readers a shipped feature does not exist:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn retired_claims_name_something_that_actually_ships() {
    // The counterpart, and what keeps the table above from becoming a list of
    // arbitrary forbidden words. Each entry says a feature retired the claim;
    // if that feature is gone, the sentence may be true again and the entry
    // should go with it.
    let cli = include_str!("../src/cli.rs");
    let offsite = include_str!("../src/helpers/offsite.rs");
    let haystack = format!("{cli}{offsite}");

    assert!(
        !RETIRED_CLAIMS.is_empty(),
        "an empty table would pass every check above for free"
    );
    for claim in RETIRED_CLAIMS {
        assert!(
            haystack.contains(claim.retired_by),
            "`{}` retires {:?}, but nothing in the CLI or the offsite planner \
             mentions it any more. If the feature was removed, drop the entry — \
             the manual may be allowed to say that again",
            claim.retired_by,
            claim.phrase
        );
    }
}

#[test]
fn the_backups_page_documents_the_offsite_feature_at_all() {
    // The other half of the staleness check: forbidding the old sentence is
    // satisfied by deleting the whole section, so require the new one.
    assert!(
        BACKUPS_DOC.contains("OFFSITE_BACKUP"),
        "the backups page must document the offsite feature"
    );
    assert!(
        BACKUPS_DOC.contains("--decrypt-backup"),
        "a backup format with no documented restore path is not documented"
    );
}

// ── The README's test count ─────────────────────────────────────────────────
//
// It said "1,690+" while the workspace carried nearly twice that. Nobody lied;
// the number was written once and the suite kept growing, which is what every
// hand-maintained number in a README does.
//
// Counted statically, from the `#[test]` attributes themselves, rather than by
// running the suite: a test that runs the suite to count it cannot run inside
// the suite. That undercounts — `proptest!` blocks and `#[test_case]`-style
// expansions are one attribute for many cases — which is the right direction
// for a floor.

/// The README, compiled in.
const README: &str = include_str!("../README.md");

/// `#[test]` and `#[tokio::test]` attributes across the workspace.
fn declared_tests() -> usize {
    let mut count = 0;
    for (_, text) in rust_sources() {
        for line in text.lines() {
            let line = line.trim();
            if line == "#[test]" || line == "#[tokio::test]" {
                count += 1;
            }
        }
    }
    count
}

/// Every `.rs` file under `src/`, `crates/` and `tests/`.
fn rust_sources() -> Vec<(PathBuf, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for top in ["src", "crates", "tests", "benches"] {
        collect_rust(&root.join(top), &root, &mut out);
    }
    out
}

fn collect_rust(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rust(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push((path.strip_prefix(root).unwrap_or(&path).to_path_buf(), text));
        }
    }
}

/// The `N,NNN+` figure from the README's Tests bullet.
fn claimed_test_count() -> Option<usize> {
    let bullet = README
        .split("**Tests**")
        .nth(1)?
        .trim_start_matches([' ', '—', '-']);
    let digits: String = bullet
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[test]
fn the_readmes_test_count_is_not_an_overstatement() {
    let claimed = claimed_test_count().expect(
        "the README's Tests bullet must still open with a number — if it was \
         reworded, retarget this gate rather than deleting it",
    );
    let declared = declared_tests();
    assert!(
        declared >= 500,
        "the attribute scanner found only {declared} tests, which cannot be \
         right — it has stopped matching and this gate is vacuous"
    );
    assert!(
        claimed <= declared,
        "the README claims {claimed}+ tests; only {declared} `#[test]` \
         attributes exist. The claim has drifted above reality"
    );
    // And the other direction: a claim far below the truth is not false, but it
    // is the state this gate was written to fix, so it has to be caught.
    //
    // The tolerance is two-thirds, chosen against the case that prompted this:
    // "1,690+" beside 3,103 real tests. A factor of two — the first thing
    // written here — let that exact number through, which would have made the
    // assertion a decoration. Two-thirds catches it and still leaves room for
    // the suite to grow by half before anyone is nagged.
    assert!(
        claimed * 3 >= declared * 2,
        "the README claims {claimed}+ tests but there are {declared}. That is \
         not false, but it is what \"1,690+\" looked like while the suite grew \
         past three thousand — round the claim up to something near {declared}"
    );
}

// ── the dependency ADR names crates that still exist ───────────────────────

/// The dependency ADR, compiled in.
const DEPS_DOC: &str = include_str!("../docs/architecture/04-dependencies.md");

/// The lockfile, compiled in — the only honest answer to "does this build
/// actually have that crate".
const LOCKFILE: &str = include_str!("../Cargo.lock");

/// The header cell that marks a column as naming crates this project uses.
const CRATE_HEADER: &str = "Crate";

/// The header cell that marks the one table whose crates are deliberately
/// absent. Checking *those* against the lockfile would assert the opposite of
/// what the table says.
const EXCLUSION_HEADER: &str = "Reason for exclusion";

/// Names in a `Crate` column that are not crates.
///
/// Kept to things that read like a crate name and are not one; each costs a
/// line and a reason, which is the point. A dropped dependency must never be
/// silenced by adding it here — that is the failure this gate exists to catch.
const NOT_OUR_CRATES: &[(&str, &str)] = &[(
    "aead",
    "a `ring` feature, listed in parentheses beside the crate that provides it",
)];

/// Is this row the `|---|---|` rule under a header?
fn is_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Split a Markdown table row into trimmed cells.
fn cells_of(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// Could this token name a crate at all?
///
/// Shape, not allowlist: a crate name is lowercase letters, digits, `-` and
/// `_`, so a version string or a capitalised acronym that wanders into a
/// `Crate` cell is dropped without anybody having to enumerate it.
fn could_name_a_crate(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Every crate named in a `Crate` column of the ADR, with the row it sits in.
///
/// Driven by each table's own header rather than by a fixed column index: the
/// ADR has five tables and `Crate` is the second column in two of them and the
/// first in the rest, so a fixed index reads prose out of the wrong cell and
/// needs an allowlist to make up for it. The rejected-alternatives table is
/// skipped by its header, because a crate listed there is *supposed* to be
/// missing from the lockfile.
///
/// Returns an empty vector if the tables moved or were reshaped, which the
/// caller treats as a failure rather than a pass: a matcher that quietly finds
/// nothing is how a gate like this becomes decorative.
fn documented_crates() -> Vec<(&'static str, &'static str)> {
    let lines: Vec<&'static str> = DEPS_DOC.lines().collect();
    let mut found = Vec::new();
    let mut crate_col: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if !line.trim_start().starts_with('|') {
            crate_col = None; // a table ends where the pipes stop
            continue;
        }
        if is_separator(line) {
            continue;
        }
        let cells = cells_of(line);

        // A header is a pipe row sitting directly above the separator rule.
        if lines.get(i + 1).is_some_and(|next| is_separator(next)) {
            crate_col = if cells.contains(&EXCLUSION_HEADER) {
                None
            } else {
                cells.iter().position(|c| *c == CRATE_HEADER)
            };
            continue;
        }

        let (Some(col), Some(row)) = (crate_col, cells.first()) else {
            continue;
        };
        let Some(cell) = cells.get(col) else { continue };
        for token in cell.split('`').skip(1).step_by(2) {
            if could_name_a_crate(token) {
                found.push((*row, token));
            }
        }
    }
    found
}

/// The ADR must not advertise a crate this build does not have.
///
/// Nothing structural connects that table to `Cargo.lock`, so a dropped
/// dependency leaves its row behind reading as current. It has happened twice:
/// `rustls-pemfile` went on being named as the thing that parses PEM after it
/// was replaced by `rustls_pki_types::pem`, and `configparser` was named as
/// the config parser while `birdnet-core::config` had always hand-rolled it —
/// the crate was declared in `[workspace.dependencies]`, used by no member,
/// and had therefore never even reached the lockfile.
///
/// The manual-staleness gate above cannot see either. It matches sentences
/// claiming a feature is *missing*; a table naming a crate that is *gone* has
/// no sentence to match.
#[test]
fn the_dependency_adr_names_no_crate_this_build_does_not_have() {
    let documented = documented_crates();
    assert!(
        !documented.is_empty(),
        "no crate names were read out of docs/architecture/04-dependencies.md \
         — the tables moved or were reshaped; retarget this gate rather than \
         deleting it"
    );

    let missing: Vec<String> = documented
        .iter()
        .filter(|(_, krate)| !NOT_OUR_CRATES.iter().any(|(name, _)| name == krate))
        .filter(|(_, krate)| !LOCKFILE.contains(&format!("name = \"{krate}\"")))
        .map(|(row, krate)| format!("`{krate}` (row: {row})"))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/architecture/04-dependencies.md names {} crate(s) that are not in \
         Cargo.lock:\n  {}\nEither the dependency was dropped and the row is now \
         false, or the name is not a crate — in which case add it to \
         NOT_OUR_CRATES with the reason.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// The counterpart, kept green: the gate above is worth nothing if its matcher
/// reads no rows, and worth little if it reads only some of the tables.
///
/// Names one crate from each table shape — `Crate` second, `Crate` first — so
/// a matcher that silently stopped reading one of them fails here. Asserts
/// membership rather than the full list, so ordinary churn need not touch it.
#[test]
fn the_dependency_adr_matcher_actually_reads_every_table() {
    let found = documented_crates();
    assert!(
        found.len() >= 30,
        "only {} crate names read out of the ADR, which is fewer than one \
         table's worth — the matcher is losing rows: {found:?}",
        found.len()
    );
    for expected in ["axum", "tokio", "rusqlite", "ort", "birdnet-core"] {
        assert!(
            found.iter().any(|(_, krate)| *krate == expected),
            "`{expected}` is named in a Crate column of the ADR but the matcher \
             did not find it: {found:?}"
        );
    }
}

/// The rejected-alternatives table must stay out of scope.
///
/// Its crates are the ones this project deliberately does *not* depend on, so
/// they are absent from the lockfile by design. If the skip ever stopped
/// working the gate above would demand the project take on `russh`.
#[test]
fn the_rejected_alternatives_table_is_not_checked_against_the_lockfile() {
    assert!(
        DEPS_DOC.contains(EXCLUSION_HEADER),
        "the rejected-alternatives table is gone or renamed; this gate skips it \
         by its `{EXCLUSION_HEADER}` header, so retarget the skip"
    );
    let found = documented_crates();
    for rejected in ["russh", "ssh2", "aws-sdk-s3"] {
        assert!(
            !found.iter().any(|(_, krate)| *krate == rejected),
            "`{rejected}` is a deliberately rejected crate and must not be \
             checked against the lockfile"
        );
    }
}

/// And backwards, so an allowlist entry cannot outlive the row it excuses.
///
/// `NOT_OUR_CRATES` suppresses a name; once the row naming it is gone the entry
/// is dead weight that would silently excuse the same name if it came back
/// meaning something else.
#[test]
fn no_allowlisted_non_crate_has_outlived_its_row() {
    let documented = documented_crates();
    for (name, why) in NOT_OUR_CRATES {
        assert!(
            documented.iter().any(|(_, krate)| krate == name),
            "NOT_OUR_CRATES still excuses `{name}` ({why}), but no Crate column \
             in docs/architecture/04-dependencies.md names it any more — drop \
             the entry"
        );
    }
}
