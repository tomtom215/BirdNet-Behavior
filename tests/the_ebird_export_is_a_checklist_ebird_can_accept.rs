//! The eBird export is the one surface whose output leaves the station and
//! enters a public database, so what it writes is held to eBird's rules rather
//! than ours. Each gate here was observed failing against the shipped export
//! before the fix; the failure text is in the commit message.
//!
//! Reference behaviour, checked rather than remembered:
//! * eBird's "Upload spreadsheet data" guidance (fetched 2026-09-07): the
//!   Record Format has **no header row**; `Number` may be `X` for "present but
//!   not counted"; `Protocol` is one word such as `Stationary`; coordinates are
//!   optional decimal degrees; `All observations reported` is a single `Y`/`N`.
//! * BirdNET-Pi `scripts/history.php` at `88985a3`: `Confidence > 0.75`, one
//!   record per species per hour, duration 60, protocol and observer count
//!   from the operator's form, coordinates from the station config.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt;

use birdnet_db::settings::{self, SettingsCategory};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// `(date, time, sci, com, confidence, verdict)`.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    f64,
    Option<&'static str>,
);

const BLACKBIRD: (&str, &str) = ("Turdus merula", "Eurasian Blackbird");
const ROBIN: (&str, &str) = ("Erithacus rubecula", "European Robin");

fn station(rows: &[Row], located: bool) -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    for (date, time, sci, com, conf, verdict) in rows {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, review_verdict)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![date, time, sci, com, conf, verdict],
        )
        .unwrap();
    }
    if located {
        settings::set(&conn, "latitude", "51.4769", SettingsCategory::Location).unwrap();
        settings::set(&conn, "longitude", "-0.0005", SettingsCategory::Location).unwrap();
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn export(state: AppState, query: &str) -> (StatusCode, String) {
    let app = build_router(state);
    let uri = format!("/api/v2/detections/export/ebird{query}");
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Split a CSV body into records (RFC 4180 quoting honoured), ignoring the
/// trailing newline.
fn records(body: &str) -> Vec<Vec<String>> {
    body.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut fields = Vec::new();
            let mut field = String::new();
            let mut quoted = false;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '"' if quoted && chars.peek() == Some(&'"') => {
                        field.push('"');
                        chars.next();
                    }
                    '"' => quoted = !quoted,
                    ',' if !quoted => fields.push(std::mem::take(&mut field)),
                    _ => field.push(c),
                }
            }
            fields.push(field);
            fields
        })
        .collect()
}

/// A checklist that is not placed anywhere must say so with blanks, not with
/// the coordinates of Null Island; and a station that knows where it is must
/// put that on every record without being asked.
#[tokio::test]
async fn coordinates_come_from_the_station_and_are_never_null_island() {
    let rows: Vec<Row> = vec![(
        "2026-03-12",
        "06:30:00",
        BLACKBIRD.0,
        BLACKBIRD.1,
        0.9,
        None,
    )];

    let (status, body) = export(station(&rows, false), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(recs.len(), 1, "{body}");
    assert_eq!(
        (recs[0][6].as_str(), recs[0][7].as_str()),
        ("", ""),
        "an unlocated station wrote coordinates it does not have: {body}"
    );
    assert!(
        !body.contains(",0,0,"),
        "the export placed the station at Null Island: {body}"
    );

    let (status, body) = export(station(&rows, true), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(
        (recs[0][6].as_str(), recs[0][7].as_str()),
        ("51.4769", "-0.0005"),
        "the configured location did not reach the checklist: {body}"
    );
}

/// The counterpart: an explicit coordinate still wins over the configured one,
/// and half a coordinate is an error rather than a guess.
#[tokio::test]
async fn a_caller_supplied_coordinate_overrides_the_configured_one() {
    let rows: Vec<Row> = vec![(
        "2026-03-12",
        "06:30:00",
        BLACKBIRD.0,
        BLACKBIRD.1,
        0.9,
        None,
    )];
    let (status, body) = export(station(&rows, true), "?lat=48.8566&lon=2.3522").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(
        (recs[0][6].as_str(), recs[0][7].as_str()),
        ("48.8566", "2.3522"),
        "{body}"
    );

    let (status, body) = export(station(&rows, true), "?lat=48.8566").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = export(station(&rows, true), "?lat=91&lon=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// A checklist is a claim of presence. Detections under the floor are the ones
/// that must not be claimed — and the floor is the operator's to move, so the
/// counterpart admits the same row at a lower floor.
#[tokio::test]
async fn detections_below_the_confidence_floor_are_not_claimed() {
    let rows: Vec<Row> = vec![
        (
            "2026-03-12",
            "06:30:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.9,
            None,
        ),
        ("2026-03-12", "06:31:00", ROBIN.0, ROBIN.1, 0.5, None),
    ];
    let (status, body) = export(station(&rows, true), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains(ROBIN.1),
        "a 0.50-confidence detection was claimed on the checklist: {body}"
    );
    assert!(body.contains(BLACKBIRD.1), "{body}");

    let (status, body) = export(station(&rows, true), "?min_confidence=0.4").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains(ROBIN.1),
        "lowering the floor must admit the row it excluded: {body}"
    );

    let (status, _) = export(station(&rows, true), "?min_confidence=1.5").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// An autonomous recorder counts vocalisations, not birds. One species in one
/// hour is one record whose `Number` is `X` (present, not counted) with the
/// tally kept in the comment; a second hour is a second record.
#[tokio::test]
async fn a_species_is_one_record_per_hour_marked_present_not_counted() {
    let rows: Vec<Row> = vec![
        (
            "2026-03-12",
            "06:05:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.8,
            None,
        ),
        (
            "2026-03-12",
            "06:30:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.9,
            None,
        ),
        (
            "2026-03-12",
            "06:45:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.85,
            None,
        ),
        (
            "2026-03-12",
            "07:10:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.8,
            None,
        ),
    ];
    let (status, body) = export(station(&rows, true), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(
        recs.len(),
        2,
        "expected one record per species per hour (06 and 07): {body}"
    );
    for rec in &recs {
        assert_eq!(
            rec[3], "X",
            "a detection tally was written as a count of birds: {body}"
        );
        assert_eq!(rec[14], "60", "duration is the hour, in minutes: {body}");
    }
    assert_eq!(recs[0][9], "06:00", "{body}");
    assert_eq!(recs[1][9], "07:00", "{body}");
    assert!(
        body.contains("3 detections"),
        "the tally must survive in the comment: {body}"
    );
}

/// A reviewer's rejection is the one verdict the station has; it must never
/// be undone at the surface that publishes.
#[tokio::test]
async fn a_rejected_detection_never_reaches_the_checklist() {
    let rows: Vec<Row> = vec![
        (
            "2026-03-12",
            "06:30:00",
            BLACKBIRD.0,
            BLACKBIRD.1,
            0.9,
            Some("rejected"),
        ),
        (
            "2026-03-12",
            "06:31:00",
            ROBIN.0,
            ROBIN.1,
            0.9,
            Some("confirmed"),
        ),
    ];
    let (status, body) = export(station(&rows, true), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains(BLACKBIRD.1),
        "a rejected detection was exported to eBird: {body}"
    );
    assert!(body.contains(ROBIN.1), "{body}");
}

/// Protocol, observer count, region and completeness are the submitter's
/// facts, not the station's, so they come from the caller; and the file is a
/// Record Format file, which has no header row.
#[tokio::test]
async fn effort_fields_are_the_callers_and_the_file_has_no_header_row() {
    let rows: Vec<Row> = vec![(
        "2026-03-12",
        "06:30:00",
        BLACKBIRD.0,
        BLACKBIRD.1,
        0.9,
        None,
    )];
    let (status, body) = export(station(&rows, true), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(
        recs[0][0], BLACKBIRD.1,
        "the first row must be an observation, not a header: {body}"
    );
    assert_eq!(recs[0].len(), 19, "{body}");
    assert_eq!(
        (
            recs[0][12].as_str(),
            recs[0][13].as_str(),
            recs[0][15].as_str()
        ),
        ("Stationary", "1", "N"),
        "{body}"
    );

    let (status, body) = export(
        station(&rows, true),
        "?protocol=Traveling&observers=2&state=ENG&country=GB&complete=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recs = records(&body);
    assert_eq!(
        (
            recs[0][10].as_str(),
            recs[0][11].as_str(),
            recs[0][12].as_str(),
            recs[0][13].as_str(),
            recs[0][15].as_str()
        ),
        ("ENG", "GB", "Traveling", "2", "Y"),
        "{body}"
    );
}
