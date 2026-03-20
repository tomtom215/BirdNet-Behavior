//! Export endpoints for bulk data download.
//!
//! Provides CSV and JSON export of detection data, compatible with the
//! original BirdNET-Pi `BirdDB.txt` CSV format.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use super::is_valid_date;
use crate::state::AppState;

/// Export routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections/export", get(export_detections))
        .route("/species/export", get(export_species))
        .route("/detections/export/ebird", get(export_ebird))
        .route("/detections/export/birddb", get(export_birddb))
}

#[derive(Deserialize)]
struct ExportQuery {
    /// Output format: "csv" or "json" (default: "csv").
    format: Option<String>,
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

async fn export_detections(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> impl IntoResponse {
    let format = query.format.as_deref().unwrap_or("csv");

    // Validate date parameters if provided
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
struct SpeciesExportQuery {
    /// Output format: "csv" or "json" (default: "csv").
    format: Option<String>,
    /// Maximum number of species to export (default: all).
    limit: Option<u32>,
}

async fn export_species(
    State(state): State<AppState>,
    Query(query): Query<SpeciesExportQuery>,
) -> impl IntoResponse {
    let format = query.format.as_deref().unwrap_or("csv");
    // Use a very large limit to effectively return all species when none specified
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

// ---------------------------------------------------------------------------
// BirdDB.txt legacy flat-file export
// ---------------------------------------------------------------------------

/// Export detections in BirdNET-Pi's legacy `BirdDB.txt` semicolon-delimited format.
///
/// Many external tools (Gravel, BirdDB viewers, custom scripts) consume this format.
/// Each line: `Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name`
#[derive(Deserialize)]
struct BirdDbQuery {
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

async fn export_birddb(
    State(state): State<AppState>,
    Query(query): Query<BirdDbQuery>,
) -> impl IntoResponse {
    // Validate date parameters if provided
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
///
/// Format per line: `Date;Time;Sci_Name;Com_Name;Confidence;Lat;Lon;Cutoff;Week;Sens;Overlap;File_Name`
/// This matches the format used by `birdnet_recording.sh` append mode in BirdNET-Pi.
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

/// Convert detection rows to CSV format.
///
/// Uses the same column order as the original BirdNET-Pi `BirdDB.txt`.
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

// ---------------------------------------------------------------------------
// eBird CSV export
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EbirdQuery {
    /// Date to export (YYYY-MM-DD). Defaults to today.
    date: Option<String>,
    /// Station latitude.
    lat: Option<f64>,
    /// Station longitude.
    lon: Option<f64>,
    /// Location name.
    location: Option<String>,
}

async fn export_ebird(
    State(state): State<AppState>,
    Query(query): Query<EbirdQuery>,
) -> impl IntoResponse {
    let date = query.date.clone();
    let date_for_filename = date.clone();

    // Validate date if provided
    if let Some(ref d) = date {
        if !is_valid_date(d) {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"error": "invalid date format, expected YYYY-MM-DD"}).to_string(),
            )
                .into_response();
        }
    }

    let lat = query.lat.unwrap_or(0.0);
    let lon = query.lon.unwrap_or(0.0);
    let location = query.location.clone().unwrap_or_default();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let from = date.as_deref();
            let to = date.as_deref();
            birdnet_db::sqlite::all_detections(conn, from, to)
        })
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            let csv = detections_to_ebird_csv(&detections, lat, lon, &location);
            let filename = date_for_filename.as_deref().unwrap_or("all");
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"ebird_export_{filename}.csv\""),
                    ),
                ],
                csv,
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

/// Build an eBird-compatible CSV from detection rows.
///
/// eBird format: Common Name, Genus, Species, Number, Species Comments,
/// Location Name, Latitude, Longitude, Date, Start Time, State/Province,
/// Country Code, Protocol, Number of Observers, Duration (Min),
/// All Species Reported, Effort Distance (km), Effort Area (ha),
/// Submission Comments
fn detections_to_ebird_csv(
    rows: &[birdnet_db::sqlite::DetectionRow],
    lat: f64,
    lon: f64,
    location: &str,
) -> String {
    let mut csv = String::with_capacity(rows.len() * 200);
    csv.push_str("Common Name,Genus,Species,Number,Species Comments,Location Name,Latitude,Longitude,Date,Start Time,State/Province,Country Code,Protocol,Number of Observers,Duration (Min),All Species Reported,Effort Distance (km),Effort Area (ha),Submission Comments\n");

    // Group by species to get counts per species per date
    let mut species_groups: std::collections::HashMap<
        String,
        Vec<&birdnet_db::sqlite::DetectionRow>,
    > = std::collections::HashMap::new();
    for row in rows {
        species_groups
            .entry(format!("{}|{}", row.com_name, row.date))
            .or_default()
            .push(row);
    }

    for (_, group) in &species_groups {
        let first = group[0];
        let count = group.len();
        // Parse sci_name into genus and species
        let (genus, species) = first
            .sci_name
            .split_once(' ')
            .unwrap_or((&first.sci_name, "sp."));

        // Find earliest time
        let start_time = group
            .iter()
            .map(|d| d.time.as_str())
            .min()
            .unwrap_or(&first.time);

        // Average confidence as comment
        let avg_conf: f64 = group.iter().map(|d| d.confidence).sum::<f64>() / count as f64;
        let comment = format!("BirdNET avg confidence: {:.0}%", avg_conf * 100.0);

        // Convert date from YYYY-MM-DD to MM/DD/YYYY for eBird
        let ebird_date = if first.date.len() == 10 {
            format!(
                "{}/{}/{}",
                &first.date[5..7],
                &first.date[8..10],
                &first.date[..4]
            )
        } else {
            first.date.clone()
        };

        let loc = if location.is_empty() {
            "BirdNet-Behavior Station"
        } else {
            location
        };

        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{},{},{},{},,,S,1,,,,,",
            escape_csv(&first.com_name),
            escape_csv(genus),
            escape_csv(species),
            count,
            escape_csv(&comment),
            escape_csv(loc),
            lat,
            lon,
            ebird_date,
            escape_csv(start_time),
        );
    }

    csv
}

/// Escape a value for CSV output (RFC 4180).
///
/// Wraps the value in double quotes if it contains commas, quotes, or newlines.
fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_csv_plain_text() {
        assert_eq!(escape_csv("hello"), "hello");
    }

    #[test]
    fn escape_csv_with_comma() {
        assert_eq!(escape_csv("hello, world"), "\"hello, world\"");
    }

    #[test]
    fn escape_csv_with_quotes() {
        assert_eq!(escape_csv("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

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
