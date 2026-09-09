//! Raven selection tables and Audacity label tracks (FR-1).
//!
//! The ecologist's next step after a detection is Raven, and a station that
//! cannot hand its detections to Raven makes them re-find every call by ear.
//! A selection table is a tab-separated file whose rows are *begin and end
//! seconds inside an audio file*; this writes the format BirdNET-Analyzer
//! writes (its `RAVEN_TABLE_HEADER`, v2.0.0 `analyze/utils.py`, and the
//! combined table `analyze/core.py` builds with `Begin Path` and
//! `File Offset (s)`), so anything that already reads that reads this.
//!
//! Three routes:
//!
//! * `GET /api/v2/detections/export/raven?from&to` — one combined table over
//!   every detection with a clip, `Begin Path` naming the clip, times inside
//!   it. Raven Pro opens a multi-file table against the recordings folder.
//! * `GET /api/v2/recordings/{clip}/raven.txt` — the table for one clip.
//! * `GET /api/v2/recordings/{clip}/labels.txt` — the same selections as an
//!   Audacity label track (`begin\tend\tlabel`, File → Import → Labels).
//!
//! Where a selection sits comes from migration 47: the extractor records the
//! lead-in it actually wrote and the window's length. A row from before that
//! column has neither, and its selection is the whole clip — honest, and
//! still a file Raven can open. A row with no clip length at all, or no clip,
//! cannot be placed in any file and is left out rather than given a made-up
//! window.

use std::fmt::Write as _;

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use birdnet_db::sqlite::DetectionRow;
use serde::Deserialize;
use serde_json::json;

use super::{MAX_EXPORT_ROWS, export_too_large};
use crate::routes::is_valid_date;
use crate::routes::recordings::is_safe_filename;
use crate::state::AppState;

/// The header BirdNET-Analyzer writes, column for column.
pub const RAVEN_HEADER: &str = "Selection\tView\tChannel\tBegin Time (s)\tEnd Time (s)\tLow Freq (Hz)\tHigh Freq (Hz)\tCommon Name\tSpecies Code\tConfidence\tBegin Path\tFile Offset (s)";

/// The band the model listens to, as BirdNET-Analyzer's `SIG_FMIN` /
/// `SIG_FMAX` (v2.0.0 `config.py`) put on every selection: the model's band,
/// not a measured one, and the same number the reference tool writes.
pub const LOW_FREQ_HZ: u32 = 0;
/// See [`LOW_FREQ_HZ`].
pub const HIGH_FREQ_HZ: u32 = 15_000;

/// The seconds a detection occupies inside its clip, or `None` when the row
/// cannot be placed in a file.
#[must_use]
pub fn selection_window(row: &DetectionRow) -> Option<(f64, f64)> {
    row.file_name.as_ref()?;
    match (row.clip_offset_secs, row.detection_secs) {
        (Some(begin), Some(len)) => Some((begin, begin + len)),
        // Written before migration 47: the clip is the best answer there is.
        _ => row.duration_secs.map(|d| (0.0, d)),
    }
}

/// A name with the two characters that would break a tab-separated line
/// replaced by spaces.
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// The combined selection table for `rows`, in the order given, numbered
/// from one. `code_for` supplies the eBird species code for a scientific
/// name; a species without one is coded by its scientific name, as the
/// reference does with a label it has no code for.
pub fn selection_table<'a, F>(rows: &'a [DetectionRow], code_for: F) -> String
where
    F: Fn(&'a str) -> Option<&'a str>,
{
    let mut out = String::from(RAVEN_HEADER);
    out.push('\n');
    let mut n = 0_usize;
    for row in rows {
        let Some((begin, end)) = selection_window(row) else {
            continue;
        };
        let Some(clip) = row.file_name.as_deref() else {
            continue;
        };
        n += 1;
        let code = code_for(&row.sci_name).unwrap_or(&row.sci_name);
        let _ = writeln!(
            out,
            "{n}\tSpectrogram 1\t1\t{begin:.3}\t{end:.3}\t{LOW_FREQ_HZ}\t{HIGH_FREQ_HZ}\t{}\t{}\t{:.4}\t{}\t{begin:.3}",
            field(&row.com_name),
            field(code),
            row.confidence,
            field(clip),
        );
    }
    out
}

/// The same selections as an Audacity label track: `begin\tend\tlabel`, the
/// label the common name and the confidence as a percentage.
pub fn audacity_labels(rows: &[DetectionRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let Some((begin, end)) = selection_window(row) else {
            continue;
        };
        let _ = writeln!(
            out,
            "{begin:.3}\t{end:.3}\t{} {:.0}%",
            field(&row.com_name),
            row.confidence * 100.0
        );
    }
    out
}

#[derive(Deserialize)]
pub(super) struct RavenQuery {
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

fn attachment(body: String, name: &str) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "text/tab-separated-values; charset=utf-8",
            ),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    )
        .into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

/// `GET /detections/export/raven`: one table over every detection with a clip.
pub(super) async fn export_raven(
    State(state): State<AppState>,
    Query(query): Query<RavenQuery>,
) -> Response {
    for date in [&query.from, &query.to].into_iter().flatten() {
        if !is_valid_date(date) {
            return bad_request("invalid date format, expected YYYY-MM-DD");
        }
    }
    let db = state.clone();
    let (from, to) = (query.from.clone(), query.to.clone());
    let result = tokio::task::spawn_blocking(move || {
        db.with_db(|conn| {
            birdnet_db::sqlite::analytic_detections(
                conn,
                from.as_deref(),
                to.as_deref(),
                MAX_EXPORT_ROWS,
            )
        })
    })
    .await;
    match result {
        Ok(Ok((mut rows, truncated))) => {
            if truncated {
                return export_too_large().into_response();
            }
            // The read comes newest first; a selection table reads in time
            // order, clip by clip.
            rows.sort_by(|a, b| {
                a.file_name
                    .cmp(&b.file_name)
                    .then(a.date.cmp(&b.date))
                    .then(a.time.cmp(&b.time))
            });
            let table = selection_table(&rows, |sci| state.ebird_species_code(sci));
            attachment(table, "detections.raven.txt")
        }
        Ok(Err(e)) => db_error(&e.to_string()),
        Err(e) => db_error(&e.to_string()),
    }
}

fn db_error(detail: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": "database error", "detail": detail }).to_string(),
    )
        .into_response()
}

/// The rows for one clip, or the response to give instead.
async fn rows_for_clip(
    state: &AppState,
    filename: &str,
) -> Result<Vec<DetectionRow>, Box<Response>> {
    if !is_safe_filename(filename) {
        return Err(Box::new(bad_request("invalid filename")));
    }
    let db = state.clone();
    let name = filename.to_owned();
    let rows = tokio::task::spawn_blocking(move || {
        db.with_db(|conn| birdnet_db::sqlite::detections_for_clip(conn, &name))
    })
    .await
    .map_err(|e| Box::new(db_error(&e.to_string())))?
    .map_err(|e| Box::new(db_error(&e.to_string())))?;
    if rows.is_empty() {
        return Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                json!({ "error": "no detection names this clip" }).to_string(),
            )
                .into_response(),
        ));
    }
    Ok(rows)
}

/// `GET /recordings/{filename}/raven.txt`.
pub(super) async fn clip_raven(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    match rows_for_clip(&state, &filename).await {
        Ok(rows) => attachment(
            selection_table(&rows, |sci| state.ebird_species_code(sci)),
            &format!("{filename}.raven.txt"),
        ),
        Err(resp) => *resp,
    }
}

/// `GET /recordings/{filename}/labels.txt`.
pub(super) async fn clip_labels(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    match rows_for_clip(&state, &filename).await {
        Ok(rows) => attachment(audacity_labels(&rows), &format!("{filename}.labels.txt")),
        Err(resp) => *resp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        clip: Option<&str>,
        offset: Option<f64>,
        len: Option<f64>,
        dur: Option<f64>,
    ) -> DetectionRow {
        DetectionRow {
            sci_name: "Turdus merula".to_owned(),
            com_name: "Eurasian\tBlackbird".to_owned(),
            confidence: 0.912_34,
            file_name: clip.map(str::to_owned),
            clip_offset_secs: offset,
            detection_secs: len,
            duration_secs: dur,
            ..DetectionRow::default()
        }
    }

    #[test]
    fn a_stored_offset_places_the_selection_and_an_absent_one_spans_the_clip() {
        assert_eq!(
            selection_window(&row(Some("a.wav"), Some(1.5), Some(3.0), Some(6.0))),
            Some((1.5, 4.5))
        );
        assert_eq!(
            selection_window(&row(Some("b.wav"), None, None, Some(6.0))),
            Some((0.0, 6.0))
        );
        assert_eq!(
            selection_window(&row(Some("c.wav"), None, None, None)),
            None
        );
        assert_eq!(
            selection_window(&row(None, Some(1.5), Some(3.0), Some(6.0))),
            None
        );
    }

    #[test]
    fn the_table_is_the_reference_format_with_tabs_kept_out_of_the_fields() {
        let rows = [
            row(Some("a.wav"), Some(1.5), Some(3.0), Some(6.0)),
            row(Some("c.wav"), None, None, None),
        ];
        let table = selection_table(&rows, |sci| (sci == "Turdus merula").then_some("eurbla"));
        let mut lines = table.lines();
        assert_eq!(lines.next(), Some(RAVEN_HEADER));
        assert_eq!(
            lines.next(),
            Some(
                "1\tSpectrogram 1\t1\t1.500\t4.500\t0\t15000\tEurasian Blackbird\teurbla\t0.9123\ta.wav\t1.500"
            )
        );
        assert_eq!(lines.next(), None, "a row with no clip length is left out");
        let uncoded = selection_table(&rows[..1], |_| None);
        assert!(
            uncoded
                .lines()
                .nth(1)
                .unwrap()
                .contains("\tTurdus merula\t"),
            "a species with no code is coded by its scientific name: {uncoded}"
        );
    }

    #[test]
    fn the_label_track_is_begin_end_label() {
        let rows = [row(Some("a.wav"), Some(1.5), Some(3.0), Some(6.0))];
        assert_eq!(
            audacity_labels(&rows),
            "1.500\t4.500\tEurasian Blackbird 91%\n"
        );
    }
}
