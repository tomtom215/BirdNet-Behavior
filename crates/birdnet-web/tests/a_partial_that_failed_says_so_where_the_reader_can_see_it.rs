//! A failed HTMX partial must answer 200, because htmx throws a 5xx body away.
//!
//! # The defect
//!
//! `static/htmx.min.js` ships
//! `responseHandling:[…,{code:"[45]..",swap:false,error:true}]`. A 5xx body is
//! **never swapped into the page**. Twenty-four partials answered
//! `500` with a hand-written message — `<p>Error loading species</p>` and
//! friends — and every one of those messages was unreachable: driving
//! `/pages/top-species` to 500 in a browser and reading the DOM back shows the
//! server's body absent from it.
//!
//! What the reader got instead was `layout.html`'s `htmx:responseError`
//! fallback: *"This section could not load (HTTP 500). Reload the page"* — an
//! HTTP status code shown to a birdwatcher, and advice that repeats the
//! failure, since reloading a station whose database will not read fails
//! again. Meanwhile nothing logged the error: the arms were `_ =>`, which
//! discards it, and `with_read_db` is a pass-through that logs nothing itself.
//! So the one copy of the real reason was thrown away too.
//!
//! # What this gate holds
//!
//! Against a station whose `detections` table has been dropped — what a
//! partially-restored or corrupt database looks like from a handler — every
//! detections-backed partial must answer **200** carrying the shared
//! `error_states` fragment.
//!
//! # Why the counterpart is not optional
//!
//! "Always render the error" would satisfy the first test completely. The
//! second drives the same routes against a station that is *fine* and requires
//! real content and no error marker, so a handler that returned the failure
//! unconditionally fails here.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Partials whose content comes from `detections`, so dropping that table is
/// enough to drive every one of them down its failure path.
///
/// `/pages/today-count` is deliberately absent: its target is a `<span>`
/// inside a `<button>` label, so it cannot carry the shared fragment at all.
/// `a_count_that_failed_stays_inside_its_sentence` covers it instead.
const PARTIALS: &[&str] = &[
    "/pages/detections",
    "/pages/best-detections",
    "/pages/top-species",
    "/pages/species-list",
    "/pages/hourly-chart",
    "/pages/daily-chart",
    "/pages/confidence-chart",
    "/pages/today-list",
    "/pages/today-daystrip",
    "/pages/life-accumulation",
    "/pages/species-summary?name=Eurasian%20Blackbird",
    "/pages/species-hourly?name=Eurasian%20Blackbird",
    "/pages/species-daily?name=Eurasian%20Blackbird",
    "/pages/species-detections?name=Eurasian%20Blackbird",
    "/pages/species-companions?name=Eurasian%20Blackbird",
];

/// The prose only the shared error states say. Matched rather than the
/// `bnb-load-error` class, which `layout.html`'s fallback script and
/// `onboarding.rs`'s `<noscript>` note also carry — the same trap
/// `read_paths_against_real_readers` documents.
const MARKER: &str = "We couldn't load";

fn station(break_reads: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    for i in 0..6 {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens,
                  Overlap, File_Name, chunk_offset_secs)
             VALUES (date('now','localtime'), ?1, 'Turdus merula',
                     'Eurasian Blackbird', 0.88, 0.7, 38, 1.25, 0.0, ?2, 0)",
            rusqlite::params![format!("0{}:15:00", 4 + i), format!("clip{i}.wav")],
        )
        .expect("seed");
    }
    if break_reads {
        // Both, and the second is not obvious. `species_summary` is the
        // per-species aggregate migration 30 maintains on write, not a view
        // over `detections` — so dropping `detections` alone leaves
        // `/pages/species-list` reading a live, populated table and rendering
        // a perfectly good species list. The first draft of this fixture did
        // exactly that and the gate reported the handler as broken when it
        // was fine.
        for table in ["detections", "species_summary"] {
            conn.execute(&format!("DROP TABLE {table}"), [])
                .unwrap_or_else(|e| panic!("drop {table}: {e}"));
        }
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let res = birdnet_web::server::build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn a_failed_partial_answers_200_with_the_shared_error_state() {
    let state = station(true);
    for uri in PARTIALS {
        let (status, body) = get(&state, uri).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{uri} answered {status}. htmx ships \
             `{{code:\"[45]..\",swap:false}}`, so this body is discarded and \
             the reader is left with layout.html's generic fallback naming an \
             HTTP status code."
        );
        assert!(
            body.contains(MARKER),
            "{uri} answered 200 without saying anything failed: {}",
            body.chars().take(240).collect::<String>()
        );
    }
}

/// The counterpart: the same routes on a station with nothing wrong.
#[tokio::test]
async fn a_healthy_partial_renders_its_content_and_no_error() {
    let state = station(false);
    for uri in PARTIALS {
        let (status, body) = get(&state, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} did not answer");
        assert!(
            !body.contains(MARKER),
            "{uri} reported a failure on a healthy station: {}",
            body.chars().take(240).collect::<String>()
        );
        assert!(
            !body.trim().is_empty(),
            "{uri} rendered nothing at all, so the test above proves nothing"
        );
    }
}

/// `/pages/today-count` fills `#td-total`, a `<span>` carrying a number inside
/// the "Show the full day (34) — search, filter, lock & delete" button.
///
/// Flow content is not legal there and an `<a href>` is worse: nested
/// interactive content, which no keyboard or screen-reader user can resolve.
/// Both the old 500 body and `layout.html`'s fallback put exactly that inside
/// the button. So this one answers 200 with phrasing content and no link, and
/// says nothing it cannot support — `?` is not a count, where an em dash would
/// read as "none".
#[tokio::test]
async fn a_count_that_failed_stays_inside_its_sentence() {
    let broken = station(true);
    let (status, body) = get(&broken, "/pages/today-count?bare=1").await;
    assert_eq!(status, StatusCode::OK, "a discarded body helps nobody");
    assert!(
        body.contains('?'),
        "the slot must say the count is unknown: {body:?}"
    );
    for illegal in ["<a ", "<div", "<p "] {
        assert!(
            !body.contains(illegal),
            "{illegal:?} is not legal inside a <button> label: {body:?}"
        );
    }
    assert!(
        body.contains("sr-only"),
        "`?` alone is not an explanation for a screen reader: {body:?}"
    );

    // And the counterpart: a healthy station still gets its number.
    let (_, ok_body) = get(&station(false), "/pages/today-count?bare=1").await;
    assert_eq!(
        ok_body.trim(),
        "6",
        "the real count must survive: {ok_body:?}"
    );
}

/// No handler under `routes/pages/` may pair a 5xx with an HTML body again.
///
/// The behavioural tests above cover the partials reachable from `detections`;
/// this covers the rest of the module by construction, including handlers
/// nobody has written yet.
///
/// Lines are joined before matching, because the status and the body sit on
/// different lines and a check that reads one line at a time would miss the
/// pairing entirely — the failure mode `CLAUDE.md` records for the
/// command-line gate.
#[test]
fn no_page_handler_answers_a_5xx_with_a_body_htmx_will_discard() {
    // `detection_detail.rs` renders a whole page rather than a partial: a 5xx
    // there is not swallowed by htmx, it is the browser's own error page, and
    // it fires only when the blocking task panics.
    const EXEMPT: &[&str] = &["detection_detail.rs"];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/pages");
    let mut offenders = Vec::new();
    let mut scanned = 0;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if EXEMPT.contains(&name.as_str()) {
                continue;
            }
            scanned += 1;
            let joined = std::fs::read_to_string(&path)
                .expect("read")
                .replace('\n', " ");
            if joined.contains("INTERNAL_SERVER_ERROR") {
                offenders.push(name);
            }
        }
    }
    assert!(
        scanned > 10,
        "precondition: expected to scan the page modules, scanned {scanned}"
    );
    assert!(
        offenders.is_empty(),
        "these page handlers answer a 5xx whose body htmx will discard: {offenders:?}. \
         Answer 200 with `error_states::failed_partial` instead."
    );
}
