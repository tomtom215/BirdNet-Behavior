//! Detection API endpoints.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DailyCount, DateRange, DbError, DetectionFilter, DetectionRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Detection routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections", get(list_detections))
        .route("/detections/recent", get(recent_detections))
        .route("/detections/daily", get(daily_detections))
        .route("/detections/models", get(detection_models))
}

#[derive(Deserialize)]
struct DetectionQuery {
    date: Option<String>,
    species: Option<String>,
    /// Audio source label, as the `source` field of a row.
    source: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

/// A filter sent empty (`?species=`, as a form with a blank field submits it)
/// means "no filter", as it does on the search page. Before the filters were
/// combined, an empty `species` matched `Com_Name = ''` and returned nothing.
/// An empty `date` never reaches here: it fails the format check first.
fn nonblank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Maximum allowed limit for detection queries.
const MAX_LIMIT: u32 = 1000;

async fn list_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = query.limit.unwrap_or(100).min(MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);

    // Validate date format if provided
    if let Some(ref date) = query.date
        && !is_valid_date(date)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid date format, expected YYYY-MM-DD",
                "detections": [],
                "count": 0,
            })),
        );
    }

    // Every filter given is applied, together. This used to branch on
    // `species`, then `date`, so a request with both silently dropped the
    // date — and `source` did not exist. The search page's filter already
    // combines all three; this is that filter, so the two cannot disagree.
    let filter = DetectionFilter {
        species: nonblank(query.species).into_iter().collect(),
        dates: nonblank(query.date).map_or(DateRange::Any, DateRange::On),
        sources: nonblank(query.source).into_iter().collect(),
        ..DetectionFilter::default()
    };
    let result: Result<Result<(Vec<DetectionRow>, i64), DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_read_db(|conn| {
                let rows = birdnet_db::sqlite::search_detections(conn, &filter, limit, offset)?;
                let total = birdnet_db::sqlite::search_detection_count(conn, &filter)?;
                Ok((rows, total))
            })
        })
        .await;

    match result {
        Ok(Ok((detections, total_count))) => {
            let count = detections.len();
            (
                StatusCode::OK,
                Json(json!({
                    "detections": detections,
                    "count": count,
                    "total_count": total_count,
                    "limit": limit,
                    "offset": offset,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
                "detections": [],
                "count": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
                "detections": [],
                "count": 0,
            })),
        ),
    }
}

#[derive(Deserialize)]
struct ModelsQuery {
    date: Option<String>,
    time: Option<String>,
    sci_name: Option<String>,
    /// The detection's clip (`file_name` on a detection row). Needed only when
    /// several detections share the date, time and species; `""` names one with
    /// no clip.
    file_name: Option<String>,
}

/// Which detection a models request named.
enum ModelsLookup {
    /// No detection has that date, time, species (and clip).
    NotFound,
    /// Several detections share the date, time and species; the caller must say
    /// which by clip. Carries every clip.
    Ambiguous(Vec<String>),
    /// The detection: its clip, what its row recorded, and every stored answer.
    Found(
        String,
        birdnet_db::sqlite::DetectionModelSummary,
        Vec<birdnet_db::sqlite::ModelScore>,
    ),
}

/// What each classifier said about one detection (migrations 57 and 58).
///
/// `model_id` and `model_agreement` are the row's own record of which
/// classifier supplied its answer and how many cleared their bars; `models` is
/// every classifier's own answer, which may be empty (a row from before the
/// feature, a station that runs one classifier, or a source only one classifier
/// listened to) and is then not the same as "no classifier heard it". Each score
/// is on its classifier's own scale and none is combined with another.
///
/// Two sources hearing one bird in one second are two detections that share a
/// date, a time and a species. Pass `file_name` (as it appears on the detection
/// row) to say which; without it the request is answered when the triple names
/// one detection and refused with `409` and the clips to choose from when it
/// names several, rather than answering for whichever came first.
async fn detection_models(
    State(state): State<AppState>,
    Query(query): Query<ModelsQuery>,
) -> (StatusCode, Json<Value>) {
    let (Some(date), Some(time), Some(sci_name)) = (
        nonblank(query.date),
        nonblank(query.time),
        nonblank(query.sci_name),
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "date, time and sci_name are required"})),
        );
    };
    if !is_valid_date(&date) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid date format, expected YYYY-MM-DD"})),
        );
    }
    let (read_date, read_time, read_sci, named_file) = (
        date.clone(),
        time.clone(),
        sci_name.clone(),
        query.file_name,
    );
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let file = if let Some(file) = named_file {
                file
            } else {
                let mut files = birdnet_db::sqlite::detection_files_at(
                    conn, &read_date, &read_time, &read_sci,
                )?;
                match files.len() {
                    0 => return Ok(ModelsLookup::NotFound),
                    1 => files.remove(0),
                    _ => return Ok(ModelsLookup::Ambiguous(files)),
                }
            };
            let key = birdnet_db::sqlite::DetectionKey {
                date: &read_date,
                time: &read_time,
                sci_name: &read_sci,
                file_name: Some(&file),
            };
            let Some(summary) = birdnet_db::sqlite::detection_model_summary(conn, &key)? else {
                return Ok(ModelsLookup::NotFound);
            };
            let models = birdnet_db::sqlite::model_scores_for_detection(conn, &key)?;
            Ok::<_, DbError>(ModelsLookup::Found(file, summary, models))
        })
    })
    .await;

    match result {
        Ok(Ok(ModelsLookup::Found(file, summary, models))) => {
            let mut body = json!({
                "detection": format!("{date} {time} {sci_name}"),
                "file_name": file,
                "models": models,
            });
            // Omitted when unknown, as on a detection row: a legacy row has no
            // record of either, which `null` would blur with a recorded zero.
            if let Some(id) = summary.model_id {
                body["model_id"] = json!(id);
            }
            if let Some(n) = summary.agreement {
                body["model_agreement"] = json!(n);
            }
            (StatusCode::OK, Json(body))
        }
        Ok(Ok(ModelsLookup::Ambiguous(files))) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "several detections share that date, time and species; pass file_name to choose one",
                "file_names": files,
            })),
        ),
        Ok(Ok(ModelsLookup::NotFound)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such detection"})),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
    }
}

async fn recent_detections(
    State(state): State<AppState>,
    Query(query): Query<DetectionQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = query.limit.unwrap_or(20).min(MAX_LIMIT);

    let result: Result<Result<Vec<DetectionRow>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_read_db(|conn| birdnet_db::sqlite::recent_detections(conn, limit))
        })
        .await;

    match result {
        Ok(Ok(detections)) => {
            let total = detections.len();
            (
                StatusCode::OK,
                Json(json!({
                    "detections": detections,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
    }
}

#[derive(Deserialize)]
struct DailyQuery {
    days: Option<u32>,
}

/// Daily detection counts for trend analysis.
async fn daily_detections(
    State(state): State<AppState>,
    Query(query): Query<DailyQuery>,
) -> (StatusCode, Json<Value>) {
    let days = query.days.unwrap_or(7).min(365);

    let result: Result<Result<Vec<DailyCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_read_db(|conn| birdnet_db::sqlite::daily_counts(conn, days))
        })
        .await;

    match result {
        Ok(Ok(daily)) => {
            let total = daily.len();
            (
                StatusCode::OK,
                Json(json!({
                    "daily": daily,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
    }
}

// Use the shared date validation from the routes module.
use super::is_valid_date;
