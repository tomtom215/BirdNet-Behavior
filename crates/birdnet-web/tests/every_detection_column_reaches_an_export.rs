//! Every column of `detections` reaches at least one export, or is named here
//! as internal with a reason (RES-08).
//!
//! The CSV export left out columns a methods section needs: which source heard
//! the detection, the reviewer's verdict, where the chunk sat in its recording,
//! whether the clip's audio was pruned, which import it came from and its
//! `BirdWeather` soundscape. Several were in no export at all. Nothing noticed,
//! because nothing compared the table with what leaves the station; this does,
//! so a column added by a later migration fails here until someone decides
//! whether it is exported.
//!
//! How: one detection is written with a distinct value in every column, the
//! exports are fetched through the router, and each column's stored value must
//! be found as a field of one of them.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use rusqlite::types::Value;
use tower::ServiceExt as _;

/// Columns that are deliberately in no export, each with its reason.
const INTERNAL: &[(&str, &str)] = &[(
    "is_locked",
    "the operator's instruction to the retention jobs to keep the clip; it says \
     nothing about the detection, and whether the audio is still there is \
     exported as Clip_Pruned_At",
)];

/// A distinct, constraint-satisfying value for each column, as SQL literals.
/// A column with no entry here and none in [`INTERNAL`] fails the gate: a new
/// column has to be given one, which is the point at which somebody decides
/// whether it is exported.
const SENTINELS: &[(&str, &str)] = &[
    ("Date", "'2026-04-05'"),
    ("Time", "'06:07:08'"),
    ("Sci_Name", "'Sentinelus sciensis'"),
    ("Com_Name", "'Sentinel Warbler'"),
    ("Confidence", "0.8765"),
    ("Lat", "12.3456"),
    ("Lon", "65.4321"),
    ("Cutoff", "0.4567"),
    ("Week", "37"),
    ("Sens", "1.2345"),
    ("Overlap", "0.375"),
    ("File_Name", "'sentinel-clip.wav'"),
    ("chunk_offset_secs", "4.5"),
    ("is_locked", "1"),
    ("Clip_Pruned_At", "1788123456"),
    ("Source", "'sentinel-source'"),
    ("correlation_id", "'sentinel-correlation'"),
    ("Duration_Secs", "6.25"),
    ("import_batch_id", "4343"),
    ("review_verdict", "'confirmed'"),
    ("detected_at_utc", "1775368028"),
    ("birdweather_soundscape_id", "5454"),
    ("run_id", "4242"),
    ("clip_offset_secs", "1.125"),
    ("detection_secs", "2.875"),
    ("model_id", "'sentinel-model'"),
    ("model_agreement", "3"),
];

fn columns(conn: &rusqlite::Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM pragma_table_info('detections')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The station, the column names and each column's stored value.
fn station() -> (AppState, Vec<(String, Value)>) {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let cols = columns(&conn);
    let unaccounted: Vec<&String> = cols
        .iter()
        .filter(|c| !SENTINELS.iter().any(|(n, _)| n == c))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "detections has columns this gate has no value for: {unaccounted:?}. Give each a \
         sentinel, and export it or name it in INTERNAL with a reason"
    );
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    let (names, values): (Vec<&str>, Vec<&str>) = SENTINELS.iter().copied().unzip();
    conn.execute(
        &format!(
            "INSERT INTO detections ({}) VALUES ({})",
            names.join(", "),
            values.join(", ")
        ),
        [],
    )
    .expect("seed the sentinel row");
    // Read back what was stored: a trigger may have written a column itself.
    let stored: Vec<(String, Value)> = cols
        .iter()
        .map(|c| {
            let v: Value = conn
                .query_row(&format!("SELECT \"{c}\" FROM detections"), [], |r| r.get(0))
                .unwrap();
            (c.clone(), v)
        })
        .collect();
    (
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")),
        stored,
    )
}

async fn body(state: &AppState, uri: &str) -> String {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Every leaf value of a JSON document, as text.
fn json_leaves(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => m.values().for_each(|x| json_leaves(x, out)),
        serde_json::Value::Array(a) => a.iter().for_each(|x| json_leaves(x, out)),
        serde_json::Value::String(s) => out.push(s.clone()),
        other => out.push(other.to_string()),
    }
}

fn matches(field: &str, stored: &Value) -> bool {
    let field = field.trim().trim_matches('"');
    match stored {
        Value::Text(t) => field == t,
        Value::Integer(i) => field == i.to_string(),
        Value::Real(r) => field.parse::<f64>().is_ok_and(|f| (f - r).abs() < 1e-9),
        Value::Null | Value::Blob(_) => false,
    }
}

#[tokio::test]
async fn every_detections_column_is_in_an_export_or_named_internal() {
    let (state, stored) = station();
    let mut fields: Vec<String> = Vec::new();
    for (uri, sep) in [
        ("/api/v2/detections/export?format=csv", ','),
        ("/api/v2/detections/export/birddb", ';'),
        ("/api/v2/detections/export/raven", '\t'),
    ] {
        let text = body(&state, uri).await;
        fields.extend(
            text.lines()
                .flat_map(|l| l.split(sep).map(str::to_owned).collect::<Vec<_>>()),
        );
    }
    let json: serde_json::Value =
        serde_json::from_str(&body(&state, "/api/v2/detections/export?format=json").await).unwrap();
    json_leaves(&json, &mut fields);

    let mut missing = Vec::new();
    for (column, value) in &stored {
        assert!(
            !matches!(value, Value::Null),
            "precondition: {column} was stored as NULL, so it cannot be looked for"
        );
        let internal = INTERNAL.iter().any(|(c, _)| c == column);
        let found = fields.iter().any(|f| matches(f, value));
        if !found && !internal {
            missing.push(column.as_str());
        }
    }
    assert!(
        missing.is_empty(),
        "these detections columns reach no export: {missing:?}"
    );
}

/// The counterpart: the gate can see a column that is exported. Without it a
/// matcher that never matched would leave every column "missing" except the
/// internal ones, and the assertion above would report that loudly; this
/// checks the opposite direction, that an internal column really is absent,
/// so the exemption is a fact and not a leftover.
#[tokio::test]
async fn an_internal_column_is_really_in_no_export() {
    let (state, _) = station();
    let csv = body(&state, "/api/v2/detections/export?format=csv").await;
    let header = csv.lines().next().unwrap();
    for (column, _) in INTERNAL {
        assert!(
            !header.split(',').any(|h| h.eq_ignore_ascii_case(column)),
            "{column} is named internal but the CSV exports it"
        );
    }
    let json = body(&state, "/api/v2/detections/export?format=json").await;
    for (column, _) in INTERNAL {
        assert!(
            !json.contains(&format!("\"{}\"", column.to_lowercase())),
            "{column} is named internal but the JSON exports it"
        );
    }
}
