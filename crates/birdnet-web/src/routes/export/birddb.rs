//! BirdDB.txt legacy flat-file export.
//!
//! Many external tools (Gravel, `BirdDB` viewers, custom scripts) consume this format.
//! Each line: `Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name`

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::routes::is_valid_date;
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct BirdDbQuery {
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

pub(super) async fn export_birddb(
    State(state): State<AppState>,
    Query(query): Query<BirdDbQuery>,
) -> impl IntoResponse {
    for date in [&query.from, &query.to].into_iter().flatten() {
        if !is_valid_date(date) {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": "invalid date format, expected YYYY-MM-DD"}).to_string(),
            )
                .into_response();
        }
    }

    let from = query.from.clone();
    let to = query.to.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::all_detections(conn, from.as_deref(), to.as_deref())
        })
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            let birddb = detections_to_birddb(&detections);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"BirdDB.txt\"",
                    ),
                ],
                birddb,
            )
                .into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": e.to_string()}).to_string(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": format!("internal error: {e}")}).to_string(),
        )
            .into_response(),
    }
}

/// Convert detection rows to BirdNET-Pi's BirdDB.txt semicolon-delimited format.
fn detections_to_birddb(rows: &[birdnet_db::sqlite::DetectionRow]) -> String {
    let mut out = String::with_capacity(rows.len() * 100);

    for row in rows {
        let _ = writeln!(
            out,
            "{};{};{};{};{:.4};{};{};{};{};{};{};{}",
            row.date,
            row.time,
            row.sci_name,
            row.com_name,
            row.confidence,
            row.lat.map_or(String::new(), |v| v.to_string()),
            row.lon.map_or(String::new(), |v| v.to_string()),
            row.cutoff.map_or(String::new(), |v| v.to_string()),
            row.week.map_or(String::new(), |v| v.to_string()),
            row.sens.map_or(String::new(), |v| v.to_string()),
            row.overlap.map_or(String::new(), |v| v.to_string()),
            row.file_name.as_deref().unwrap_or(""),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birddb_format_semicolons() {
        let row = birdnet_db::sqlite::DetectionRow {
            date: "2026-03-12".into(),
            time: "06:30:00".into(),
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            lat: Some(51.5),
            lon: Some(-0.12),
            cutoff: None,
            week: Some(11),
            sens: None,
            overlap: None,
            file_name: Some("test.wav".into()),
        };
        let out = detections_to_birddb(&[row]);
        let line = out.lines().next().unwrap();
        assert!(
            line.contains(';'),
            "BirdDB.txt should use semicolon delimiters"
        );
        let parts: Vec<&str> = line.split(';').collect();
        assert_eq!(parts.len(), 12, "BirdDB.txt should have 12 fields");
        assert_eq!(parts[0], "2026-03-12");
        assert_eq!(parts[2], "Turdus merula");
        assert_eq!(parts[11], "test.wav");
    }
}
