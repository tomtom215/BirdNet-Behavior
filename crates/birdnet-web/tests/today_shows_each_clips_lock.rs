//! Today's full-day list shows each clip's real lock state (M9).
//!
//! Every card carried the same "🔒 Lock" button: a locked clip offered to be
//! locked again and could not be unlocked from Today at all, and a detection
//! with no clip offered a lock that protects nothing. The Recordings page
//! already renders the right control; Today now uses the same one.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

#[tokio::test]
async fn each_card_shows_its_own_lock_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        let today: String = conn
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .unwrap();
        for (time, sci, com, file) in [
            ("06:00:00", "Turdus merula", "Blackbird", Some("locked.wav")),
            ("07:00:00", "Erithacus rubecula", "Robin", Some("open.wav")),
            ("08:00:00", "Parus major", "Great Tit", None),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (?1, ?2, ?3, ?4, 0.9, ?5)",
                rusqlite::params![today, time, sci, com, file],
            )
            .unwrap();
        }
        assert!(
            birdnet_db::sqlite::lock_detection(conn, &today, "06:00:00", "Turdus merula").unwrap(),
            "precondition: the blackbird's clip is locked"
        );
    });

    let res = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/pages/today-list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    // One card per detection, cut at each card's opening tag.
    let card = |name: &str| -> String {
        html.split("tdl-card\"")
            .find(|c| c.contains(&format!(">{name}</a>")))
            .unwrap_or_else(|| panic!("no card for {name}"))
            .to_owned()
    };
    assert!(
        card("Blackbird").contains("/pages/recordings-unlock"),
        "a locked clip must offer to unlock: {}",
        card("Blackbird")
    );
    assert!(
        card("Robin").contains("/pages/recordings-lock"),
        "{}",
        card("Robin")
    );
    let tit = card("Great Tit");
    assert!(
        !tit.contains("-lock") && !tit.contains("-unlock"),
        "a detection with no clip has nothing to lock: {tit}"
    );
}
