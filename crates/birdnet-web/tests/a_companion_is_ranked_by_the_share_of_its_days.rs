//! A companion species is ranked by the share of its days it has in common with the bird asked
//! about, and the page says what that share is made of (UIC-04).
//!
//! Both companion tables (Patterns › Together's lookup and the species page) ranked by raw
//! shared days, so the commonest resident was every species' first "companion" and the answer
//! barely depended on the species asked about. The pair view had already left that measure for
//! the same reason. Nutcracker is heard on ten days; Pine Grosbeak on eight, each one a
//! Nutcracker day; House Sparrow on every day of the window.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

async fn get(state: &AppState, uri: &str) -> String {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_resident_heard_every_day_is_not_the_first_companion_on_either_page() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        // Days counted back from today: the window is "the last 30 days", whatever today is,
        // and nothing here depends on a month or a year boundary.
        for back in 1..=29 {
            let mut rows = vec![("09:00:00", "Passer domesticus", "House Sparrow")];
            if back <= 10 {
                rows.push(("07:00:00", "Nucifraga caryocatactes", "Nutcracker"));
            }
            if back <= 8 {
                rows.push(("07:30:00", "Pinicola enucleator", "Pine Grosbeak"));
            }
            for (time, sci, com) in rows {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES (DATE('now', 'localtime', ?1), ?2, ?3, ?4, 0.9)",
                    rusqlite::params![format!("-{back} days"), time, sci, com],
                )
                .unwrap();
            }
        }
    });

    for uri in [
        "/pages/companion-species?species=Nutcracker&days=30",
        "/pages/species-companions?name=Nutcracker",
    ] {
        let html = get(&state, uri).await;
        // The table body, row by row, as the reader meets it.
        let body = html
            .split("<tbody>")
            .nth(1)
            .unwrap_or_else(|| panic!("{uri}: a table: {html}"));
        let rows: Vec<&str> = body.split("<tr>").skip(1).collect();
        let order: Vec<&str> = rows
            .iter()
            .filter_map(|r| {
                ["Pine Grosbeak", "House Sparrow"]
                    .into_iter()
                    .find(|n| r.contains(&format!(">{n}</a>")))
            })
            .collect();
        // Counterpart: the resident is still listed, below the true companion.
        assert_eq!(order, ["Pine Grosbeak", "House Sparrow"], "{uri}: {body}");
        assert!(
            rows[0].contains("8 of the Nutcracker’s 10 days"),
            "{uri}: the first row says what its rank is made of: {}",
            rows[0]
        );
        assert!(
            rows[1].contains("10 of the Nutcracker’s 10 days") && rows[1].contains("34%"),
            "{uri}: the resident shares every one of the trigger's days, a third of its own: {}",
            rows[1]
        );
    }
}
