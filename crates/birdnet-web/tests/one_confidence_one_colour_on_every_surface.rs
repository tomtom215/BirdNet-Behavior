//! A confidence is the same colour wherever it is drawn.
//!
//! It was graded by five rules. The Today feed's bar: `> 90 %` high, `> 75 %`
//! mid, else no class. Every pill — the review queue, quarantine, the species
//! detail table, the dashboard card, the kiosk: `>= 80 %` high, `>= 50 %` mid,
//! else low (red). The share page: 85 / 60. So the same 82 % detection was
//! **amber** on the Today feed and **green** on the Review page, and with the
//! station's cut-off at 70–75 % every detection just above it drew grey, the
//! colour of a disabled control, although the station had accepted it.
//!
//! This drives the real routes with one station and one set of detections and
//! reads, on every surface that draws a confidence, the pair *(percent shown,
//! tier class)*. Every pair must agree with `confidence::tier` for the
//! station's own cut-off, and the surfaces must also agree with each other.
//!
//! The station's cut-off is set explicitly (70 %), not inherited from the
//! shipped default: the maintainer retunes the default and this test is about
//! the rule, not the constant. A second pass moves the cut-off to 85 % and
//! checks the same detections re-tier, which a surface that hard-coded a
//! threshold could not do.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// What `confidence::tier` says, restated here so a bug in the module cannot
/// hide itself: whole percent, 90 and up high, then relative to the cut-off.
fn expected(pct: u32, floor_pct: u32) -> &'static str {
    if pct >= 90 {
        "high"
    } else if pct < (floor_pct + 10).min(90) {
        "low"
    } else {
        "mid"
    }
}

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

/// Every `(percent, tier)` pair a page draws, from bars and pills alike.
///
/// A bar is `<span class="bnb-conf TIER" …>…<span class="val">NN%</span>`; a
/// pill is `<span class="bnb-conf-pill TIER">NN%`. The legacy `conf TIER` pill
/// is read too, so a surface that never moved to the shared atom is caught
/// with the old rule's answer rather than missed.
fn pairs(html: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for marker in [
        "class=\"bnb-conf ",
        "class=\"bnb-conf-pill ",
        "class=\"conf ",
    ] {
        let mut rest = html;
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            let tier: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
            // The first `NN%` after the class is this confidence's number.
            let Some(pct_at) = rest.find('%') else { break };
            let digits: String = rest[..pct_at]
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if let Ok(pct) = digits.parse::<u32>() {
                out.push((pct, tier));
            }
        }
    }
    out
}

fn station(floor: &str) -> (AppState, tempfile::TempDir) {
    // The directory is returned with the state: the database file has to
    // outlive every request the test makes.
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "confidence_threshold",
            floor,
            birdnet_db::settings::SettingsCategory::Detection,
        )
        .unwrap();
        let today: String = conn
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .unwrap();
        // One species per score, so the species list's *average* is that score
        // too. Every row has a clip, so Recordings lists it.
        for (i, (conf, sci, com)) in [
            (0.95, "Turdus merula", "Blackbird"),
            (0.85, "Erithacus rubecula", "Robin"),
            (0.82, "Parus major", "Great Tit"),
            (0.72, "Cyanistes caeruleus", "Blue Tit"),
        ]
        .into_iter()
        .enumerate()
        {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    today,
                    format!("0{}:00:00", 6 + i),
                    sci,
                    com,
                    conf,
                    format!("clip{i}.wav")
                ],
            )
            .unwrap();
        }
    });
    (state, dir)
}

async fn every_surface_agrees(floor: &str, floor_pct: u32) {
    let (state, _dir) = station(floor);
    let today: String = state.with_db(|c| {
        c.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .unwrap()
    });
    // (route, label). Every one draws at least one confidence.
    let surfaces = [
        ("/pages/today-list".to_string(), "Today full-day list"),
        ("/pages/detections".to_string(), "Today live feed"),
        ("/pages/recordings-clips".to_string(), "Recordings"),
        ("/species?view=list".to_string(), "Species list"),
        ("/pages/detection-reviews-queue".to_string(), "Review queue"),
        (format!("/reports/day?date={today}"), "Open day"),
        // The kiosk's plate (the default face) draws no confidence, so the gate grades the face
        // that does. Pointing it at the plate would fail "drew no confidence at all", which is
        // the plate being what it is and not a surface that disagrees.
        (
            "/pages/kiosk-content?mode=numbers".to_string(),
            "Kiosk (numbers)",
        ),
    ];
    // Collected, not asserted one by one: the failure is the whole list of
    // disagreements, which is what shows the defect was five rules and not one
    // surface's typo.
    let mut problems: Vec<String> = Vec::new();
    let mut by_pct: std::collections::BTreeMap<u32, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut all = surfaces.to_vec();
    all.push((
        format!("/detections/detail?date={today}&time=08:00:00&name=Great%20Tit"),
        "Detection page",
    ));
    for (uri, label) in &all {
        let html = get(&state, uri).await;
        let found = pairs(&html);
        // Without this the test passes for the wrong reason on a surface that
        // stopped drawing a confidence at all.
        assert!(
            !found.is_empty(),
            "{label} ({uri}) drew no confidence at all:\n{}",
            &html[..html.len().min(600)]
        );
        for (pct, tier) in &found {
            by_pct.entry(*pct).or_default().insert(tier.clone());
            if tier != expected(*pct, floor_pct) {
                problems.push(format!(
                    "{label}: {pct}% is drawn as `{tier}`, a {floor_pct}% station says `{}`",
                    expected(*pct, floor_pct)
                ));
            }
        }
    }
    problems.sort();
    problems.dedup();
    // Cross-surface: whatever tier one surface gave a percent, no other gave a
    // different one. Implied by the per-surface check; this names the symptom
    // the maintainer saw.
    for (pct, tiers) in by_pct {
        if tiers.len() > 1 {
            problems.push(format!(
                "{pct}% is drawn in more than one colour across the app: {tiers:?}"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{} disagreement(s) at a station whose cut-off is {floor_pct}%:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

#[tokio::test]
async fn at_a_seventy_percent_station_every_surface_colours_a_confidence_the_same_way() {
    every_surface_agrees("0.70", 70).await;
}

/// The same detections, a stricter station: 82 % and 85 % are now within ten
/// points of the cut-off. A surface with a threshold written into it would
/// still call them mid.
#[tokio::test]
async fn the_tiers_follow_the_stations_own_cut_off() {
    every_surface_agrees("0.85", 85).await;
}
