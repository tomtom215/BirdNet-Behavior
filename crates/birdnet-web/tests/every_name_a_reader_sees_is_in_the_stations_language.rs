//! A station set to German shows German bird names wherever a person reads one,
//! and keeps the classifier's names wherever a program does.
//!
//! # What this guards
//!
//! `DATABASE_LANG` used to do nothing: the setting was stored, the page accepted
//! it, and the pages drew the stored (English) name regardless. It now works by
//! translating when a name is *drawn* (`birdnet_core::i18n`), which is only true
//! of a page if every place that draws a name goes through the localizer. There
//! are about forty such places, written over many releases, and one that is
//! missed is a page that is German except for one row — which reads as a bug in
//! the translation, not in the page.
//!
//! Two checks, because each fails where the other cannot:
//!
//! 1. **Behaviour** — the pages that draw species names are fetched from a
//!    station set to German, over a database holding English-named detections.
//!    What a reader sees (text, and the attributes that are read — `title`,
//!    `alt`, `aria-label`, the player's label, a confirmation prompt) must hold
//!    the German name and none of the English ones. What a program uses (links,
//!    form values, `hx-vals`, the JSON API) is not in that set, and is checked
//!    to *still* carry the stored name: a link that said `name=Elster` would
//!    open a page that does not know the bird.
//! 2. **Source** — every `escape_html`/`escape_xml` of a stored name in the web
//!    crate is on a named list, with the reason it is not a place a name is
//!    drawn from the database untranslated. A new one fails until it is
//!    classified.
//!
//! Observed failing against the pre-feature pages (every one drew the stored
//! name), and against each mutant listed in the commit that added it.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::i18n::{LanguagePack, NameLocalizer};
use birdnet_core::inference::labels::LabelSet;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// `(scientific name, the name the classifier stores, the German name)`.
const SPECIES: [(&str, &str, &str); 3] = [
    ("Pica pica", "Eurasian Magpie", "Elster"),
    ("Turdus merula", "Common Blackbird", "Amsel"),
    ("Parus major", "Great Tit", "Kohlmeise"),
];

/// The classifier's label file for the seeded birds, as the station loads it.
/// Pages that hold only a stored name translate it through this, so a station
/// without one would not — which `init_i18n` and `--doctor` say.
fn labels() -> LabelSet {
    let mut text = String::new();
    for (sci, com, _) in SPECIES {
        text.push_str(sci);
        text.push('_');
        text.push_str(com);
        text.push('\n');
    }
    LabelSet::load_from_str(&text).expect("labels")
}

fn state(german: bool) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    let state = if german {
        state.with_names(NameLocalizer::new(
            LanguagePack::bundled("de").expect("German is bundled"),
            Some(&labels()),
        ))
    } else {
        state
    };
    (dir, state)
}

/// Dates the seed uses, as `YYYY-MM-DD` in the station's local calendar.
struct Days {
    today: String,
    monday: String,
}

fn seed(state: &AppState) -> Days {
    state.with_db(|conn| {
        let day = |modifier: &str| -> String {
            conn.query_row(
                &format!("SELECT date('now','localtime'{modifier})"),
                [],
                |r| r.get(0),
            )
            .expect("date")
        };
        let mut n = 0;
        for back in 0..4 {
            let date = day(&format!(",'-{back} days'"));
            for (i, (sci, com, _)) in SPECIES.iter().enumerate() {
                for hour in [6, 7] {
                    n += 1;
                    conn.execute(
                        "INSERT INTO detections
                             (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens,
                              Overlap, File_Name, chunk_offset_secs)
                         VALUES (?1, ?2, ?3, ?4, ?5, 0.7, 18, 1.25, 0.0, ?6, 0)",
                        rusqlite::params![
                            date,
                            format!("{hour:02}:{:02}:00", 10 + i),
                            sci,
                            com,
                            0.80 + 0.04 * f64::from(u8::try_from(i).unwrap()),
                            format!("clip-{n}.wav"),
                        ],
                    )
                    .expect("insert detection");
                }
            }
        }
        conn.execute(
            "INSERT INTO quarantine (date, time, sci_name, com_name, confidence, reason)
             VALUES (?1, '05:30:00', 'Pica pica', 'Eurasian Magpie', 0.55, 'low_confidence')",
            [day("")],
        )
        .expect("insert quarantine row");
        birdnet_db::settings::ensure_settings_table(conn).ok();
        birdnet_db::notifications::log_notification(
            conn,
            &birdnet_db::notifications::NotifRecord {
                channel: "apprise",
                species_com_name: Some("Eurasian Magpie"),
                species_sci_name: Some("Pica pica"),
                confidence: Some(0.9),
                detection_date: Some(&day("")),
                detection_time: Some("06:10:00"),
                status: birdnet_db::notifications::NotifStatus::Sent,
                message: None,
                error: None,
            },
        )
        .expect("log notification");
        Days {
            today: day(""),
            monday: day(",'weekday 0','-6 days'"),
        }
    })
}

async fn get(state: &AppState, uri: &str, htmx: bool) -> (StatusCode, String) {
    let mut req = Request::builder().uri(uri);
    if htmx {
        req = req.header("hx-request", "true");
    }
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 23)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The attributes whose value a reader reads (or a screen reader announces).
const READ_ATTRS: &[&str] = &[
    "title",
    "alt",
    "aria-label",
    "placeholder",
    "data-clip-name",
    "data-confirm-body",
    "hx-confirm",
];

/// What a person reads in `html`: its text, and the value of [`READ_ATTRS`].
/// Links, form values and `hx-vals` are keys, not prose, and are not here.
fn read_by_a_person(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        out.push('\n');
        let tag_and_after = &rest[open..];
        let Some(close) = tag_and_after.find('>') else {
            break;
        };
        let tag = &tag_and_after[..=close];
        for attr in READ_ATTRS {
            let needle = format!(" {attr}=\"");
            let mut from = 0;
            while let Some(at) = tag[from..].find(&needle) {
                let value_start = from + at + needle.len();
                let Some(len) = tag[value_start..].find('"') else {
                    break;
                };
                out.push_str(&tag[value_start..value_start + len]);
                out.push('\n');
                from = value_start + len;
            }
        }
        rest = &tag_and_after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The surfaces that draw a species name from the database, each with what it
/// needs. Every route is one `birdnet-web` serves; a path that stopped existing
/// answers 404 and fails the status check, so the list cannot rot quietly.
fn surfaces(days: &Days) -> Vec<(String, bool)> {
    let d = &days.today;
    let m = &days.monday;
    let stored = "Eurasian%20Magpie";
    [
        // The home and the dashboard's partials.
        ("/pages/today-list".to_owned(), true),
        ("/pages/detections".to_owned(), true),
        ("/pages/best-detections".to_owned(), true),
        ("/pages/top-species".to_owned(), true),
        ("/pages/species-list".to_owned(), true),
        ("/pages/most-recent".to_owned(), true),
        ("/pages/activity-heatmap".to_owned(), true),
        ("/pages/kiosk-content".to_owned(), true),
        // Species: the list, the photos, the life list, and one species' page.
        ("/species".to_owned(), false),
        ("/species?view=photos".to_owned(), false),
        ("/species?view=lifelist".to_owned(), false),
        (format!("/species/detail?name={stored}"), false),
        (format!("/pages/species-info?name={stored}"), true),
        (format!("/pages/species-companions?name={stored}"), true),
        // One detection, a day, the clips, the search.
        (
            format!("/detections/detail?date={d}&time=06:10:00&name={stored}"),
            false,
        ),
        (format!("/reports/day?date={d}"), false),
        (format!("/pages/history-chart?date={d}"), true),
        ("/pages/recordings-clips".to_owned(), true),
        ("/pages/search-results".to_owned(), true),
        // Review and quarantine.
        ("/pages/quarantine-list".to_owned(), true),
        ("/pages/detection-reviews-queue".to_owned(), true),
        // Reports and analytics that read the detections table directly.
        (format!("/pages/weekly-content?week={m}"), true),
        ("/reports?tab=year".to_owned(), false),
        ("/pages/dawn-list".to_owned(), true),
        ("/pages/activity-streamgraph".to_owned(), true),
        // The companion lookup, asked in German.
        (
            "/pages/companion-species?species=Elster&days=30".to_owned(),
            true,
        ),
        ("/pages/notif-history".to_owned(), true),
        // The command palette, searched in German.
        ("/pages/cmdk?q=kohl".to_owned(), true),
        // The feeds.
        ("/feeds/today.rss".to_owned(), false),
        ("/feeds/rare.rss".to_owned(), false),
        ("/feeds/rare.ics".to_owned(), false),
    ]
    .into()
}

#[tokio::test]
async fn a_german_station_reads_german_everywhere_a_name_is_drawn() {
    let (_dir, state) = state(true);
    let days = seed(&state);
    let mut failures = Vec::new();
    for (uri, htmx) in surfaces(&days) {
        let (status, body) = get(&state, &uri, htmx).await;
        if status != StatusCode::OK {
            failures.push(format!("{uri}: answered {status}"));
            continue;
        }
        let text = read_by_a_person(&body);
        // The pages are real, not "could not load" shells: each must draw at
        // least one of the seeded birds, in German.
        if !SPECIES.iter().any(|(_, _, de)| text.contains(de)) {
            failures.push(format!("{uri}: draws no German name at all"));
        }
        for (_, english, _) in SPECIES {
            if text.contains(english) {
                failures.push(format!("{uri}: a reader would see {english:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} surface(s) drew a stored name:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[tokio::test]
async fn the_same_pages_on_an_english_station_are_unchanged() {
    // The counterpart that makes the German result mean something: the same
    // surfaces, the same data, no language chosen, and every name is the one
    // the classifier stores. A localizer that translated unconditionally, or
    // an English station that lost its names, fails here.
    let (_dir, state) = state(false);
    let days = seed(&state);
    let mut failures = Vec::new();
    for (uri, htmx) in surfaces(&days) {
        // The two surfaces that are asked in German have no meaning in English.
        if uri.contains("species=Elster") || uri.contains("q=kohl") {
            continue;
        }
        let (status, body) = get(&state, &uri, htmx).await;
        if status != StatusCode::OK {
            failures.push(format!("{uri}: answered {status}"));
            continue;
        }
        let text = read_by_a_person(&body);
        if !SPECIES.iter().any(|(_, en, _)| text.contains(en)) {
            failures.push(format!("{uri}: draws no English name at all"));
        }
        for (_, _, german) in SPECIES {
            if text.contains(german) {
                failures.push(format!("{uri}: an English station drew {german:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn what_a_program_uses_still_carries_the_stored_name() {
    let (_dir, state) = state(true);
    let days = seed(&state);

    // A link is a key. The Today list links each row to its species and to its
    // detection by the stored name; `name=Elster` would open a page that has
    // never heard of the bird.
    let (_, today) = get(&state, "/pages/today-list", true).await;
    assert!(today.contains("name=Eurasian%20Magpie"), "{today}");
    assert!(
        !today.contains("name=Elster"),
        "a link carried the German name"
    );

    // The review queue posts the identity back as hidden fields.
    let (_, queue) = get(&state, "/pages/detection-reviews-queue", true).await;
    assert!(
        queue.contains(r#"name="com_name" value="Eurasian Magpie""#),
        "a form posted the German name back: {queue}"
    );

    // The JSON API is the stable contract: stored names, and nothing else.
    let (_, api) = get(&state, "/api/v2/detections?limit=5", false).await;
    assert!(
        api.contains("Eurasian Magpie") && !api.contains("Elster"),
        "{api}"
    );

    // The detection page is found by its stored name, and by no other.
    let found = format!(
        "/detections/detail?date={}&time=06:10:00&name=Eurasian%20Magpie",
        days.today
    );
    assert_eq!(get(&state, &found, false).await.0, StatusCode::OK);
}

// ── what is typed ───────────────────────────────────────────────────────────

/// The German names a page shows, as the set it names.
fn german_shown(text: &str) -> Vec<&'static str> {
    SPECIES
        .iter()
        .map(|(_, _, de)| *de)
        .filter(|de| text.contains(de))
        .collect()
}

#[tokio::test]
async fn a_name_typed_in_the_stations_language_finds_the_bird_in_every_search_box() {
    let (_dir, state) = state(true);
    seed(&state);
    // `(address, what a page that found only the great tit shows)`: the same
    // typed fragment, "kohl", into each of the four boxes that take free text.
    for uri in [
        "/pages/today-list?search=kohl",
        "/pages/recordings-clips?q=kohl",
        "/pages/search-results?q=kohl",
        // The exact-species field of the search page takes the whole name.
        "/pages/search-results?species=Kohlmeise",
    ] {
        let (status, body) = get(&state, uri, true).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            german_shown(&read_by_a_person(&body)),
            ["Kohlmeise"],
            "{uri}: only the great tit matches 'kohl'"
        );
    }

    // `NOT`, the legacy BirdNET-Pi exclusion, leaves it out and keeps the rest.
    for uri in [
        "/pages/today-list?search=NOT%20kohl",
        "/pages/recordings-clips?q=NOT%20kohl",
        "/pages/search-results?q=NOT%20kohl",
    ] {
        let (_, body) = get(&state, uri, true).await;
        assert_eq!(
            german_shown(&read_by_a_person(&body)),
            ["Elster", "Amsel"],
            "{uri}: 'NOT kohl' excludes the great tit and nothing else"
        );
    }

    // The stored and scientific names still work in a German station.
    for term in ["great", "Parus"] {
        let (_, body) = get(&state, &format!("/pages/today-list?search={term}"), true).await;
        assert_eq!(
            german_shown(&read_by_a_person(&body)),
            ["Kohlmeise"],
            "{term}"
        );
    }
}

#[tokio::test]
async fn an_english_station_does_not_read_what_is_typed_as_german() {
    // The counterpart: the same fragment in a station with no language finds
    // nothing, because no stored name contains it, and an English term works.
    let (_dir, state) = state(false);
    seed(&state);
    let (_, body) = get(&state, "/pages/today-list?search=kohl", true).await;
    assert!(german_shown(&read_by_a_person(&body)).is_empty());
    assert!(!read_by_a_person(&body).contains("Great Tit"), "{body}");
    let (_, body) = get(&state, "/pages/today-list?search=great", true).await;
    assert!(read_by_a_person(&body).contains("Great Tit"));
}

async fn post_form(state: &AppState, uri: &str, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("hx-request", "true")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn stored_list(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| birdnet_db::settings::get(conn, key).ok())
}

#[tokio::test]
async fn a_name_typed_into_a_species_list_is_saved_as_the_species() {
    // The filter resolves entries against the classifier's names and the
    // scientific names. "Elster" would be accepted, shown as a pill and match
    // nothing for ever, so a German station saves the scientific name — which
    // also survives a change of language.
    let (_dir, state) = state(true);
    for (kind, key) in [
        ("exclude", "species_exclude"),
        ("include", "species_include"),
    ] {
        let (status, page) =
            post_form(&state, &format!("/admin/species/{kind}/add"), "name=Elster").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            stored_list(&state, key).as_deref(),
            Some("Pica pica"),
            "{kind}"
        );
        // The list shows the bird in German beside the name it is saved under.
        assert!(
            page.contains("Elster <i class=\"sci\">Pica pica</i>"),
            "{kind}: {page}"
        );
    }
    // Counterparts: a name that is already the scientific or the classifier's is
    // saved as typed, and a name nothing resolves is saved as typed too (the
    // list has always accepted it).
    let (_, _) = post_form(
        &state,
        "/admin/species/exclude/add",
        "name=Common+Blackbird",
    )
    .await;
    let (_, _) = post_form(
        &state,
        "/admin/species/exclude/add",
        "name=Nonexistent+name",
    )
    .await;
    assert_eq!(
        stored_list(&state, "species_exclude").as_deref(),
        Some("Pica pica, Common Blackbird, Nonexistent name")
    );
    // An English station keeps what it was given.
    let (_dir2, english) = self::state(false);
    post_form(&english, "/admin/species/exclude/add", "name=Elster").await;
    assert_eq!(
        stored_list(&english, "species_exclude").as_deref(),
        Some("Elster")
    );
}

// ── the source check ────────────────────────────────────────────────────────

/// Where a stored name is escaped into output without going through the
/// localizer, and why that is right: `(file, the call, why)`.
///
/// A new site is not in this list, so it fails
/// `no_unclassified_site_escapes_a_stored_name` until somebody decides which it
/// is: a place a name is drawn (use the localizer), or one of these.
const NOT_DRAWN_FROM_THE_DATABASE_UNTRANSLATED: &[(&str, &str, &str)] = &[
    (
        "routes/pages/detection_reviews.rs",
        "escape_html(&d.com_name)",
        "the hidden `com_name` input of the review form: the identity the form posts back",
    ),
    (
        "routes/pages/detection_reviews.rs",
        "escape_html(com_name)",
        "the hidden `com_name` input of the review widget: the identity the form posts back",
    ),
    (
        "routes/admin/quality.rs",
        "escape_html(&r.shape.com_name)",
        "already localized: `load_phantoms` puts the station's name in the report before any row is drawn",
    ),
    (
        "routes/admin/species/render.rs",
        "escape_html(&s.com_name)",
        "already localized: `load_thresholds_and_suggestions` builds the suggestion with the station's name",
    ),
    (
        "routes/admin/species/manage.rs",
        "escape_html(&r.com_name)",
        "already localized: `load` puts the station's name in every row before they are drawn",
    ),
    (
        "routes/admin/migration/render.rs",
        "escape_html(&s.common_name)",
        "the import preview shows the names in the file being imported, which are that file's, in its language",
    ),
];

/// The tokens that name a stored common name in the web crate's source.
const STORED_NAME_TOKENS: &[&str] = &[
    "com_name",
    "common_name",
    "species_com_name",
    ".companion",
    ".species_a",
    ".species_b",
];

fn web_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out.sort();
    out
}

/// A source file without its test modules.
///
/// A test module is `#[cfg(..test..)]` then `mod name {`, and — `rustfmt` being a
/// CI gate — ends at the next line that is exactly `}`. Production code can sit
/// after one (`admin/species/render.rs` has a `mod tests` in the middle), so the
/// file is not simply cut at the first.
fn without_test_modules(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let is_test_mod = lines[i].starts_with("#[cfg(")
            && lines[i].contains("test")
            && lines
                .get(i + 1)
                .is_some_and(|next| next.starts_with("mod "));
        if is_test_mod {
            while i < lines.len() && lines[i] != "}" {
                i += 1;
            }
            i += 1;
            continue;
        }
        kept.push(lines[i]);
        i += 1;
    }
    kept.join("\n")
}

/// Each `escape_html(..)` / `escape_xml(..)` call in `text`, as the call
/// (whitespace collapsed) — the argument is found by matching parentheses, so a
/// call that `rustfmt` wrapped over several lines is read whole.
fn escape_calls(text: &str) -> Vec<String> {
    let mut calls = Vec::new();
    for opener in ["escape_html(", "escape_xml("] {
        let mut from = 0;
        while let Some(at) = text[from..].find(opener) {
            let start = from + at;
            let args_start = start + opener.len();
            let mut depth = 1;
            let mut end = args_start;
            for (i, c) in text[args_start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = args_start + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let argument = text[args_start..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            calls.push(format!("{opener}{argument})"));
            from = args_start;
        }
    }
    calls
}

/// Every non-test escape of something that names a stored common name and does
/// not go through the localizer, as `(path under src/, the call)`.
fn untranslated_escapes() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for path in web_sources() {
        let text = std::fs::read_to_string(&path).expect("read");
        let production = without_test_modules(&text);
        // Comments quote the very calls they explain.
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for call in escape_calls(&code) {
            let names_a_stored_name = STORED_NAME_TOKENS.iter().any(|t| call.contains(t));
            let goes_through_the_localizer = call.contains("names.") || call.contains(".names()");
            if names_a_stored_name && !goes_through_the_localizer {
                let rel = path
                    .strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                hits.push((rel, call));
            }
        }
    }
    hits
}

#[test]
fn no_unclassified_site_escapes_a_stored_name() {
    let unclassified: Vec<String> = untranslated_escapes()
        .into_iter()
        .filter(|(file, call)| {
            !NOT_DRAWN_FROM_THE_DATABASE_UNTRANSLATED
                .iter()
                .any(|(f, c, _)| file == f && call == c)
        })
        .map(|(file, call)| format!("{file}: {call}"))
        .collect();
    assert!(
        unclassified.is_empty(),
        "these calls put a stored common name on a page without the localizer. Draw it with \
         `state.names().name(sci, com)` (or `name_of_stored`), or — if it is a key, or already \
         translated — add it to NOT_DRAWN_FROM_THE_DATABASE_UNTRANSLATED with the reason:\n  {}",
        unclassified.join("\n  ")
    );
}

#[test]
fn the_classified_list_has_no_stale_entries() {
    let hits = untranslated_escapes();
    for (file, call, why) in NOT_DRAWN_FROM_THE_DATABASE_UNTRANSLATED {
        assert!(!why.is_empty());
        assert!(
            hits.iter().any(|(f, c)| f == file && c == call),
            "{file}: `{call}` is on the list but is no longer in the source — remove the entry"
        );
    }
}

#[test]
fn the_sweep_reads_the_crate_and_can_tell_a_translated_call_from_an_untranslated_one() {
    // A scan that found nothing because the path was wrong is a green gate that
    // grades nothing, and one that flagged everything would be switched off.
    assert!(
        web_sources().len() > 100,
        "found {} sources",
        web_sources().len()
    );
    let sample =
        "a(escape_html(\n    names.name(&d.sci_name,\n   &d.com_name)), escape_html(&d.com_name));";
    let calls = escape_calls(sample);
    assert_eq!(
        calls,
        [
            "escape_html(names.name(&d.sci_name, &d.com_name))",
            "escape_html(&d.com_name)"
        ]
    );
    assert!(
        !untranslated_escapes().is_empty(),
        "the list's own entries must be found"
    );
}
