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

    let permit = state.heavy_read_permit().await;

    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        state.with_read_db(|conn| {
            let rows = birdnet_db::sqlite::analytic_detections(
                conn,
                from.as_deref(),
                to.as_deref(),
                MAX_EXPORT_ROWS,
            )?;
            // The model each row was made with (R-1): joined in memory over
            // the run table, which is one row per daemon start.
            let runs = birdnet_db::sqlite::model_attribution(conn)?;
            // Where, with what and under which clock each row was recorded
            // (RES-12, RES-22, MSC-13): read once, attributed in memory.
            let provenance = RowProvenance::load(conn)?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, (runs, provenance)))
        })
    })
    .await;

    match result {
        Ok(Ok(((detections, truncated), (runs, provenance)))) => {
            if truncated {
                return export_too_large();
            }
            if format == "json" {
                let total = detections.len();
                let detections: Vec<ExportedDetection<'_>> = detections
                    .iter()
                    .map(|row| ExportedDetection::new(row, &runs, &provenance))
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
                let csv = detections_to_csv_with(&detections, &runs, &provenance);
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

    let permit = state.heavy_read_permit().await;

    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        state.with_read_db(|conn| birdnet_db::sqlite::top_species(conn, limit))
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
///
/// It also carries the row's instant in both readings (R-8): `event_date`,
/// the local wall clock with the offset that was in force, RFC 3339; and
/// `detected_at_utc` from the row itself. `date`/`time` are the local wall
/// clock and say nothing about the offset; these two do.
#[derive(serde::Serialize)]
struct ExportedDetection<'a> {
    #[serde(flatten)]
    row: &'a birdnet_db::sqlite::DetectionRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_sha256: Option<&'a str>,
    #[serde(flatten)]
    recorded: Recorded<'a>,
}

/// Where, with what and under which clock a row was recorded.
#[derive(serde::Serialize, Default)]
struct Recorded<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    station_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployment_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_device: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_state: Option<&'static str>,
}

/// The station's identity, deployments, device records and clock record, for
/// attributing exported rows.
#[derive(Default)]
pub(super) struct RowProvenance {
    sources: birdnet_db::station::SourceProvenance,
    clock: birdnet_db::capture_ledger::ClockHistory,
}

impl RowProvenance {
    pub(super) fn load(conn: &rusqlite::Connection) -> Result<Self, birdnet_db::sqlite::DbError> {
        let sources = birdnet_db::station::SourceProvenance::load(conn).map_err(|e| match e {
            birdnet_db::station::StationError::Sqlite(e) => birdnet_db::sqlite::DbError::Sqlite(e),
            birdnet_db::station::StationError::Invalid(m) => birdnet_db::sqlite::DbError::Schema(m),
        })?;
        Ok(Self {
            sources,
            clock: birdnet_db::capture_ledger::ClockHistory::load(conn)?,
        })
    }

    /// The row's attribution. A row imported from elsewhere was timed by
    /// another station's clock and heard by another station's device, so it
    /// is given no deployment, device or clock state of this station's; a row
    /// with no instant or no source is given only what does not need one.
    fn of<'a>(&'a self, row: &birdnet_db::sqlite::DetectionRow) -> Recorded<'a> {
        let station_id = self.sources.station_uuid.as_deref();
        if row.import_batch_id.is_some() {
            return Recorded {
                station_id: None,
                ..Recorded::default()
            };
        }
        let Some(at) = row.detected_at_utc else {
            return Recorded {
                station_id,
                ..Recorded::default()
            };
        };
        let source = row.source.as_deref();
        Recorded {
            station_id,
            deployment_id: source
                .and_then(|s| self.sources.deployment_at(s, at))
                .map(|d| d.id),
            source_device: source
                .and_then(|s| self.sources.device_at(s, at))
                .map(|d| d.device_seq),
            clock_state: Some(self.clock.state_at(at).as_str()),
        }
    }
}

impl<'a> ExportedDetection<'a> {
    fn new(
        row: &'a birdnet_db::sqlite::DetectionRow,
        runs: &'a birdnet_db::sqlite::ModelAttribution,
        provenance: &'a RowProvenance,
    ) -> Self {
        let model = runs.for_row(row.run_id, row.model_id.as_deref());
        Self {
            row,
            event_date: event_date_of(row),
            model_name: model.map(|m| m.model_name.as_str()),
            model_sha256: model.map(|m| m.model_sha256.as_str()),
            recorded: provenance.of(row),
        }
    }
}

/// The row's local wall clock with its offset, RFC 3339 — blank when the row
/// has no instant, never guessed.
fn event_date_of(row: &birdnet_db::sqlite::DetectionRow) -> Option<String> {
    birdnet_core::civil::rfc3339_local(&row.date, &row.time, row.detected_at_utc)
}

/// Convert detection rows to CSV format.
///
/// The twelve BirdNET-Pi columns first, in BirdNET-Pi's order — `Date` and
/// `Time` are the station's local wall clock with no offset, as BirdNET-Pi
/// wrote them — then the instant in two readings (`Event_Date`, the same wall
/// clock with the offset that was in force, RFC 3339; `Detected_At_UTC`, the
/// instant in UTC), then the run's identity: `Run_Id`, `Model_Name`,
/// `Model_SHA256`, then the classifier behind the row: `Model_Id` (the
/// classifier whose answer the row carries) and `Model_Agreement` (how many
/// classifiers cleared their own bar). The instant columns are empty on a row
/// that names no point in time; the run columns on a row no run of this
/// station produced; the classifier columns on a row written before they were
/// recorded, where empty means nobody was counting and `1` means one
/// classifier was asked and one answered. Each classifier's own score is in the
/// separate model-scores export.
///
/// Then, appended so no earlier column moves (RES-08): `Source` (the input
/// that heard it), `Review_Verdict` (`confirmed`, or empty for unreviewed:
/// rejected rows are not exported), `Chunk_Offset_Secs` (where the analysed
/// chunk began in its recording, part of the detection's key),
/// `Clip_Pruned_At` (when retention removed the clip's audio, RFC 3339 UTC;
/// empty while it is kept), `Import_Batch_Id` (the import it came from; empty
/// for a row this station recorded) and `BirdWeather_Soundscape_Id`.
///
/// Then, appended after those (RES-25, RES-23): `Detection_Id`, the
/// detection's permanent identifier, which a relabel or a `VACUUM` does not
/// change and which the CSV import keeps when the file is read back; and
/// `Clip_SHA256` and `Clip_Bytes`, the clip's checksum and length as it was
/// written, empty where the station did not record one (a row from before it
/// did, or whose clip could not be read when it was written): unknown, not
/// computed from the file there now.
///
/// Then, appended likewise (RES-12, RES-22, MSC-13): `Station_Id` (this
/// station's UUID), `Deployment_Id` (the operator-stated deployment of the
/// row's source in force at its instant; empty when none was stated),
/// `Source_Device` (which of the source's devices it was, numbered from 1;
/// the same device string keeps its number) and `Clock_State` (`synced`,
/// `unsynced`, `unknown` or `unset`: what the system said of its clock when
/// the row's instant was stamped; `unknown` before the station recorded its
/// clock). All four are empty on an imported row, which another station
/// recorded and timed.
#[cfg(test)]
fn detections_to_csv(
    rows: &[birdnet_db::sqlite::DetectionRow],
    runs: &birdnet_db::sqlite::ModelAttribution,
) -> String {
    detections_to_csv_with(rows, runs, &RowProvenance::default())
}

/// The detections CSV with each row's attribution: the detections export's
/// and the search export's (UIC-10), so the two carry the same columns.
pub(super) fn detections_to_csv_with(
    rows: &[birdnet_db::sqlite::DetectionRow],
    runs: &birdnet_db::sqlite::ModelAttribution,
    provenance: &RowProvenance,
) -> String {
    let mut csv = String::with_capacity(rows.len() * 240);
    csv.push_str(
        "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,\
         Event_Date,Detected_At_UTC,Run_Id,Model_Name,Model_SHA256,Model_Id,Model_Agreement,\
         Source,Review_Verdict,Chunk_Offset_Secs,Clip_Pruned_At,Import_Batch_Id,\
         BirdWeather_Soundscape_Id,Detection_Id,Clip_SHA256,Clip_Bytes,Station_Id,Deployment_Id,Source_Device,Clock_State\n",
    );

    for row in rows {
        let model = runs.for_row(row.run_id, row.model_id.as_deref());
        let recorded = provenance.of(row);
        let _ = writeln!(
            csv,
            "{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
            event_date_of(row).unwrap_or_default(),
            row.detected_at_utc
                .map_or(String::new(), birdnet_core::civil::rfc3339_utc),
            row.run_id.map_or(String::new(), |v| v.to_string()),
            model.map_or(String::new(), |m| escape_csv(&m.model_name)),
            model.map_or(String::new(), |m| escape_csv(&m.model_sha256)),
            row.model_id.as_deref().map_or(String::new(), escape_csv),
            row.model_agreement.map_or(String::new(), |n| n.to_string()),
            row.source.as_deref().map_or(String::new(), escape_csv),
            row.review_verdict
                .as_deref()
                .map_or(String::new(), escape_csv),
            row.chunk_offset_secs
                .map_or(String::new(), |v| v.to_string()),
            row.clip_pruned_at
                .map_or(String::new(), birdnet_core::civil::rfc3339_utc),
            row.import_batch_id.map_or(String::new(), |v| v.to_string()),
            row.birdweather_soundscape_id
                .map_or(String::new(), |v| v.to_string()),
            row.detection_uuid
                .as_deref()
                .map_or(String::new(), escape_csv),
            row.clip_sha256.as_deref().map_or(String::new(), escape_csv),
            row.clip_bytes.map_or(String::new(), |v| v.to_string()),
            recorded.station_id.map_or(String::new(), escape_csv),
            recorded
                .deployment_id
                .map_or(String::new(), |v| v.to_string()),
            recorded
                .source_device
                .map_or(String::new(), |v| v.to_string()),
            recorded.clock_state.unwrap_or_default(),
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
        let csv = detections_to_csv(&[], &birdnet_db::sqlite::ModelAttribution::default());
        assert!(csv.starts_with("Date,Time,Sci_Name,Com_Name,Confidence"));
        assert_eq!(
            csv.trim_end(),
            "Date,Time,Sci_Name,Com_Name,Confidence,Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,\
             Event_Date,Detected_At_UTC,Run_Id,Model_Name,Model_SHA256,Model_Id,Model_Agreement,\
             Source,Review_Verdict,Chunk_Offset_Secs,Clip_Pruned_At,Import_Batch_Id,\
             BirdWeather_Soundscape_Id,Detection_Id,Clip_SHA256,Clip_Bytes,Station_Id,Deployment_Id,Source_Device,Clock_State"
        );
    }

    /// RES-08: the columns a methods section needs are appended, and the
    /// nineteen a consumer already reads keep their positions.
    #[test]
    fn the_appended_columns_carry_the_row_and_move_nothing() {
        let row = birdnet_db::sqlite::DetectionRow {
            date: "2026-04-05".into(),
            time: "06:07:08".into(),
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            confidence: 0.9,
            file_name: Some("a.wav".into()),
            source: Some("garden".into()),
            review_verdict: Some("confirmed".into()),
            chunk_offset_secs: Some(4.5),
            clip_pruned_at: Some(0),
            import_batch_id: Some(12),
            birdweather_soundscape_id: Some(34),
            detection_uuid: Some("6f1c2a52-0e4b-4c1e-9a77-0d6f3b8e2c11".into()),
            clip_sha256: Some("ab".repeat(32)),
            clip_bytes: Some(4321),
            ..Default::default()
        };
        let csv = detections_to_csv(&[row], &birdnet_db::sqlite::ModelAttribution::default());
        let mut lines = csv.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        let cells: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(header.len(), cells.len());
        assert_eq!(header[11], "File_Name");
        assert_eq!(cells[11], "a.wav");
        assert_eq!(header[18], "Model_Agreement");
        let tail: Vec<(&str, &str)> = header[19..]
            .iter()
            .copied()
            .zip(cells[19..].iter().copied())
            .collect();
        assert_eq!(
            tail,
            [
                ("Source", "garden"),
                ("Review_Verdict", "confirmed"),
                ("Chunk_Offset_Secs", "4.5"),
                ("Clip_Pruned_At", "1970-01-01T00:00:00Z"),
                ("Import_Batch_Id", "12"),
                ("BirdWeather_Soundscape_Id", "34"),
                ("Detection_Id", "6f1c2a52-0e4b-4c1e-9a77-0d6f3b8e2c11"),
                ("Clip_SHA256", "ab".repeat(32).as_str()),
                ("Clip_Bytes", "4321"),
                ("Station_Id", ""),
                ("Deployment_Id", ""),
                ("Source_Device", ""),
                ("Clock_State", ""),
            ]
        );
    }

    /// RES-12 / RES-22 / MSC-13: each row names the station, the deployment
    /// and device of its source at its instant, and the clock state it was
    /// stamped under; an imported row names none of this station's.
    #[test]
    fn each_row_names_its_station_deployment_device_and_clock() {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.insert(&NewAudioSource::defaults(
            "mic",
            SourceKind::UsbAlsa,
            "plughw:1,0",
        ))
        .unwrap();
        let d = birdnet_db::station::open_deployment(
            &conn,
            &birdnet_db::station::NewDeployment {
                source_id: "mic".into(),
                start_utc: 1_000,
                stated: birdnet_db::station::DeploymentFields::default(),
            },
            1_000,
        )
        .unwrap();
        conn.execute_batch(
            "UPDATE audio_source_devices SET from_utc = NULL;
             INSERT INTO clock_observations (at_utc, state, authority) VALUES (1500, 'unsynced', 'timedatectl'),
                                                                              (3000, 'synced', 'timedatectl');",
        )
        .unwrap();
        let provenance = RowProvenance::load(&conn).unwrap();
        let station = birdnet_db::station::station_uuid(&conn).unwrap();
        let row = |at: i64, import: Option<i64>| birdnet_db::sqlite::DetectionRow {
            date: "2026-04-05".into(),
            time: "06:07:08".into(),
            sci_name: "Turdus merula".into(),
            com_name: "Eurasian Blackbird".into(),
            confidence: 0.9,
            source: Some("mic".into()),
            detected_at_utc: Some(at),
            import_batch_id: import,
            ..Default::default()
        };
        let csv = detections_to_csv_with(
            &[
                row(2_000, None),
                row(3_500, None),
                row(900, None),
                row(2_000, Some(4)),
            ],
            &birdnet_db::sqlite::ModelAttribution::default(),
            &provenance,
        );
        let tails: Vec<Vec<String>> = csv
            .lines()
            .skip(1)
            .map(|l| {
                l.rsplitn(5, ',')
                    .take(4)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|mut v| {
                v.reverse();
                v
            })
            .collect();
        let id = d.id.to_string();
        assert_eq!(tails[0], [station.as_str(), id.as_str(), "1", "unsynced"]);
        assert_eq!(tails[1], [station.as_str(), id.as_str(), "1", "synced"]);
        assert_eq!(
            tails[2],
            [station.as_str(), "", "1", "unknown"],
            "before the deployment and before the clock was recorded"
        );
        assert_eq!(
            tails[3],
            ["", "", "", ""],
            "an imported row is not this station's"
        );
    }

    /// R-8: the export says which clock its columns are on. `Date`/`Time`
    /// stay BirdNET-Pi's bare local wall clock; `Event_Date` carries the
    /// offset that was in force and `Detected_At_UTC` the instant, so the two
    /// passes of a repeated autumn hour export as different rows.
    #[test]
    fn detections_csv_and_json_carry_the_instant_with_its_offset() {
        // London, 2026-10-25 01:30 local: 00:30Z under BST, 01:30Z under GMT.
        let bst =
            birdnet_core::civil::unix_secs_from_local("2026-10-25", "01:30:00", 3600).unwrap();
        let first = birdnet_db::sqlite::DetectionRow {
            date: "2026-10-25".into(),
            time: "01:30:00".into(),
            sci_name: "Pica pica".into(),
            com_name: "Eurasian Magpie".into(),
            confidence: 0.9,
            detected_at_utc: Some(bst),
            ..Default::default()
        };
        let second = birdnet_db::sqlite::DetectionRow {
            detected_at_utc: Some(bst + 3600),
            ..first.clone()
        };
        let unplaceable = birdnet_db::sqlite::DetectionRow {
            date: "2026-03-29".into(),
            time: "01:30:00".into(),
            detected_at_utc: None,
            ..first.clone()
        };
        let runs = std::collections::HashMap::new();
        let csv = detections_to_csv(
            &[first.clone(), second, unplaceable.clone()],
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
        );
        let lines: Vec<&str> = csv.lines().collect();
        assert!(
            lines[1].contains(",2026-10-25T01:30:00+01:00,2026-10-25T00:30:00Z,"),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].contains(",2026-10-25T01:30:00+00:00,2026-10-25T01:30:00Z,"),
            "{}",
            lines[2]
        );
        assert!(
            lines[3].ends_with(",,,,,,,,"),
            "unplaceable row: {}",
            lines[3]
        );
        assert_eq!(lines[3].matches(',').count(), lines[1].matches(',').count());

        let v = serde_json::to_value(ExportedDetection::new(
            &first,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
            &RowProvenance::default(),
        ))
        .unwrap();
        assert_eq!(v["event_date"], "2026-10-25T01:30:00+01:00");
        assert_eq!(v["detected_at_utc"], bst);
        let v = serde_json::to_value(ExportedDetection::new(
            &unplaceable,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs),
            &RowProvenance::default(),
        ))
        .unwrap();
        assert!(v.get("event_date").is_none(), "{v}");
        assert!(v.get("detected_at_utc").is_none(), "{v}");
    }

    /// The classifier behind a row is exported, and a row from before it was
    /// recorded exports empty cells, not a `1` and not a `0`.
    #[test]
    fn detections_csv_and_json_carry_the_classifier_and_its_agreement() {
        let two = birdnet_db::sqlite::DetectionRow {
            date: "2026-11-01".into(),
            time: "06:30:00".into(),
            sci_name: "Pica pica".into(),
            com_name: "Eurasian Magpie".into(),
            confidence: 0.9,
            model_id: Some("perch,v2".into()),
            model_agreement: Some(2),
            ..Default::default()
        };
        let legacy = birdnet_db::sqlite::DetectionRow {
            model_id: None,
            model_agreement: None,
            ..two.clone()
        };
        let runs = std::collections::HashMap::new();
        let csv = detections_to_csv(
            &[two.clone(), legacy.clone()],
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
        );
        let lines: Vec<&str> = csv.lines().collect();
        assert!(
            lines[1].ends_with(",\"perch,v2\",2,,,,,,,,,,,,,"),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].ends_with(",,"),
            "a legacy row leaves both empty: {}",
            lines[2]
        );
        let v = serde_json::to_value(ExportedDetection::new(
            &two,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
            &RowProvenance::default(),
        ))
        .unwrap();
        assert_eq!(v["model_id"], "perch,v2");
        assert_eq!(v["model_agreement"], 2);
        let v = serde_json::to_value(ExportedDetection::new(
            &legacy,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs),
            &RowProvenance::default(),
        ))
        .unwrap();
        assert!(
            v.get("model_id").is_none() && v.get("model_agreement").is_none(),
            "{v}"
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
        let csv = detections_to_csv(
            &[made_by_run, imported],
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
        );
        let lines: Vec<&str> = csv.lines().collect();
        // The run's three columns are followed by `Model_Id,Model_Agreement`,
        // empty here because neither row names a classifier.
        assert!(
            lines[1].ends_with(",7,BirdNET+_V3.0-preview3_Global_11K_FP32,2a0f9efb,,,,,,,,,,,,,,,"),
            "{}",
            lines[1]
        );
        assert!(lines[2].ends_with(",,,,,,,"), "{}", lines[2]);
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
        let v = serde_json::to_value(ExportedDetection::new(
            &row,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
            &RowProvenance::default(),
        ))
        .unwrap();
        assert_eq!(v["sci_name"], "Pica pica");
        assert_eq!(v["run_id"], 3);
        assert_eq!(v["model_name"], "m");
        assert_eq!(v["model_sha256"], "abc");
        let orphan = birdnet_db::sqlite::DetectionRow::default();
        let v = serde_json::to_value(ExportedDetection::new(
            &orphan,
            &birdnet_db::sqlite::ModelAttribution::from_runs(runs.clone()),
            &RowProvenance::default(),
        ))
        .unwrap();
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
        let csv = detections_to_csv(&[row], &birdnet_db::sqlite::ModelAttribution::default());
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
