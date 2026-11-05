//! CSV and JSON export of what each classifier said about each detection
//! (migrations 57 and 58).
//!
//! One row per (detection, classifier), the form the table stores and the form a
//! spreadsheet or a script can filter: a wide form would need a column per
//! classifier, and the classifiers a station runs change over its life.
//!
//! Scores are each classifier's own, on its own scale, and are not combined: a
//! `Score` is comparable with another row of the same `Model_Id` and not with a
//! different classifier's. `State` says what the classifier could say (`heard`,
//! `below_bar`, `no_label`, `failed`), and only the first two carry a `Score` and
//! the `Bar` it was judged against.

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
pub(super) struct ScoresQuery {
    /// Output format: "csv" or "json" (default: "csv").
    format: Option<String>,
    /// Start date filter (inclusive, YYYY-MM-DD).
    from: Option<String>,
    /// End date filter (inclusive, YYYY-MM-DD).
    to: Option<String>,
}

pub(super) async fn export_model_scores(
    State(state): State<AppState>,
    Query(query): Query<ScoresQuery>,
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
    let (from, to) = (query.from.clone(), query.to.clone());
    let permit = state.heavy_read_permit().await;
    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        state.with_read_db(|conn| {
            birdnet_db::sqlite::model_scores_between(
                conn,
                from.as_deref(),
                to.as_deref(),
                MAX_EXPORT_ROWS,
            )
        })
    })
    .await;

    match result {
        Ok(Ok((_, true))) => export_too_large(),
        Ok(Ok((rows, false))) => {
            if format == "json" {
                let total = rows.len();
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    serde_json::to_string(&json!({"model_scores": rows, "total": total}))
                        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.into()),
                )
                    .into_response()
            } else {
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=\"detection-model-scores.csv\"",
                        ),
                    ],
                    scores_to_csv(&rows),
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

/// `Date,Time,Sci_Name,File_Name` name the detection exactly as the detections
/// export does, so the two join on them. All four are needed: two sources
/// hearing one bird in one second share the first three. `File_Name` is empty
/// for a detection with no clip. `Score` and `Bar` are empty when the
/// classifier did not score (`no_label`, `failed`), never `0`.
fn scores_to_csv(rows: &[birdnet_db::sqlite::DetectionModelScore]) -> String {
    let mut csv = String::with_capacity(rows.len() * 90);
    csv.push_str(
        "Date,Time,Sci_Name,File_Name,Model_Id,State,Score,Bar,Window_Start_Secs,Window_End_Secs\n",
    );
    let num = |v: Option<f64>, places: usize| v.map_or(String::new(), |x| format!("{x:.places$}"));
    for r in rows {
        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{},{},{},{}",
            escape_csv(&r.date),
            escape_csv(&r.time),
            escape_csv(&r.sci_name),
            escape_csv(&r.file_name),
            escape_csv(&r.answer.model_id),
            r.answer.state.as_str(),
            num(r.answer.score, 4),
            num(r.answer.bar, 4),
            num(r.answer.window_start_secs, 2),
            num(r.answer.window_end_secs, 2),
        );
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite::{DetectionModelScore, ModelScore, ModelScoreState};

    fn row(model: &str, state: ModelScoreState, score: Option<f64>) -> DetectionModelScore {
        DetectionModelScore {
            date: "2026-11-01".into(),
            time: "06:30:00".into(),
            sci_name: "Pica pica".into(),
            file_name: "pond.wav".into(),
            answer: ModelScore {
                model_id: model.into(),
                state,
                score,
                bar: score.map(|_| 0.7),
                window_start_secs: Some(3.0),
                window_end_secs: Some(7.5),
            },
        }
    }

    #[test]
    fn the_csv_has_one_row_per_classifier_and_the_header_the_manual_names() {
        let csv = scores_to_csv(&[
            row("v3", ModelScoreState::Heard, Some(0.87)),
            row("perch", ModelScoreState::BelowBar, Some(0.41)),
        ]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0],
            "Date,Time,Sci_Name,File_Name,Model_Id,State,Score,Bar,Window_Start_Secs,Window_End_Secs"
        );
        assert_eq!(
            lines[1],
            "2026-11-01,06:30:00,Pica pica,pond.wav,v3,heard,0.8700,0.7000,3.00,7.50"
        );
        assert_eq!(
            lines[2],
            "2026-11-01,06:30:00,Pica pica,pond.wav,perch,below_bar,0.4100,0.7000,3.00,7.50"
        );
    }

    #[test]
    fn a_classifier_that_did_not_score_has_empty_cells_and_not_zeros() {
        let csv = scores_to_csv(&[
            row("regional", ModelScoreState::NoLabel, None),
            row("flaky", ModelScoreState::Failed, None),
        ]);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[1],
            "2026-11-01,06:30:00,Pica pica,pond.wav,regional,no_label,,,3.00,7.50"
        );
        assert_eq!(
            lines[2],
            "2026-11-01,06:30:00,Pica pica,pond.wav,flaky,failed,,,3.00,7.50"
        );
    }

    #[test]
    fn an_operator_typed_model_id_cannot_break_the_csv_or_run_as_a_formula() {
        let csv = scores_to_csv(&[row(
            "=HYPERLINK(\"x\"),v2",
            ModelScoreState::Heard,
            Some(0.9),
        )]);
        let line = csv.lines().nth(1).unwrap();
        assert!(line.contains(",\"'=HYPERLINK(\"\"x\"\"),v2\","), "{line}");
    }

    #[test]
    fn the_json_row_is_flat_and_omits_what_the_classifier_did_not_say() {
        let v = serde_json::to_value(row("regional", ModelScoreState::NoLabel, None)).unwrap();
        assert_eq!(v["model_id"], "regional");
        assert_eq!(v["state"], "no_label");
        assert_eq!(v["sci_name"], "Pica pica");
        assert_eq!(v["file_name"], "pond.wav", "the JSON names the clip too");
        assert!(v["score"].is_null() && v["bar"].is_null(), "{v}");
    }

    /// A detection with no clip has an empty `File_Name`, not the word "null".
    #[test]
    fn a_detection_with_no_clip_has_an_empty_file_name() {
        let mut r = row("v3", ModelScoreState::Heard, Some(0.9));
        r.file_name = String::new();
        let csv = scores_to_csv(&[r]);
        assert_eq!(
            csv.lines().nth(1).unwrap(),
            "2026-11-01,06:30:00,Pica pica,,v3,heard,0.9000,0.7000,3.00,7.50"
        );
    }
}
