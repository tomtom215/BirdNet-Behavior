//! CSV and JSON export for detections and species.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use super::escape_csv;
use crate::routes::is_valid_date;
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct ExportQuery {
    /// Output format: "csv" or "json" (default: "csv").
    format: Option<String>,
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

pub(super) async fn export_detections(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> impl IntoResponse {
    let format = query.format.as_deref().unwrap_or("csv");

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
            if format == "json" {
                let total = detections.len();
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    serde_json::to_string(&json!({
                        "detections": detections,
                        "total": total,
                    }))
                    .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.into()),
                )
                    .into_response()
            } else {
                let csv = detections_to_csv(&detections);
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=\"detections.csv\"",
                        ),
                    ],
                    csv,
                )
                    .into_response()
            }
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

#[derive(Deserialize)]
pub(super) struct SpeciesExportQuery {
    /// Output format: "csv" or "json" (default: "csv").
    format: Option<String>,
    /// Maximum number of species to export (default: all).
    limit: Option<u32>,
}

pub(super) async fn export_species(
    State(state): State<AppState>,
    Query(query): Query<SpeciesExportQuery>,
) -> impl IntoResponse {
    let format = query.format.as_deref().unwrap_or("csv");
    let limit = query.limit.unwrap_or(100_000);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, limit))
    })
    .await;

    match result {
        Ok(Ok(species)) => {
            if format == "json" {
                let total = species.len();
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    serde_json::to_string(&json!({
                        "species": species,
                        "total": total,
                    }))
                    .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.into()),
                )
                    .into_response()
            } else {
                let csv = species_to_csv(&species);
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=\"species.csv\"",
                        ),
                    ],
                    csv,
                )
                    .into_response()
            }
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

/// Convert detection rows to CSV format.
fn detections_to_csv(rows: &[birdnet_db::sqlite::DetectionRow]) -> String {
    let mut csv = String::with_capacity(rows.len() * 120);
    csv.push_str(
        "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name\n",
    );

    for row in rows {
        let _ = writeln!(
            csv,
            "{},{},{},{},{:.4},{},{},{},{},{},{},{}",
            escape_csv(&row.date),
            escape_csv(&row.time),
            escape_csv(&row.sci_name),
            escape_csv(&row.com_name),
            row.confidence,
            row.lat.map_or(String::new(), |v| v.to_string()),
            row.lon.map_or(String::new(), |v| v.to_string()),
            row.cutoff.map_or(String::new(), |v| v.to_string()),
            row.week.map_or(String::new(), |v| v.to_string()),
            row.sens.map_or(String::new(), |v| v.to_string()),
            row.overlap.map_or(String::new(), |v| v.to_string()),
            row.file_name.as_deref().map_or(String::new(), escape_csv),
        );
    }

    csv
}

/// Convert species counts to CSV format.
fn species_to_csv(species: &[birdnet_db::sqlite::SpeciesCount]) -> String {
    let mut csv = String::with_capacity(species.len() * 80);
    csv.push_str("Com_Name,Sci_Name,Count,Avg_Confidence\n");

    for s in species {
        let _ = writeln!(
            csv,
            "{},{},{},{:.4}",
            escape_csv(&s.com_name),
            escape_csv(&s.sci_name),
            s.count,
            s.avg_confidence,
        );
    }

    csv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detections_csv_header() {
        let csv = detections_to_csv(&[]);
        assert!(csv.starts_with("Date,Time,Sci_Name,Com_Name,Confidence"));
    }

    #[test]
    fn detections_csv_row() {
        let row = birdnet_db::sqlite::DetectionRow {
            date: "2026-03-12".into(),
            time: "06:30:00".into(),
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(11),
            sens: None,
            overlap: None,
            file_name: Some("test.wav".into()),
        };
        let csv = detections_to_csv(&[row]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Turdus merula"));
        assert!(lines[1].contains("0.8700"));
        assert!(lines[1].contains("test.wav"));
    }

    #[test]
    fn species_csv_header() {
        let csv = species_to_csv(&[]);
        assert!(csv.starts_with("Com_Name,Sci_Name,Count,Avg_Confidence"));
    }

    #[test]
    fn species_csv_row() {
        let species = birdnet_db::sqlite::SpeciesCount {
            com_name: "Great Tit".into(),
            sci_name: "Parus major".into(),
            count: 42,
            avg_confidence: 0.85,
        };
        let csv = species_to_csv(&[species]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("Great Tit"));
        assert!(lines[1].contains("42"));
    }
}
