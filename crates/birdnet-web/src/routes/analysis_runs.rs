//! `GET /api/v2/analysis-runs` — which model made which rows (R-1).
//!
//! One run per detection-daemon start, with the SHA-256 of the model and
//! labels it analysed with and the settings in force. A detection row's
//! `run_id` points at one of these; the exports resolve it to the model's name
//! and checksum per row, and this endpoint is where a researcher reads the
//! rest: when the run started, how many rows it produced, what the label
//! count and geomodel were.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Mount the analysis-run routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analysis-runs", get(list_runs))
        .route("/analysis-runs/{id}", get(get_run))
}

/// The default and largest page.
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;

/// Query for [`list_runs`].
#[derive(Debug, Deserialize)]
struct ListQuery {
    /// Newest `limit` runs; default 50, at most 500.
    limit: Option<u32>,
}

/// A run with the number of detection rows that carry it.
fn run_json(run: &birdnet_db::sqlite::AnalysisRun, detections: i64) -> Value {
    let mut v = serde_json::to_value(run).unwrap_or_else(|_| json!({}));
    if let Value::Object(map) = &mut v {
        map.insert("detections".into(), json!(detections));
    }
    v
}

/// `GET /api/v2/analysis-runs` — newest first.
async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let runs = birdnet_db::sqlite::list_analysis_runs(conn, limit)?;
            let counts = birdnet_db::sqlite::detection_counts_by_run(conn)?;
            Ok::<_, birdnet_db::sqlite::DbError>((runs, counts))
        })
    })
    .await;
    match result {
        Ok(Ok((runs, counts))) => {
            let items: Vec<Value> = runs
                .iter()
                .map(|run| run_json(run, counts.get(&run.id).copied().unwrap_or(0)))
                .collect();
            (StatusCode::OK, Json(json!({ "runs": items }))).into_response()
        }
        Ok(Err(e)) => internal(&e),
        Err(e) => internal(&e),
    }
}

/// `GET /api/v2/analysis-runs/{id}`.
async fn get_run(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let Some(run) = birdnet_db::sqlite::analysis_run(conn, id)? else {
                return Ok(None);
            };
            let count = birdnet_db::sqlite::detection_counts_by_run(conn)?
                .get(&id)
                .copied()
                .unwrap_or(0);
            Ok::<_, birdnet_db::sqlite::DbError>(Some((run, count)))
        })
    })
    .await;
    match result {
        Ok(Ok(Some((run, count)))) => (StatusCode::OK, Json(run_json(&run, count))).into_response(),
        Ok(Ok(None)) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such analysis run" })),
        )
            .into_response(),
        Ok(Err(e)) => internal(&e),
        Err(e) => internal(&e),
    }
}

fn internal<E: std::fmt::Display>(e: &E) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": crate::routes::log_internal("internal error", e) })),
    )
        .into_response()
}
