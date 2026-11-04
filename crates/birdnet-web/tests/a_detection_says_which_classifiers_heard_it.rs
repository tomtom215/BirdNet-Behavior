//! A detection says which classifiers heard it, and Station Health lists the
//! classifiers the station runs.
//!
//! The pieces are tested where they live: the table and its readers in
//! `birdnet-db`, the two renderers in this crate's unit tests. What neither
//! covers is that they are *connected*: a page that never reads the scores, or a
//! Health tab that never renders the board, passes every one of them.
//!
//! The thing this guards is that a classifier's silence is never reported as its
//! opinion, and that a one-classifier station is told nothing about a second
//! opinion it never sought.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_core::inference::health::{ErrorClass, FAILING_AFTER, ModelHealth};
use birdnet_db::sqlite::{ModelScore, ModelScoreState, insert_model_score};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const DATE: &str = "2026-05-01";
const TIME: &str = "06:00:00";
const SCI: &str = "Dryobates villosus";

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

fn add_detection(state: &AppState, model: Option<&str>, agreement: Option<i64>) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs, model_id, model_agreement)
             VALUES (?1, ?2, ?3, 'Hairy Woodpecker', 0.9, 0.7, 18, 1.25, 0.0, 'x.wav', 0, ?4, ?5)",
            rusqlite::params![DATE, TIME, SCI, model, agreement],
        )
        .expect("insert detection");
    });
}

fn store(
    state: &AppState,
    model: &str,
    st: ModelScoreState,
    score: Option<f64>,
    own_bar: Option<f64>,
) {
    state.with_db(|conn| {
        insert_model_score(
            conn,
            DATE,
            TIME,
            SCI,
            &ModelScore {
                model_id: model.to_owned(),
                state: st,
                score,
                bar: own_bar,
                window_start_secs: Some(0.0),
                window_end_secs: Some(3.0),
            },
        )
        .expect("store score");
    });
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
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

async fn detail(state: &AppState) -> String {
    let (status, body) = get(
        state,
        &format!("/detections/detail?date={DATE}&time={TIME}&name=Hairy%20Woodpecker"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

/// The card's own region, so a phrase in a template comment or elsewhere on the
/// page cannot satisfy an assertion about it.
fn card(body: &str) -> Option<&str> {
    let start = body.find("ws-card")?;
    let rest = &body[start..];
    let end = rest
        .find("</table>")
        .or_else(|| rest.find("</p>\n</div>"))?;
    Some(&rest[..end])
}

#[tokio::test]
async fn two_classifiers_that_heard_it_are_named_with_each_ones_own_score() {
    let (_dir, state) = station();
    add_detection(&state, Some("birdnet-v3"), Some(2));
    store(
        &state,
        "birdnet-v3",
        ModelScoreState::Heard,
        Some(0.91),
        Some(0.70),
    );
    store(
        &state,
        "perch-v2",
        ModelScoreState::Heard,
        Some(0.64),
        Some(0.50),
    );
    let body = detail(&state).await;
    let card = card(&body).expect("the Who heard it card renders");
    assert!(card.contains("Who heard it"), "{card}");
    assert!(
        card.contains("Heard by 2 of 2 models that could judge it."),
        "{card}"
    );
    assert!(
        card.contains("<b>91%</b>, cleared its bar of 70%"),
        "{card}"
    );
    assert!(
        card.contains("<b>64%</b>, cleared its bar of 50%"),
        "{card}"
    );
}

#[tokio::test]
async fn a_model_that_scored_it_below_its_bar_is_a_split_and_a_model_without_the_species_is_not_a_vote()
 {
    let (_dir, state) = station();
    add_detection(&state, Some("birdnet-v3"), Some(1));
    store(
        &state,
        "birdnet-v3",
        ModelScoreState::Heard,
        Some(0.88),
        Some(0.70),
    );
    store(
        &state,
        "perch-v2",
        ModelScoreState::BelowBar,
        Some(0.33),
        Some(0.50),
    );
    store(&state, "regional", ModelScoreState::NoLabel, None, None);
    let body = detail(&state).await;
    let card = card(&body).expect("card");
    assert!(
        card.contains("Heard by 1 of 2 models that could judge it."),
        "{card}"
    );
    assert!(
        card.contains("Scored it <b>33%</b>, under its bar of 50%"),
        "{card}"
    );
    assert!(
        card.contains("Has no class for this species, so it cannot judge"),
        "{card}"
    );
    assert!(
        card.contains("1 model has no class for this species"),
        "{card}"
    );
}

#[tokio::test]
async fn a_one_classifier_station_is_never_shown_a_second_opinion_it_did_not_seek() {
    let (_dir, state) = station();
    add_detection(&state, Some("birdnet"), Some(1));
    let body = detail(&state).await;
    assert!(
        !body.contains("Who heard it"),
        "one classifier, nothing stored, no card"
    );
}

#[tokio::test]
async fn a_row_from_before_the_columns_existed_says_nothing_about_classifiers() {
    let (_dir, state) = station();
    add_detection(&state, None, None);
    let body = detail(&state).await;
    assert!(!body.contains("Who heard it"));
}

#[tokio::test]
async fn the_row_alone_can_still_say_that_several_classifiers_cleared_their_bars() {
    let (_dir, state) = station();
    add_detection(&state, Some("birdnet-v3"), Some(2));
    let body = detail(&state).await;
    assert!(
        body.contains("2 classifiers each cleared their own bar"),
        "{body}"
    );
    assert!(body.contains("were not kept for this detection"));
}

#[tokio::test]
async fn scores_that_cannot_be_read_are_a_fault_and_the_rest_of_the_page_still_renders() {
    let (_dir, state) = station();
    add_detection(&state, Some("birdnet-v3"), Some(2));
    state.with_db(|c| {
        c.execute_batch("DROP TABLE detection_model_scores")
            .expect("drop");
    });
    let body = detail(&state).await;
    assert!(
        body.contains("couldn&#39;t load which classifiers heard this")
            || body.contains("couldn't load which classifiers heard this"),
        "a failed read must say so, not look like nobody else heard it"
    );
    assert!(
        !body.contains("Who heard it"),
        "no card built on a read that failed"
    );
    assert!(
        body.contains("This detection"),
        "the detection itself still renders"
    );
}

fn health(windows: u64, ok: bool) -> ModelHealth {
    ModelHealth {
        consecutive_failures: if ok { 0 } else { FAILING_AFTER },
        windows,
        last_window_at: Some(birdnet_core::inference::health::unix_now()),
        last_success_at: ok.then(birdnet_core::inference::health::unix_now),
        error_class: (!ok).then_some(ErrorClass::Inference),
        ..ModelHealth::default()
    }
}

#[tokio::test]
async fn station_health_has_no_models_section_until_the_daemon_has_reported() {
    let (_dir, state) = station();
    let (status, body) = get(&state, "/station").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body.contains("ws-models"),
        "nothing reported, nothing to show"
    );
}

#[tokio::test]
async fn station_health_lists_each_classifier_with_the_detections_it_supplied_today() {
    let (_dir, state) = station();
    state.metrics().set_model_health(&[
        ("birdnet-v3".to_owned(), health(120, true)),
        ("perch-v2".to_owned(), health(118, true)),
    ]);
    state.with_db(|conn| {
        for (i, (model, agree)) in [("birdnet-v3", 2), ("birdnet-v3", 1), ("perch-v2", 1)]
            .into_iter()
            .enumerate()
        {
            conn.execute(
                "INSERT INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                      File_Name, chunk_offset_secs, model_id, model_agreement)
                 VALUES (date('now', 'localtime'), ?1, ?2, ?2, 0.9, 0.7, 18, 1.25, 0.0, 'x.wav', 0, ?3, ?4)",
                rusqlite::params![format!("06:0{i}:00"), format!("Species {i}"), model, agree],
            )
            .expect("insert");
        }
    });
    let (status, body) = get(&state, "/station").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("ws-models"), "{body}");
    let start = body.find("ws-models").expect("board");
    let board = &body[start..];
    assert!(
        board.contains(">birdnet-v3<") && board.contains(">perch-v2<"),
        "{board}"
    );
    // birdnet-v3 supplied two of today's three answers and one was cleared by a second model.
    let v3 = board.split("birdnet-v3").nth(1).expect("v3 card");
    let v3 = v3.split("perch-v2").next().expect("before perch card");
    assert!(
        v3.contains("<b class=\"mono\">2</b> detections today"),
        "{v3}"
    );
    assert!(
        v3.contains("<b class=\"mono\">1</b> of them another classifier also cleared"),
        "{v3}"
    );
}

#[tokio::test]
async fn a_classifier_failing_every_window_says_why_on_its_card() {
    let (_dir, state) = station();
    state.metrics().set_model_health(&[
        ("birdnet-v3".to_owned(), health(120, true)),
        ("perch-v2".to_owned(), health(118, false)),
    ]);
    let (_status, body) = get(&state, "/station").await;
    let start = body.find("ws-models").expect("board");
    let board = &body[start..];
    assert!(board.contains("Failing"), "{board}");
    assert!(
        board.contains(&format!("{FAILING_AFTER} windows in a row failed")),
        "{board}"
    );
}
