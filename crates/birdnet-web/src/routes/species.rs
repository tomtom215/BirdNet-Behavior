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
        .route("/species/tracking", get(species_tracking))
}

/// Query for [`species_tracking`].
#[derive(Deserialize)]
struct TrackingQuery {
    /// Date to report, `YYYY-MM-DD`. Defaults to today.
    date: Option<String>,
    /// Return only species with something notable about them.
    notable_only: Option<bool>,
}

/// `GET /api/v2/species/tracking` — what is notable about today's species.
///
/// First-ever, first-of-the-year, first-of-the-season, and returning after an
/// absence, for every species detected on the date.
///
/// The windows are reported alongside the species, and not as a courtesy: a
/// bare "first this season" is unreadable without knowing which season the
/// station thinks it is in, and that depends on the latitude it was given. A
/// station with no latitude reports `season: null`, which is the honest
/// answer — it has no seasons, and nothing will ever be new in one.
async fn species_tracking(
    State(state): State<AppState>,
    Query(q): Query<TrackingQuery>,
) -> Json<Value> {
    let date = q
        .date
        .filter(|d| is_iso_date(d))
        .unwrap_or_else(crate::routes::pages::today_date_string);
    let notable_only = q.notable_only.unwrap_or(false);

    let (windows, rows) = state.with_read_db(|conn| {
        let windows = crate::tracking::resolve_windows(conn, &date);
        let rows =
            birdnet_db::species_tracking::statuses_for_date(conn, &date, windows.as_windows())
                .unwrap_or_default();
        (windows, rows)
    });

    let species: Vec<Value> = rows
        .iter()
        .filter(|r| !notable_only || r.status.is_notable())
        .map(|r| {
            json!({
                "sci_name": r.sci_name,
                "com_name": r.com_name,
                "headline": r.status.headline(),
                "new_ever": r.status.new_ever,
                "new_this_year": r.status.new_this_year,
                "new_this_season": r.status.new_this_season,
                "returning_after_absence": r.status.returning_after_absence,
                "days_since_previous": r.status.days_since_previous,
            })
        })
        .collect();

    Json(json!({
        "date": date,
        "year_start": windows.year_start,
        "season": windows.season,
        "season_start": windows.season_start,
        "absence_days": windows.absence_days,
        "species": species,
    }))
}

/// Whether `s` is a plausible `YYYY-MM-DD`.
///
/// Shape only. The query is parameterised so a malformed date is not a
/// injection risk; the check is here so a typo returns an empty day rather
/// than silently answering for today, which would look like a working page
/// showing the wrong data.
fn is_iso_date(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
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
                "error": crate::routes::log_internal("internal error", &e),
                "species": [],
                "total": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
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
                "error": crate::routes::log_internal("internal error", &e),
                "species": [],
                "total": 0,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
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
                "error": crate::routes::log_internal("internal error", &e),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
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
                "error": crate::routes::log_internal("internal error", &e),
                "activity": [],
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::routes::log_internal("internal error", &e),
                "activity": [],
            })),
        ),
    }
}
