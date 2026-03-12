//! Detection API endpoints.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::sqlite::{DailyCount, DbError, DetectionRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Detection routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detections", get(list_detections))
        .route("/detections/recent", get(recent_detections))
        .route("/detections/daily", get(daily_detections))
}

#[derive(Deserialize)]
struct DetectionQuery {
    date: Option<String>,
    species: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
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
    if let Some(ref date) = query.date {
        if !is_valid_date(date) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid date format, expected YYYY-MM-DD",
                    "detections": [],
                    "count": 0,
                })),
            );
        }
    }

    // Track whether this is a paginated query (for including total_count)
    let is_paginated = query.species.is_none() && query.date.is_none();

    #[allow(clippy::type_complexity)]
    let result: Result<Result<(Vec<DetectionRow>, Option<i64>), DbError>, _> =
        if let Some(species) = &query.species {
            let species = species.clone();
            tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    birdnet_db::sqlite::detections_by_species(conn, &species, limit)
                        .map(|rows| (rows, None))
                })
            })
            .await
        } else if let Some(date) = &query.date {
            let date = date.clone();
            tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    birdnet_db::sqlite::detections_by_date(conn, &date).map(|rows| (rows, None))
                })
            })
            .await
        } else {
            tokio::task::spawn_blocking(move || {
                state.with_db(|conn| {
                    let rows = birdnet_db::sqlite::recent_detections_page(conn, limit, offset)?;
                    let total = birdnet_db::sqlite::detection_count(conn)?;
                    Ok((rows, Some(total)))
                })
            })
            .await
        };

    match result {
        Ok(Ok((detections, total_count))) => {
            let count = detections.len();
            let mut response = json!({
                "detections": detections,
                "count": count,
                "limit": limit,
                "offset": offset,
            });
            if is_paginated {
                if let Some(total) = total_count {
                    response["total_count"] = json!(total);
                }
            }
            (StatusCode::OK, Json(response))
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": e.to_string(),
                "detections": [],
                "count": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("internal error: {e}"),
                "detections": [],
                "count": 0,
            })),
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
            state.with_db(|conn| birdnet_db::sqlite::recent_detections(conn, limit))
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
            Json(json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
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
            state.with_db(|conn| birdnet_db::sqlite::daily_counts(conn, days))
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
            Json(json!({ "error": e.to_string() })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("internal error: {e}") })),
        ),
    }
}

/// Validate a date string is in YYYY-MM-DD format.
fn is_valid_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_date_format() {
        assert!(is_valid_date("2026-03-12"));
        assert!(is_valid_date("2020-01-01"));
        assert!(is_valid_date("1999-12-31"));
    }

    #[test]
    fn invalid_date_format() {
        assert!(!is_valid_date(""));
        assert!(!is_valid_date("2026"));
        assert!(!is_valid_date("03-12-2026"));
        assert!(!is_valid_date("2026/03/12"));
        assert!(!is_valid_date("not-a-date"));
        assert!(!is_valid_date("20260312"));
        assert!(!is_valid_date("2026-3-12"));
    }
}
