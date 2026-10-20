//! A person can ask the station why a bird is not heard here, and is told the
//! station's actual decision, with its numbers (parity row P-3).
//!
//! # What was missing
//!
//! The occurrence ("range") filter holds back every species the range model
//! scores below the station's threshold for its place and week. It is the most
//! likely reason a bird an owner can hear is not in the log, and nothing could
//! say so: the filter lives on the detection thread behind an ONNX session, and
//! the web crate saw only a count of species admitted (`OccurrenceFilterState`).
//! birdnet-go has a route that lists what its range filter admits; this had
//! nothing.
//!
//! # What this holds
//!
//! The source of the numbers is a stub (`FixedOccurrence`), so everything the
//! web layer does with them is checked here without a model; the real models
//! are checked in `tests/the_range_page_explains_the_real_filter.rs`.
//!
//! 1. `GET /api/v2/species/occurrence` lists every species the model scores,
//!    most likely first, each with the station's decision and the sentence for
//!    it, for the week asked for (the current week by default), and says which
//!    threshold and which place (to one decimal) it was asked about.
//! 2. The four verdicts are told apart: allowed by the range model, allowed by
//!    the "always allowed" list (under the threshold), held back by the range
//!    model, and held back by the operator's lists (over the threshold).
//! 3. The refusals are refusals: a station with no coordinates, no range model,
//!    a model that will not run, or no detection daemon answers `503` with a
//!    `reason` and **no `species` key** — never an empty list, which would read
//!    as "nothing occurs here". A model failure does not leak the paths in its
//!    message. A bad `week`, `admitted`, `limit` or `offset` is a `400` that
//!    names the range, and the source is not asked.
//! 4. The page says the same in a sentence a person reads, escapes what they
//!    typed, finds a bird by the name in the station's language, and gives each
//!    unavailable state its own words.
//! 5. A species page links to the answer when (and only when) the station holds
//!    the species back this week; the species list links to the page.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::i18n::{LanguagePack, NameLocalizer};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::OccurrenceScore;
use birdnet_web::occurrence::{FixedOccurrence, OccurrenceReport, Unavailable};
use birdnet_web::state::AppState;
use serde_json::Value;
use tower::ServiceExt as _;

const THRESHOLD: f32 = 0.03;

fn score(sci: &str, com: &str, probability: f32, admitted: bool) -> OccurrenceScore {
    OccurrenceScore {
        scientific_name: sci.to_owned(),
        common_name: com.to_owned(),
        probability,
        admitted,
    }
}

/// The five cells of (over or under the threshold) x (admitted or not), most
/// likely first, as the real filter orders them.
fn species() -> Vec<OccurrenceScore> {
    vec![
        // Over, admitted: the ordinary case.
        score("Pica pica", "Eurasian Magpie", 0.62, true),
        // Over the threshold and still not admitted: the operator's lists.
        score("Tyto alba", "Barn Owl", 0.2, false),
        // Under, admitted anyway: the "always allowed" list.
        score("Troglodytes troglodytes", "Eurasian Wren", 0.015, true),
        // Under, not admitted: the range model holds it back.
        score("Bubo virginianus", "Great Horned Owl", 0.004, false),
        score("Spiza americana", "Dickcissel", 0.0004, false),
    ]
}

fn template() -> OccurrenceReport {
    OccurrenceReport {
        latitude: 52.52,
        longitude: 13.405,
        week: 1,
        threshold: THRESHOLD,
        species: species(),
    }
}

fn station(source: Option<Arc<FixedOccurrence>>) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    if let Some(source) = source {
        state.set_occurrence_source(source);
    }
    (dir, state)
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn api(state: &AppState, query: &str) -> (StatusCode, Value) {
    let (status, body) = get(state, &format!("/api/v2/species/occurrence{query}")).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn answering() -> (Arc<FixedOccurrence>, tempfile::TempDir, AppState) {
    let source = Arc::new(FixedOccurrence::answering(template()));
    let (dir, state) = station(Some(Arc::clone(&source)));
    (source, dir, state)
}

// ── the API ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_api_lists_every_species_with_the_stations_decision_and_numbers() {
    let (source, _dir, state) = answering();
    let (status, body) = api(&state, "?week=22").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        source.weeks_asked(),
        vec![22],
        "the week reached the source"
    );

    assert_eq!(body["week"], 22);
    assert_eq!(body["week_is_current"], false);
    // The number a person would write, not the f32 widened to f64
    // (0.029999999329447746).
    assert_eq!(body["threshold"], 0.03, "{body}");
    // One decimal: a public endpoint says which region, not which house.
    assert_eq!(body["location"]["latitude"], 52.5);
    assert_eq!(body["location"]["longitude"], 13.4);
    assert_eq!(body["scored"], 5);
    assert_eq!(body["admitted_total"], 2);
    assert_eq!(body["matched"], 5);

    let rows = body["species"].as_array().expect("species");
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["common_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Eurasian Magpie",
            "Barn Owl",
            "Eurasian Wren",
            "Great Horned Owl",
            "Dickcissel"
        ],
        "most likely first"
    );
    let verdicts: Vec<&str> = rows
        .iter()
        .map(|r| r["verdict"].as_str().unwrap())
        .collect();
    assert_eq!(
        verdicts,
        [
            "admitted",
            "held-back-by-lists",
            "admitted-by-whitelist",
            "held-back-by-range",
            "held-back-by-range"
        ],
        "the four verdicts are told apart"
    );
    assert_eq!(rows[0]["probability"], 0.62, "{}", rows[0]);
    assert_eq!(rows[3]["probability"], 0.004, "{}", rows[3]);
    assert_eq!(rows[0]["admitted"], true);
    assert_eq!(rows[3]["admitted"], false);
    assert_eq!(
        rows[3]["explanation"],
        "Held back: the range model gives it 0.4 % here in week 22; this station admits species at 3 % and above."
    );
    assert!(
        rows[0]["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Allowed:"),
        "{}",
        rows[0]["explanation"]
    );
    assert!(
        rows[1]["explanation"]
            .as_str()
            .unwrap()
            .contains("species lists"),
        "a bird over the threshold is not blamed on the range model: {}",
        rows[1]["explanation"]
    );
}

#[tokio::test]
async fn the_api_defaults_to_the_current_week() {
    let (source, _dir, state) = answering();
    let (status, body) = api(&state, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let asked = source.weeks_asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!((1..=48).contains(&asked[0]), "{asked:?}");
    assert_eq!(body["week"], asked[0]);
    assert_eq!(body["week_is_current"], true);
}

#[tokio::test]
async fn the_api_filters_by_name_and_by_decision_and_pages() {
    let (_source, _dir, state) = answering();

    let (_, body) = api(&state, "?week=22&q=great+horned").await;
    assert_eq!(body["matched"], 1);
    assert_eq!(body["species"][0]["scientific_name"], "Bubo virginianus");
    // Either name; any case.
    let (_, body) = api(&state, "?week=22&q=TYTO").await;
    assert_eq!(body["species"][0]["common_name"], "Barn Owl");

    // `scored` and `admitted_total` describe the station, not the selection.
    assert_eq!(
        (body["scored"].clone(), body["admitted_total"].clone()),
        (5.into(), 2.into())
    );

    let (_, held) = api(&state, "?week=22&admitted=false").await;
    let (_, kept) = api(&state, "?week=22&admitted=true").await;
    assert_eq!(
        (held["matched"].clone(), kept["matched"].clone()),
        (3.into(), 2.into())
    );
    assert!(
        held["species"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["admitted"] == false),
        "{held}"
    );
    assert!(
        kept["species"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["admitted"] == true),
        "{kept}"
    );

    let (_, page) = api(&state, "?week=22&limit=2&offset=1").await;
    assert_eq!(page["matched"], 5, "paging does not change what matched");
    assert_eq!(page["limit"], 2);
    assert_eq!(page["offset"], 1);
    let names: Vec<&str> = page["species"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["common_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Barn Owl", "Eurasian Wren"]);

    // A name nothing matches is a successful, empty answer to a question that
    // was asked: unlike the refusals below it has `scored`.
    let (status, none) = api(&state, "?week=22&q=zzzz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(none["matched"], 0);
    assert_eq!(none["scored"], 5);
    assert_eq!(none["species"], serde_json::json!([]));
}

#[tokio::test]
async fn the_api_finds_a_bird_by_its_name_in_the_stations_language() {
    let labels =
        LabelSet::load_from_str("Pica pica_Eurasian Magpie\nTyto alba_Barn Owl\n").expect("labels");
    let (_dir, state) = {
        let source = Arc::new(FixedOccurrence::answering(template()));
        let (dir, state) = station(Some(source));
        (
            dir,
            state.with_names(NameLocalizer::new(
                LanguagePack::bundled("de").expect("German is bundled"),
                Some(&labels),
            )),
        )
    };
    let (_, body) = api(&state, "?week=22&q=elster").await;
    assert_eq!(body["matched"], 1, "{body}");
    assert_eq!(body["species"][0]["scientific_name"], "Pica pica");
    // The stored name stays the key; the display name is the station's.
    assert_eq!(body["species"][0]["common_name"], "Eurasian Magpie");
    assert_eq!(body["species"][0]["display_name"], "Elster");
}

#[tokio::test]
async fn the_api_refuses_a_bad_request_by_naming_the_range_and_does_not_ask_the_source() {
    let (source, _dir, state) = answering();
    for (query, needle) in [
        ("?week=0", "1 to 48"),
        ("?week=49", "1 to 48"),
        ("?week=abc", "1 to 48"),
        ("?week=22.5", "1 to 48"),
        ("?week=-3", "1 to 48"),
        ("?admitted=maybe", "true"),
        ("?limit=0", "1 to 1000"),
        ("?limit=1001", "1 to 1000"),
        ("?limit=x", "1 to 1000"),
        ("?offset=-1", "from 0"),
    ] {
        let (status, body) = api(&state, query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains(needle)),
            "{query}: the refusal must name the range ({needle}): {body}"
        );
    }
    assert!(
        source.weeks_asked().is_empty(),
        "a refused request ran the model"
    );
    // The counterparts: the edges are accepted.
    for query in [
        "?week=1",
        "?week=48",
        "?limit=1",
        "?limit=1000",
        "?offset=0",
    ] {
        let (status, body) = api(&state, query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
    }
}

#[tokio::test]
async fn a_station_that_cannot_say_answers_503_with_a_reason_and_never_an_empty_list() {
    let cases: [(Option<Unavailable>, &str); 4] = [
        (Some(Unavailable::NoLocation), "no-location"),
        (Some(Unavailable::NoModel), "no-range-model"),
        (
            Some(Unavailable::Failed(
                "could not open /srv/secret/models/range.onnx: permission denied".to_owned(),
            )),
            "range-model-failed",
        ),
        (None, "no-detection-daemon"),
    ];
    for (reason, key) in cases {
        let source = reason.map(|r| Arc::new(FixedOccurrence::unavailable(r)));
        let (_dir, state) = station(source);
        let (status, text) = get(&state, "/api/v2/species/occurrence?week=22").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{key}: {text}");
        let body: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(body["reason"], key, "{text}");
        assert!(
            body["error"].as_str().is_some_and(|e| e.len() > 20),
            "{text}"
        );
        assert!(
            body.get("species").is_none() && body.get("matched").is_none(),
            "{key}: an empty list would read as 'nothing occurs here': {text}"
        );
        assert!(
            !text.contains("/srv/secret"),
            "{key}: the model's message names a path and must stay in the log: {text}"
        );
    }
}

#[tokio::test]
async fn the_occurrence_routes_are_not_carved_out_of_private_mode() {
    use std::collections::BTreeSet;
    // Private mode shuts every route but a short list; this one must not be on it
    // even with every carve-out the operator can grant.
    let all = birdnet_web::private_mode::PublicAccess::ALL
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for path in ["/api/v2/species/occurrence", "/species/occurrence"] {
        assert!(
            !birdnet_web::private_mode::is_open(path, &all),
            "{path} would be served to a visitor who has not signed in"
        );
    }
}

// ── the page ────────────────────────────────────────────────────────────

/// `<h1` elements in the page. The layout's own comment mentions `<h1>` (the
/// help drawer's title), so an element is `<h1` followed by a space.
fn h1_count(page: &str) -> usize {
    page.matches("<h1 ").count()
}

#[tokio::test]
async fn the_page_lists_what_the_station_admits_and_has_one_heading_one() {
    let (_source, _dir, state) = answering();
    let (status, page) = get(&state, "/species/occurrence?week=22").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h1_count(&page), 1, "one <h1>");
    assert!(
        page.contains(r#"name="q""#) && page.contains(r#"for="occ-q""#),
        "a labelled search"
    );
    // The admitted list names the two admitted, with its count, and not the rest.
    let at = page.find("occ-admitted-h").expect("the admitted section");
    let admitted = &page[at..];
    assert!(admitted.contains("Eurasian Magpie") && admitted.contains("Eurasian Wren"));
    assert!(
        admitted.contains("2 of the 5 species"),
        "the count of what is listed, of what the model scores"
    );
    for held in ["Great Horned Owl", "Dickcissel", "Barn Owl"] {
        assert!(
            !admitted.contains(held),
            "{held} is held back and must not be listed as admitted"
        );
    }
    // The basis line carries the real threshold and the number scored.
    assert!(
        page.contains("3 %") && page.contains("scores 5 species"),
        "{page}"
    );
    // The week in the selector is the one asked for.
    assert!(
        page.contains(r#"<option value="22" selected>"#),
        "week 22 selected"
    );
}

#[tokio::test]
async fn the_page_answers_a_name_in_a_sentence_with_the_real_numbers() {
    let (_source, _dir, state) = answering();
    let (_, page) = get(&state, "/species/occurrence?week=22&q=great+horned").await;
    assert!(
        page.contains(
            "Held back: the range model gives it 0.4 % here in week 22; this station admits species at 3 % and above."
        ),
        "{page}"
    );
    assert!(page.contains(">Held back<"), "the status word");
    // And an admitted bird is told so, with the counterpart's wording absent.
    let (_, page) = get(&state, "/species/occurrence?week=22&q=magpie").await;
    assert!(
        page.contains("Allowed: the range model gives it 62 % here"),
        "{page}"
    );
    assert!(page.contains(">Allowed<"));
    assert!(!page.contains("Held back: the range model gives it 62"));
    // A bird over the threshold that is still held back is blamed on the lists.
    let (_, page) = get(&state, "/species/occurrence?week=22&q=barn+owl").await;
    assert!(
        page.contains("Held back by your species lists, not by the range model"),
        "{page}"
    );
}

#[tokio::test]
async fn the_page_says_when_no_bird_has_that_name_and_escapes_what_was_typed() {
    let (_source, _dir, state) = answering();
    let (_, page) = get(&state, "/species/occurrence?week=22&q=zzzz").await;
    assert!(page.contains("No bird by that name"), "{page}");
    // What was typed is text, wherever it is printed. Not a script tag: the
    // response pass (`seal::redeem`) rewrites any `<script` it did not seal to
    // `&lt;script`, so a typed one reads as escaped whether or not the page
    // escaped it. A tag nothing rewrites tells the two apart.
    let (_, page) = get(&state, "/species/occurrence?week=22&q=%3Ci%3Ezzz%3C/i%3E").await;
    assert!(
        !page.contains("<i>zzz"),
        "a typed tag reached the page as markup"
    );
    // Assert on the regions that print it, not the document: the lookup box
    // escapes it too, and would satisfy a test of the page as a whole.
    let region = |from: &str, to: &str| -> String {
        let start = page.find(from).unwrap_or_else(|| panic!("no {from}"));
        let rest = &page[start..];
        rest[..rest.find(to).unwrap_or(rest.len())].to_owned()
    };
    for (what, text) in [
        (
            "the answer's heading",
            region("id=\"occ-answer-h\"", "</h2>"),
        ),
        (
            "the no-match explanation",
            region("class=\"bnb-meta es-sub\"", "</p>"),
        ),
    ] {
        assert!(
            text.contains("&lt;i&gt;zzz&lt;/i&gt;"),
            "{what} shows what was typed as text: {text}"
        );
    }
    // A bad week is explained on the page, which still renders.
    let (status, page) = get(&state, "/species/occurrence?week=99").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("from 1 to 48"), "{page}");
}

#[tokio::test]
async fn each_unavailable_state_has_its_own_words_and_lists_nothing() {
    let cases: [(Option<Unavailable>, &str); 4] = [
        (
            Some(Unavailable::NoLocation),
            "This station has no location",
        ),
        (Some(Unavailable::NoModel), "The range filter is off"),
        (
            Some(Unavailable::Failed("boom /srv/secret".to_owned())),
            "The range model could not be run",
        ),
        (None, "No range filter to ask"),
    ];
    for (reason, words) in cases {
        let source = reason.map(|r| Arc::new(FixedOccurrence::unavailable(r)));
        let (_dir, state) = station(source);
        let (status, page) = get(&state, "/species/occurrence").await;
        assert_eq!(status, StatusCode::OK, "{words}");
        assert!(page.contains(words), "{words}: {page}");
        assert!(
            !page.contains("occ-admitted-h") && !page.contains("Birds this station admits"),
            "{words}: an unavailable filter must not render a list"
        );
        assert!(
            !page.contains("/srv/secret"),
            "{words}: the detail stays in the log"
        );
        assert_eq!(h1_count(&page), 1);
    }
}

// ── the links from the species pages ────────────────────────────────────

fn heard(state: &AppState, sci: &str, com: &str) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES ('2026-06-01', '06:00:00', ?1, ?2, 0.9, 0.7, 22, 1.25, 0.0, 'x.wav', 0)",
            rusqlite::params![sci, com],
        )
        .expect("insert");
    });
}

#[tokio::test]
async fn a_species_page_links_to_the_answer_only_for_a_species_the_station_holds_back() {
    let (_source, _dir, state) = answering();
    heard(&state, "Spiza americana", "Dickcissel");
    heard(&state, "Pica pica", "Eurasian Magpie");

    let (_, held) = get(&state, "/pages/species-status?name=Dickcissel").await;
    assert!(
        held.contains(r#"href="/species/occurrence?q=Spiza%20americana""#),
        "a held-back species links to why: {held}"
    );
    let (_, kept) = get(&state, "/pages/species-status?name=Eurasian%20Magpie").await;
    assert!(
        !kept.contains("/species/occurrence"),
        "an admitted species has nothing to explain: {kept}"
    );
    // Not scored by the model at all: no claim.
    heard(&state, "Passer domesticus", "House Sparrow");
    let (_, unknown) = get(&state, "/pages/species-status?name=House%20Sparrow").await;
    assert!(!unknown.contains("/species/occurrence"), "{unknown}");

    // And a station with no source says nothing, rather than failing.
    let (_dir2, bare) = station(None);
    heard(&bare, "Spiza americana", "Dickcissel");
    let (status, none) = get(&bare, "/pages/species-status?name=Dickcissel").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!none.contains("/species/occurrence"));
}

#[tokio::test]
async fn the_species_list_links_to_the_page() {
    let (_dir, state) = station(None);
    let (_, list) = get(&state, "/species").await;
    assert!(
        list.contains(r#"href="/species/occurrence""#),
        "the Species home must offer the range lookup: it is the only way in"
    );
}
