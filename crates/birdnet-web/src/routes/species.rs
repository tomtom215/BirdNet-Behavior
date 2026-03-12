//! Species API endpoints.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DbError, HourlyCount, SpeciesCount};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Species routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species/top", get(top_species))
        .route("/species/search", get(search_species))
        .route("/species/activity", get(hourly_activity))
        .route("/species/detail", get(species_detail))
}

#[derive(Deserialize)]
struct TopSpeciesQuery {
    limit: Option<u32>,
}

async fn top_species(
    State(state): State<AppState>,
    Query(query): Query<TopSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = query.limit.unwrap_or(20);

    let result: Result<Result<Vec<SpeciesCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::top_species(conn, limit))
        })
        .await;

    match result {
        Ok(Ok(species)) => {
            let total = species.len();
            (
                StatusCode::OK,
                Json(json!({
                    "species": species,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
                "species": [],
                "total": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
                "species": [],
                "total": 0,
            })),
        ),
    }
}

#[derive(Deserialize)]
struct SearchSpeciesQuery {
    q: String,
    limit: Option<u32>,
}

/// Maximum length of a species search query string.
const MAX_SEARCH_LEN: usize = 200;

/// `GET /api/v2/species/search?q=...` — Search species by name.
async fn search_species(
    State(state): State<AppState>,
    Query(query): Query<SearchSpeciesQuery>,
) -> (StatusCode, Json<Value>) {
    let search = query.q;
    let limit = query.limit.unwrap_or(20);

    if search.len() > MAX_SEARCH_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "search query too long",
                "species": [],
                "total": 0,
            })),
        );
    }

    let result: Result<Result<Vec<SpeciesCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::search_species(conn, &search, limit))
        })
        .await;

    match result {
        Ok(Ok(species)) => {
            let total = species.len();
            (
                StatusCode::OK,
                Json(json!({
                    "species": species,
                    "total": total,
                })),
            )
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
                "species": [],
                "total": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
                "species": [],
                "total": 0,
            })),
        ),
    }
}

#[derive(Deserialize)]
struct SpeciesDetailQuery {
    name: String,
}

/// `GET /api/v2/species/detail?name=...` — Species detail with summary and hourly activity.
async fn species_detail(
    State(state): State<AppState>,
    Query(query): Query<SpeciesDetailQuery>,
) -> (StatusCode, Json<Value>) {
    let name = query.name;

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let summary = birdnet_db::sqlite::species_summary(conn, &name)?;
            let hourly = birdnet_db::sqlite::species_hourly_activity(conn, &name)?;
            Ok::<_, DbError>((summary, hourly))
        })
    })
    .await;

    match result {
        Ok(Ok((Some(summary), hourly))) => (
            StatusCode::OK,
            Json(json!({
                "species": {
                    "com_name": summary.com_name,
                    "sci_name": summary.sci_name,
                    "count": summary.count,
                    "avg_confidence": summary.avg_confidence,
                    "first_seen": summary.first_seen,
                    "last_seen": summary.last_seen,
                },
                "hourly_activity": hourly,
            })),
        ),
        Ok(Ok((None, _))) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "species not found",
            })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
            })),
        ),
    }
}

#[derive(Deserialize)]
struct ActivityQuery {
    date: String,
}

async fn hourly_activity(
    State(state): State<AppState>,
    Query(query): Query<ActivityQuery>,
) -> (StatusCode, Json<Value>) {
    let date = query.date;

    if !super::is_valid_date(&date) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "invalid date format, expected YYYY-MM-DD",
                "activity": [],
            })),
        );
    }

    let result: Result<Result<Vec<HourlyCount>, DbError>, _> =
        tokio::task::spawn_blocking(move || {
            state.with_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &date))
        })
        .await;

    match result {
        Ok(Ok(hours)) => (
            StatusCode::OK,
            Json(json!({
                "activity": hours,
            })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
                "activity": [],
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
                "activity": [],
            })),
        ),
    }
}
