//! A reviewer's verdict has to change the numbers, or it is not a review.
//!
//! `detection_reviews` has stored confirmed/rejected verdicts since migration
//! 13, and exactly one surface ever read them: the quality dashboard's own
//! "Review verdict trend" panel. Every other analytic — species counts, the life
//! list, the heat map, the dawn chorus, phenology, every behavioural and
//! time-series query — counted a rejected detection exactly as it counted a
//! confirmed one.
//!
//! So an operator could spend a season rejecting false positives and every chart
//! would look exactly as it did before. The only way to make a rejection *mean*
//! anything was to delete the detection, which discards the evidence — the
//! opposite of what a reviewable record is for. For a research station that is
//! the difference between a log of what a model reported and a dataset a
//! reviewer stands behind.
//!
//! # The two halves this suite holds apart
//!
//! * **Aggregates exclude rejects.** A count is a claim about what was there,
//!   and a reviewer who rejected a detection has said it was not.
//! * **Record-level surfaces still show them.** A reviewer must be able to find
//!   a rejected detection, listen again and change their mind. A verdict that
//!   hid its own evidence would be a trap, and clearing it must bring the
//!   detection straight back.
//!
//! Both halves are gated, because a change that satisfied only the first would
//! look like a fix and be a data-loss bug.

#![cfg(feature = "analytics")]

use birdnet_db::sqlite::ReviewStatus;
use birdnet_web::state::AppState;

/// A station with three detections of three species on one day, synced.
fn station(dir: &std::path::Path) -> (AppState, String) {
    let db_path = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();

    let today: String = conn
        .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
        .expect("today");
    for (time, sci, com) in [
        ("06:15:00", "Turdus merula", "Eurasian Blackbird"),
        ("07:15:00", "Erithacus rubecula", "European Robin"),
        ("08:15:00", "Parus major", "Great Tit"),
    ] {
        // `File_Name` is set because `best_detections_for_date` filters on
        // `CLIP_AVAILABLE` — a fixture without one exercises none of it and
        // would let that surface pass the gate below by returning nothing. The
        // confidence is above 0.85 for the same reason: `/feeds/rare.*` filters
        // on `Confidence > 0.85`, so a fixture at exactly 0.85 makes the feed
        // empty and every assertion about it vacuous.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES (?1, ?2, ?3, ?4, 0.92, ?5)",
            rusqlite::params![
                &today,
                time,
                sci,
                com,
                format!("{today}-birdnet-{time}.wav")
            ],
        )
        .unwrap();
    }
    drop(conn);

    let state = AppState::new_with_analytics(db_path, &dir.join("analytics.duckdb"))
        .expect("analytics state opens");
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("initial sync");
    (state, today)
}

/// Record a rejection through the **paired** write the routes use.
///
/// Deliberately not `with_db(set_detection_review)`: that reaches SQLite alone,
/// which is precisely the defect. A test that used it would pass the SQLite
/// assertions and be blind to the half that matters for the behavioural
/// dashboards.
fn reject(state: &AppState, date: &str, time: &str, sci: &str, com: &str) {
    state
        .set_detection_review(
            date,
            time,
            sci,
            com,
            ReviewStatus::Rejected,
            Some("misidentified — traffic noise"),
        )
        .expect("record verdict");
}

/// Rows the SQLite aggregate surfaces see.
/// Rows the aggregates see — read through the **query layer**, not the view.
///
/// This used to be `SELECT COUNT(*) FROM detections_analytic`, which asserts
/// nothing: it re-states the view's own WHERE clause back to itself and passes
/// whether or not a single analytic ever reads the view. Its message claimed to
/// cover "species totals, the heat map, the dawn chorus, phenology"; it covered
/// none of them, and that is how a whole tile row of the dashboard came to
/// count rejected detections while the tiles beside it did not.
fn analytic_rows(state: &AppState) -> i64 {
    state
        .with_db(birdnet_db::sqlite::analytic_detection_count)
        .expect("count")
}

/// The four headline dashboard tiles, as the partial computes them.
///
/// `(all-time detections, species, today, species today)`. Deliberately the
/// same calls `routes::pages::dashboard::stats` makes, so a tile that reverts
/// to a raw `FROM detections` count fails here.
fn dashboard_tiles(state: &AppState, today: &str) -> (i64, i64, i64, i64) {
    state.with_db(|conn| {
        (
            birdnet_db::sqlite::analytic_detection_count(conn).unwrap_or(-1),
            birdnet_db::sqlite::species_count(conn).unwrap_or(-1),
            birdnet_db::sqlite::analytic_detection_count_for_date(conn, today).unwrap_or(-1),
            birdnet_db::sqlite::analytic_species_count_for_date(conn, today).unwrap_or(-1),
        )
    })
}

/// Rows the record-level surfaces see.
fn record_rows(state: &AppState) -> i64 {
    state
        .with_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM detections", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .expect("count")
}

/// Rows the DuckDB analytics read.
fn olap_analytic_rows(state: &AppState) -> i64 {
    state
        .with_analytics(|adb| {
            adb.conn()
                .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| {
                    r.get::<_, i64>(0)
                })
                .expect("count")
        })
        .expect("analytics is configured")
}

/// Rejecting a detection removes it from the SQLite aggregates.
#[test]
fn a_rejected_detection_leaves_the_sqlite_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(analytic_rows(&state), 3, "fixture");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    assert_eq!(
        analytic_rows(&state),
        2,
        "the rejected detection is still counted by every SQLite aggregate — \
         species totals, the heat map, the dawn chorus, phenology"
    );
    // The evidence survives.
    assert_eq!(
        record_rows(&state),
        3,
        "rejecting must annotate, never delete — the audio and the row stay"
    );
}

/// …and from the DuckDB analytics, which is where the behavioural and
/// time-series dashboards read.
///
/// Fixing only the SQLite half would look like a fix and leave every
/// behavioural dashboard still counting the reject.
#[test]
fn a_rejected_detection_leaves_the_duckdb_analytics() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_analytic_rows(&state), 3, "fixture");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    assert_eq!(
        olap_analytic_rows(&state),
        2,
        "the reject still counts in every behavioural and time-series dashboard"
    );
}

/// Unreviewed detections must stay in. The counterpart that stops the fix from
/// being a blanket "hide everything".
///
/// This is not hypothetical: SQL three-valued logic makes `<> 'rejected'`
/// evaluate to NULL for an unreviewed row, and `WHERE` treats NULL as false — so
/// the obvious spelling of this filter would have hidden every detection nobody
/// had looked at yet, which on a real station is nearly all of them.
#[test]
fn unreviewed_and_confirmed_detections_stay_in_the_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    state
        .set_detection_review(
            &today,
            "06:15:00",
            "Turdus merula",
            "Eurasian Blackbird",
            ReviewStatus::Confirmed,
            None,
        )
        .expect("confirm");

    assert_eq!(
        analytic_rows(&state),
        3,
        "confirming a detection, and leaving two unreviewed, must change nothing"
    );
    assert_eq!(olap_analytic_rows(&state), 3, "same in the OLAP copy");
}

/// Clearing a verdict brings the detection back.
///
/// Without this the exclusion would outlive the judgement that justified it,
/// and an accidental click would be unrecoverable through the UI.
#[test]
fn clearing_a_verdict_returns_the_detection_to_the_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );
    assert_eq!(analytic_rows(&state), 2, "rejected");

    state
        .clear_detection_review(&today, "07:15:00", "Erithacus rubecula")
        .expect("clear verdict");

    assert_eq!(
        analytic_rows(&state),
        3,
        "clearing the verdict must undo the exclusion"
    );
}

/// A verdict recorded before this feature existed takes effect on upgrade.
///
/// Migration 26 backfills `review_verdict` from `detection_reviews`. Without the
/// backfill, every verdict an operator had already recorded would keep counting
/// for nothing, and only reviews made after the upgrade would mean anything —
/// which is precisely the complaint the feature exists to answer.
#[test]
fn verdicts_recorded_before_the_upgrade_are_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();

    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-03-01','07:15:00','Erithacus rubecula','European Robin',0.8)",
        [],
    )
    .unwrap();
    // A verdict written straight to the table, as a pre-upgrade station holds
    // it: `detection_reviews` populated, `review_verdict` still NULL.
    conn.execute(
        "INSERT INTO detection_reviews (date, time, sci_name, com_name, status)
         VALUES ('2026-03-01','07:15:00','Erithacus rubecula','European Robin','rejected')",
        [],
    )
    .unwrap();
    conn.execute("UPDATE detections SET review_verdict = NULL", [])
        .unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1,
        "before the backfill the verdict counts for nothing — this is the state \
         every existing station is in"
    );

    // Re-running the chain is a no-op for applied migrations, so drive the
    // backfill the way the migration does.
    conn.execute(
        "UPDATE detections
            SET review_verdict = (
                  SELECT r.status FROM detection_reviews r
                   WHERE r.date = detections.Date
                     AND r.time = detections.Time
                     AND r.sci_name = detections.Sci_Name)
          WHERE EXISTS (
                  SELECT 1 FROM detection_reviews r
                   WHERE r.date = detections.Date
                     AND r.time = detections.Time
                     AND r.sci_name = detections.Sci_Name)",
        [],
    )
    .unwrap();

    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "the backfill must make an already-recorded verdict count"
    );
}

/// Every tile on the dashboard's headline row must agree about the same day.
///
/// Three of them ("Species", "Last hour", the 12-day sparkline) have always
/// read `detections_analytic`; three ("Detections", "Today", "Species today")
/// counted every row including rejections. So the screen contradicted itself,
/// by exactly the number of rejections the operator had recorded — and the
/// contradiction grew the more carefully they curated.
#[test]
fn the_dashboard_tile_row_agrees_with_itself_about_a_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(
        dashboard_tiles(&state, &today),
        (3, 3, 3, 3),
        "fixture: three detections of three species, all today"
    );

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    assert_eq!(
        dashboard_tiles(&state, &today),
        (2, 2, 2, 2),
        "all four tiles must drop the rejected detection; a tile still reading \
         `FROM detections` shows 3 here while the tile beside it shows 2"
    );
}

/// The counterpart, so the gate above cannot be satisfied by tiles that simply
/// undercount: with nothing rejected, every tile must still see all three.
#[test]
fn the_dashboard_tile_row_counts_everything_when_nothing_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(dashboard_tiles(&state, &today), (3, 3, 3, 3));
}

// ---------------------------------------------------------------------------
// Aggregates that were still counting rejections
// ---------------------------------------------------------------------------

/// Every *aggregate* must drop a rejection — not just the ones that happened to
/// be wired first.
///
/// `detections_analytic` landed in migration 26, and the surfaces converted to
/// it were the ones someone thought of at the time. This enumerates the rest by
/// name, so "which aggregates honour a verdict?" has a single answer that is
/// checked rather than remembered. Each is a question about *what was there*,
/// which is exactly what a reviewer's rejection answers.
///
/// Record-level surfaces are deliberately absent: `recent_detections`,
/// `todays_detections`, `recent_clips` and the detail page must keep showing a
/// rejected detection, because the review queue holds only the last 25 verdicts
/// and hiding it everywhere else would make an older rejection unreachable.
#[test]
fn every_aggregate_drops_a_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    let before = state.with_db(|conn| {
        (
            birdnet_db::sqlite::species_for_date(conn, &today)
                .unwrap()
                .len(),
            birdnet_db::sqlite::detections_per_day(conn).unwrap()[0].count,
            birdnet_db::sqlite::detection_dates(conn, 10).unwrap().len(),
            birdnet_db::sqlite::best_detections_for_date(conn, &today, 10)
                .unwrap()
                .len(),
            birdnet_db::sqlite::detection_count_for_species_date(
                conn,
                &today,
                "Erithacus rubecula",
            )
            .unwrap(),
        )
    });
    assert_eq!(
        before,
        (3, 3, 1, 3, 1),
        "fixture: three species, one day, one Robin"
    );

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    let after = state.with_db(|conn| {
        (
            birdnet_db::sqlite::species_for_date(conn, &today)
                .unwrap()
                .len(),
            birdnet_db::sqlite::detections_per_day(conn).unwrap()[0].count,
            birdnet_db::sqlite::detection_dates(conn, 10).unwrap().len(),
            birdnet_db::sqlite::best_detections_for_date(conn, &today, 10)
                .unwrap()
                .len(),
            birdnet_db::sqlite::detection_count_for_species_date(
                conn,
                &today,
                "Erithacus rubecula",
            )
            .unwrap(),
        )
    });
    assert_eq!(
        after,
        (2, 2, 1, 2, 0),
        "species_for_date, detections_per_day, best_detections_for_date and \
         detection_count_for_species_date must all drop the rejected Robin; \
         detection_dates still reports the day, because two detections remain on it"
    );
}

/// The counterpart: with nothing rejected, every one of those aggregates must
/// still see everything. Without this, a change that simply returned less would
/// satisfy the gate above.
#[test]
fn every_aggregate_counts_everything_when_nothing_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    let counts = state.with_db(|conn| {
        (
            birdnet_db::sqlite::species_for_date(conn, &today)
                .unwrap()
                .len(),
            birdnet_db::sqlite::detections_per_day(conn).unwrap()[0].count,
            birdnet_db::sqlite::best_detections_for_date(conn, &today, 10)
                .unwrap()
                .len(),
        )
    });
    assert_eq!(counts, (3, 3, 3));
}

/// A day whose every detection was rejected is no longer a day with data.
///
/// `detection_dates` drives the history calendar's "which days can I open?".
/// Offering a day that then renders empty is a dead end, and the calendar's own
/// per-day counts already come from `detections_analytic`.
#[test]
fn a_fully_rejected_day_leaves_the_history_calendar() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    for (time, sci, com) in [
        ("06:15:00", "Turdus merula", "Eurasian Blackbird"),
        ("07:15:00", "Erithacus rubecula", "European Robin"),
        ("08:15:00", "Parus major", "Great Tit"),
    ] {
        reject(&state, &today, time, sci, com);
    }
    let dates = state.with_db(|conn| birdnet_db::sqlite::detection_dates(conn, 10).unwrap());
    assert!(
        dates.is_empty(),
        "a day with nothing left to show must not appear in the calendar; got {dates:?}"
    );
}

// ---------------------------------------------------------------------------
// Published and rendered surfaces
// ---------------------------------------------------------------------------

/// Fetch a path through the real router and return `(status, body)`.
async fn get(state: &AppState, uri: &str) -> (axum::http::StatusCode, String) {
    use tower::ServiceExt as _;
    let response = birdnet_web::server::build_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The feeds are *published*: an RSS reader, a calendar subscription, anything
/// pointed at the station. A rejection has to reach them, and they were the
/// surface where it mattered most — `/feeds/rare.rss` reports a species' first
/// detection via `MIN(Date)`, so a rejected row that happens to be the earliest
/// announces a "new species" on a date the life list does not agree with, to an
/// audience that never sees the correction.
#[tokio::test]
async fn the_published_feeds_drop_a_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    for path in ["/feeds/rare.rss", "/feeds/today.rss", "/feeds/rare.ics"] {
        let (status, body) = get(&state, path).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{path}");
        assert!(
            body.contains("European Robin"),
            "fixture: {path} should list the Robin before it is rejected"
        );
    }

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    for path in ["/feeds/rare.rss", "/feeds/today.rss", "/feeds/rare.ics"] {
        let (_, body) = get(&state, path).await;
        assert!(
            !body.contains("European Robin"),
            "{path} still publishes a detection the reviewer rejected"
        );
        assert!(
            body.contains("Eurasian Blackbird"),
            "{path} must still publish the detections nobody rejected"
        );
    }
}

/// The command palette ranks species by detection count and lists the most
/// recent — both aggregates.
#[tokio::test]
async fn the_command_palette_drops_a_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    let (_, before) = get(&state, "/pages/cmdk?q=robin").await;
    assert!(before.contains("European Robin"), "fixture");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    let (_, after) = get(&state, "/pages/cmdk?q=robin").await;
    assert!(
        !after.contains("European Robin"),
        "the palette still offers a species whose only detection was rejected"
    );
    let (_, blackbird) = get(&state, "/pages/cmdk?q=blackbird").await;
    assert!(
        blackbird.contains("Eurasian Blackbird"),
        "the palette must still find species nobody rejected"
    );
}

/// `/api/v2/metrics` exports the raw row count *and* the rejection count, so a
/// dashboard can show either the pipeline's throughput or the curated figure
/// the web UI displays. Exporting only one is what let the UI's own tiles
/// disagree; exporting neither answer is worse than exporting both.
#[tokio::test]
async fn metrics_exports_both_the_raw_and_the_curated_view() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    let (status, body) = get(&state, "/api/v2/metrics").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        body.contains("birdnet_detections_total 3"),
        "the raw count is pipeline throughput and must keep counting every row"
    );
    assert!(
        body.contains("birdnet_detections_rejected_total 1"),
        "the rejection count must be exported so `total - rejected` is derivable"
    );
    assert!(
        body.contains("birdnet_species_total 2"),
        "the species gauge is an analytic and must drop the rejected species"
    );
}

/// The Patterns page must carry the provenance slot, and the partial must
/// render nothing on a station that imported nothing.
///
/// Two halves, because either alone is satisfiable without the other: a slot
/// that is never populated says nothing, and a partial nothing embeds is
/// unreachable. Migration 25 recorded provenance for a year and nothing read
/// it — this is the gate that something does.
#[tokio::test]
async fn the_patterns_page_carries_the_provenance_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station(dir.path());

    let (status, body) = get(&state, "/patterns").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        body.contains(r#"hx-get="/pages/provenance-note""#),
        "the Patterns page must pull in the provenance note"
    );

    let (status, note) = get(&state, "/pages/provenance-note").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        note.trim().is_empty(),
        "a station that imported nothing must see nothing: {note:?}"
    );
}

/// …and it says so once a genuinely different site has been imported.
#[tokio::test]
async fn the_provenance_note_reports_an_imported_foreign_site() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO import_batches
               (imported_at, source_kind, source_label, distance_km, applied_shift_secs, row_count)
             VALUES (datetime('now'), 'birdnet-pi', 'Coastal site', 341.0, -21600, 1)",
            [],
        )
        .expect("batch");
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
             VALUES (?1, '05:00:00', 'Larus argentatus', 'Herring Gull', 0.9, ?2)",
            rusqlite::params![&today, id],
        )
        .expect("imported detection");
    });

    let (_, note) = get(&state, "/pages/provenance-note").await;
    assert!(note.contains("Coastal site"), "{note}");
    assert!(note.contains("341 km"), "{note}");
    assert!(
        note.contains("with a clock correction applied"),
        "the operator needs to know whether the two histories share a clock: {note}"
    );
}

// ---------------------------------------------------------------------------
// Reaching a rejection you recorded a long time ago
// ---------------------------------------------------------------------------

/// Every rejection must stay reachable through the UI, however many verdicts
/// came after it.
///
/// This is the counterweight to the aggregates excluding rejections. Once they
/// do, the review page's verdict list is the *only* surface in the app that
/// lists a rejected detection — and it asked for the newest 25. A station that
/// reviews diligently passes 25 verdicts in a week, after which an older
/// rejection was reachable through no page at all, only by a URL the operator
/// happened to have kept. A verdict you cannot find is one you cannot undo, and
/// the entire design rests on rejection being reversible.
#[tokio::test]
async fn an_old_rejection_is_still_reachable_through_the_review_page() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    // The rejection we will have to find again, recorded first.
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );
    // Then 40 confirmations on top of it — more than the page's 25.
    state.with_db(|conn| {
        for i in 0..40 {
            let time = format!("1{i:01}:{:02}:00", i % 60);
            birdnet_db::sqlite::set_detection_review(
                conn,
                &today,
                &time,
                "Parus major",
                "Great Tit",
                ReviewStatus::Confirmed,
                None,
            )
            .expect("confirm");
        }
    });

    // The default view no longer shows it — that is the situation, not a bug.
    let (_, first_page) = get(&state, "/pages/detection-reviews-queue").await;
    assert!(
        !first_page.contains("European Robin"),
        "fixture: 40 newer verdicts must have pushed it off the first page"
    );

    // Filtering to rejections finds it immediately.
    let (status, rejected) = get(&state, "/pages/detection-reviews-queue?status=rejected").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        rejected.contains("European Robin"),
        "filtering to rejections must surface it: {rejected}"
    );

    // And paging through the unfiltered history reaches it too.
    let (_, page_two) = get(&state, "/pages/detection-reviews-queue?offset=25").await;
    assert!(
        page_two.contains("European Robin"),
        "paging back must reach it: {page_two}"
    );
}

/// The pager must offer a page only when there is one, and say where you are.
///
/// A pager that offers a page which turns out to be empty is the same lie as a
/// list that silently ends.
#[tokio::test]
async fn the_verdict_pager_only_offers_pages_that_exist() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    // One verdict: nothing to page to.
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );
    let (_, one) = get(&state, "/pages/detection-reviews-queue").await;
    assert!(
        !one.contains("Older &rarr;"),
        "nothing older to offer: {one}"
    );
    assert!(!one.contains("&larr; Newer"));

    // Thirty: one page more.
    state.with_db(|conn| {
        for i in 0..30 {
            let time = format!("1{i:01}:{:02}:00", i % 60);
            birdnet_db::sqlite::set_detection_review(
                conn,
                &today,
                &time,
                "Parus major",
                "Great Tit",
                ReviewStatus::Confirmed,
                None,
            )
            .expect("confirm");
        }
    });
    let (_, many) = get(&state, "/pages/detection-reviews-queue").await;
    assert!(
        many.contains("Older &rarr;"),
        "31 verdicts is two pages: {many}"
    );
    assert!(
        many.contains("1&ndash;25 of 31"),
        "the pager must say where you are: {many}"
    );

    let (_, last) = get(&state, "/pages/detection-reviews-queue?offset=25").await;
    assert!(last.contains("&larr; Newer"));
    assert!(
        !last.contains("Older &rarr;"),
        "the last page offers no next: {last}"
    );
}

/// A mistyped status is a view filter that did not match, not an error page.
/// Answering "where is my verdict" with a 500 helps nobody.
#[tokio::test]
async fn an_unknown_status_filter_falls_back_to_showing_everything() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );
    let (status, body) = get(&state, "/pages/detection-reviews-queue?status=nonsense").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(body.contains("European Robin"), "{body}");
    assert!(body.contains("All verdicts"), "{body}");
}

/// A share link is a publication, and a rejection withdraws the claim.
///
/// This is the one record-level surface that must *stop* showing a rejected
/// detection. Everywhere else the reasoning runs the other way — a reviewer has
/// to be able to find what they rejected and change their mind — but nobody
/// holding a share link is going to change their mind about anything, and the
/// link is the only surface in the app that shows a detection to someone who
/// cannot see the review queue.
#[tokio::test]
async fn a_share_link_stops_resolving_once_the_detection_is_rejected() {
    // SAFETY-of-behaviour note: the share token is HMAC'd with a secret read
    // from the environment, so this drives the same encoder the routes use
    // rather than hand-building a token.
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    // An hour of validity is plenty for a test and exercises the real encoder
    // rather than a hand-built token.
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let token = birdnet_web::routes::share::encode_share_token(
        &today,
        "07:15:00",
        "European Robin",
        expiry,
    );
    let (status, body) = get(&state, &format!("/r/{token}")).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "fixture: the link resolves before the rejection"
    );
    assert!(body.contains("European Robin"), "{body}");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    let (status, body) = get(&state, &format!("/r/{token}")).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "a withdrawn claim must stop being served"
    );
    assert!(
        !body.contains("European Robin"),
        "the 404 page must not still name the species: {body}"
    );
}

/// The counterpart: a link to a detection nobody rejected keeps working. A
/// change that broke every share link would satisfy the gate above.
#[tokio::test]
async fn a_share_link_to_an_unreviewed_detection_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let token = birdnet_web::routes::share::encode_share_token(
        &today,
        "06:15:00",
        "Eurasian Blackbird",
        expiry,
    );
    let (status, body) = get(&state, &format!("/r/{token}")).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(body.contains("Eurasian Blackbird"), "{body}");
}
