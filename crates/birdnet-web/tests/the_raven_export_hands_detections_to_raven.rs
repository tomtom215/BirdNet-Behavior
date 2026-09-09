//! A detection can be handed to Raven and Audacity (FR-1).
//!
//! Before this, no output of the station was one a verification tool reads:
//! the ecologist opened the clip and found the call by ear. Gates:
//!
//! 1. The combined table is the BirdNET-Analyzer format, column for column,
//!    with each selection placed by the offset the extractor recorded, the
//!    whole clip for a row from before that column, and no row for a
//!    detection that cannot be placed in any file.
//! 2. The species code is the eBird code the station's label file supplies,
//!    and the scientific name when it does not.
//! 3. A clip has its own table and its own Audacity label track; a clip no
//!    detection names is a 404, and a path that is not a bare filename a 400.
//!
//! (`the_bulk_exports_honour_the_verdict.rs` holds that a rejected detection
//! is in none of it.)

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections
             (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Duration_Secs,
              clip_offset_secs, detection_secs, chunk_offset_secs)
         VALUES
             ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91234,
              'a.wav', 6.0, 1.5, 3.0, 0),
             ('2026-09-08', '06:10:03', 'Erithacus rubecula', 'European Robin', 0.8,
              'a.wav', 6.0, 4.5, 1.5, 3),
             ('2026-09-07', '05:00:00', 'Cuculus canorus', 'Common Cuckoo', 0.7,
              'b.wav', 6.0, NULL, NULL, 0),
             ('2026-09-06', '05:00:00', 'Cuculus canorus', 'Common Cuckoo', 0.7,
              'c.wav', NULL, NULL, NULL, 0),
             ('2026-09-05', '05:00:00', 'Cuculus canorus', 'Common Cuckoo', 0.7,
              NULL, NULL, NULL, NULL, 0);",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:")).with_species_codes([
        ("Turdus merula", "eurbla"),
        ("Erithacus rubecula", "eurrob"),
    ])
}

async fn get(state: &AppState, path: &str) -> (StatusCode, String, Option<String>) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let disposition = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        String::from_utf8(bytes.to_vec()).unwrap(),
        disposition,
    )
}

const HEADER: &str = "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tCommon Name\tSpecies Code\tConfidence\tBegin Path\tFile Offset (s)";

#[tokio::test]
async fn the_combined_table_places_every_detection_that_has_a_clip() {
    let state = station();
    let (status, body, disposition) = get(&state, "/api/v2/detections/export/raven").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        disposition.as_deref(),
        Some("attachment; filename=\"detections.raven.txt\"")
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines[0], HEADER,
        "the header must be BirdNET-Analyzer's, column for column"
    );
    assert_eq!(
        lines[1..],
        [
            "1\tSpectrogram 1\t1\t1.500\t4.500\t0\t15000\tEurasian Blackbird\teurbla\t0.9123\ta.wav\t1.500",
            "2\tSpectrogram 1\t1\t4.500\t6.000\t0\t15000\tEuropean Robin\teurrob\t0.8000\ta.wav\t4.500",
            "3\tSpectrogram 1\t1\t0.000\t6.000\t0\t15000\tCommon Cuckoo\tCuculus canorus\t0.7000\tb.wav\t0.000",
        ],
        "two placed selections in a.wav, the whole clip for the pre-migration row in b.wav, \
         and nothing for the rows that cannot be placed in a file:\n{body}"
    );
}

#[tokio::test]
async fn a_bad_date_is_refused() {
    let (status, body, _) = get(&station(), "/api/v2/detections/export/raven?from=yesterday").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn a_clip_has_its_own_table_and_its_own_label_track() {
    let state = station();
    let (status, body, disposition) = get(&state, "/api/v2/recordings/a.wav/raven.txt").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        disposition.as_deref(),
        Some("attachment; filename=\"a.wav.raven.txt\"")
    );
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 3, "header and two selections:\n{body}");
    assert!(
        lines[1].starts_with("1\tSpectrogram 1\t1\t1.500\t4.500\t"),
        "{body}"
    );
    assert!(
        lines[2].starts_with("2\tSpectrogram 1\t1\t4.500\t6.000\t"),
        "{body}"
    );

    let (status, body, _) = get(&state, "/api/v2/recordings/a.wav/labels.txt").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body, "1.500\t4.500\tEurasian Blackbird 91%\n4.500\t6.000\tEuropean Robin 80%\n",
        "an Audacity label track is begin, end, label"
    );

    let (status, _, _) = get(&state, "/api/v2/recordings/nobody.wav/raven.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a clip no detection names");
    let (status, _, _) = get(&state, "/api/v2/recordings/..%2Fa.wav/raven.txt").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a path, not a bare filename"
    );
}
