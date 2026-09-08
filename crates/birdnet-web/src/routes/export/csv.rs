//! CSV and JSON export for detections and species.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use super::{MAX_EXPORT_ROWS, escape_csv, export_too_large};
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
            let rows = birdnet_db::sqlite::analytic_detections(
                conn,
                from.as_deref(),
                to.as_deref(),
                MAX_EXPORT_ROWS,
            )?;
            // The model each row was made with (R-1): joined in memory over
            // the run table, which is one row per daemon start.
            let runs = birdnet_db::sqlite::run_models(conn)?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, runs))
        })
    })
    .await;

    match result {
        Ok(Ok(((detections, truncated), runs))) => {
            if truncated {
                return export_too_large();
            }
            if format == "json" {
                let total = detections.len();
                let detections: Vec<ExportedDetection<'_>> = detections
                    .iter()
                    .map(|row| ExportedDetection::new(row, &runs))
                    .collect();
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
                let csv = detections_to_csv(&detections, &runs);
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
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
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
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": crate::routes::log_internal("internal error", &e)}).to_string(),
        )
            .into_response(),
    }
}

/// A detection row with the model that made it attached (R-1): the row's
/// `run_id` resolved through `analysis_runs` to the model's name and the
/// SHA-256 of its bytes. Both are absent on a row no run of this station
/// produced — imported history, rows older than migration 43.
#[derive(serde::Serialize)]
struct ExportedDetection<'a> {
    #[serde(flatten)]
    row: &'a birdnet_db::sqlite::DetectionRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_sha256: Option<&'a str>,
}

impl<'a> ExportedDetection<'a> {
    fn new(
        row: &'a birdnet_db::sqlite::DetectionRow,
        runs: &'a std::collections::HashMap<i64, birdnet_db::sqlite::RunModel>,
    ) -> Self {
        let model = row.run_id.and_then(|id| runs.get(&id));
        Self {
            row,
            model_name: model.map(|m| m.model_name.as_str()),
            model_sha256: model.map(|m| m.model_sha256.as_str()),
        }
    }
}

/// Convert detection rows to CSV format.
///
/// The twelve BirdNET-Pi columns first, in BirdNET-Pi's order, then the
/// run's identity: `Run_Id`, `Model_Name`, `Model_SHA256`. Empty on a row no
/// run of this station produced.
fn detections_to_csv(
    rows: &[birdnet_db::sqlite::DetectionRow],
    runs: &std::collections::HashMap<i64, birdnet_db::sqlite::RunModel>,
) -> String {
    let mut csv = String::with_capacity(rows.len() * 200);
    csv.push_str(
        "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,\
         Run_Id,Model_Name,Model_SHA256\n",
    );

    for row in rows {
        let model = row.run_id.and_then(|id| runs.get(&id));
        let _ = writeln!(
            csv,
            "{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{}",
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
            row.run_id.map_or(String::new(), |v| v.to_string()),
            model.map_or(String::new(), |m| escape_csv(&m.model_name)),
            model.map_or(String::new(), |m| escape_csv(&m.model_sha256)),
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
        let csv = detections_to_csv(&[], &std::collections::HashMap::new());
        assert!(csv.starts_with("Date,Time,Sci_Name,Com_Name,Confidence"));
        assert_eq!(
            csv.trim_end(),
            "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,\
             Run_Id,Model_Name,Model_SHA256"
        );
    }

    /// R-1: the CSV names the model that made each row, and leaves the three
    /// columns empty on a row no run produced rather than inventing one.
    #[test]
    fn detections_csv_carries_the_model_of_each_rows_run() {
        let mut runs = std::collections::HashMap::new();
        runs.insert(
            7,
            birdnet_db::sqlite::RunModel {
                model_name: "BirdNET+_V3.0-preview3_Global_11K_FP32".into(),
                model_sha256: "2a0f9efb".into(),
            },
        );
        let made_by_run = birdnet_db::sqlite::DetectionRow {
            date: "2026-03-12".into(),
            time: "06:30:00".into(),
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            run_id: Some(7),
            ..Default::default()
        };
        let imported = birdnet_db::sqlite::DetectionRow {
            date: "2024-03-12".into(),
            time: "06:30:00".into(),
            sci_name: "Pica pica".into(),
            com_name: "Eurasian Magpie".into(),
            confidence: 0.80,
            run_id: None,
            ..Default::default()
        };
        let csv = detections_to_csv(&[made_by_run, imported], &runs);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(
            lines[1].ends_with(",7,BirdNET+_V3.0-preview3_Global_11K_FP32,2a0f9efb"),
            "{}",
            lines[1]
        );
        assert!(lines[2].ends_with(",,,"), "{}", lines[2]);
        assert_eq!(lines[2].matches(',').count(), lines[1].matches(',').count());
    }

    /// The JSON export attaches the same two fields per row, and omits them
    /// on a row no run produced.
    #[test]
    fn exported_json_carries_the_model_of_each_rows_run() {
        let mut runs = std::collections::HashMap::new();
        runs.insert(
            3,
            birdnet_db::sqlite::RunModel {
                model_name: "m".into(),
                model_sha256: "abc".into(),
            },
        );
        let row = birdnet_db::sqlite::DetectionRow {
            sci_name: "Pica pica".into(),
            run_id: Some(3),
            ..Default::default()
        };
        let v = serde_json::to_value(ExportedDetection::new(&row, &runs)).unwrap();
        assert_eq!(v["sci_name"], "Pica pica");
        assert_eq!(v["run_id"], 3);
        assert_eq!(v["model_name"], "m");
        assert_eq!(v["model_sha256"], "abc");
        let orphan = birdnet_db::sqlite::DetectionRow::default();
        let v = serde_json::to_value(ExportedDetection::new(&orphan, &runs)).unwrap();
        assert!(v.get("model_name").is_none(), "{v}");
        assert!(v.get("run_id").is_none(), "{v}");
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
            correlation_id: None,
            source: None,
            ..Default::default()
        };
        let csv = detections_to_csv(&[row], &std::collections::HashMap::new());
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
