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
        .route("/species/occurrence", get(species_occurrence))
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
) -> (StatusCode, Json<Value>) {
    state
        .run_blocking(move |state| {
            let date = q
                .date
                .filter(|d| is_iso_date(d))
                .unwrap_or_else(crate::routes::pages::today_date_string);
            let notable_only = q.notable_only.unwrap_or(false);

            // A read that failed is a 500, not `"species": []`: a consumer of
            // this endpoint stores what it is told, and an empty list is the
            // answer for a day on which nothing notable happened.
            let read = state.with_read_db(|conn| {
                let windows = crate::tracking::resolve_windows(conn, &date);
                let rows = birdnet_db::species_tracking::statuses_for_date(
                    conn,
                    &date,
                    windows.as_windows(),
                )?;
                Ok::<_, DbError>((windows, rows))
            });
            let (windows, rows) = match read {
                Ok(read) => read,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": crate::routes::log_internal("species tracking query failed", &e),
                        })),
                    );
                }
            };

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

            (
                StatusCode::OK,
                Json(json!({
                    "date": date,
                    "year_start": windows.year_start,
                    "season": windows.season,
                    "season_start": windows.season_start,
                    "absence_days": windows.absence_days,
                    "species": species,
                })),
            )
        })
        .await
}

/// Whether `s` is a plausible `YYYY-MM-DD`.
///
/// Shape only. The query is parameterised, so a malformed date is not an
/// injection risk. A value that fails this check is treated as absent, and an
/// absent date means today (what the OpenAPI document says); it is not a 400.
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

// ---------------------------------------------------------------------------
// Which species the occurrence filter admits (parity row P-3)
// ---------------------------------------------------------------------------

/// Query for [`species_occurrence`].
///
/// Every field is text, parsed by hand, so a malformed value is answered in the
/// JSON error shape every other refusal here uses and with the range named,
/// not with axum's plain-text `400`.
#[derive(Deserialize)]
struct OccurrenceQuery {
    /// The model's week, `1..=48`. Defaults to the current week.
    week: Option<String>,
    /// Text to find in either name.
    q: Option<String>,
    /// `true` for the admitted, `false` for the held back.
    admitted: Option<String>,
    /// Page size.
    limit: Option<String>,
    /// Species to skip.
    offset: Option<String>,
}

/// Why the station cannot answer, as the JSON body and the status it travels
/// with. `503` for all of them, with a `reason` a client can branch on: an
/// empty list would read as "nothing occurs here".
pub(crate) fn occurrence_unavailable(
    reason: &crate::occurrence::Unavailable,
) -> (StatusCode, Json<Value>) {
    use crate::occurrence::Unavailable;
    let (key, message) = match reason {
        Unavailable::NoLocation => (
            "no-location",
            "This station has no latitude and longitude, so the range filter has no place to be asked about. Set the station's location in Settings.".to_owned(),
        ),
        Unavailable::NoModel => (
            "no-range-model",
            "No range model is loaded, so the station does not filter species by where it is: every species the classifier knows is a candidate, wherever it is. There is no filter to list.".to_owned(),
        ),
        Unavailable::Failed(detail) => {
            // The detail names files and paths; it is for the log.
            let _ = crate::routes::log_internal("the range model could not be run", detail);
            (
                "range-model-failed",
                "The range model could not be run. The station's log has the reason.".to_owned(),
            )
        }
    };
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": message, "reason": key })),
    )
}

/// The `503` for a process with no detection daemon to ask.
pub(crate) fn occurrence_no_daemon() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "This process is not running the detection daemon, so there is no range filter to ask.",
            "reason": "no-detection-daemon",
        })),
    )
}

/// The model week of the station's current local date.
pub(crate) fn current_model_week() -> u32 {
    crate::occurrence::week_of(&crate::routes::pages::today_date_string()).unwrap_or(1)
}

/// `GET /api/v2/species/occurrence` — what the station's range filter says
/// about each species, for this place and week.
///
/// Answers "why is this bird never heard here?" with the number behind it: the
/// model's probability for the species, the threshold in force, and whether the
/// station therefore admits it. The station's coordinates are returned to one
/// decimal place (about 11 km): that says which region's birds the question was
/// about, and the endpoint is as open as the rest of the read API.
async fn species_occurrence(
    State(state): State<AppState>,
    Query(q): Query<OccurrenceQuery>,
) -> (StatusCode, Json<Value>) {
    use crate::occurrence::{
        DEFAULT_LIMIT, MAX_LIMIT, Selection, parse_admitted, parse_count, parse_week, select,
    };

    let bad = |message: String| (StatusCode::BAD_REQUEST, Json(json!({ "error": message })));
    let week = match parse_week(q.week.as_deref()) {
        Ok(w) => w,
        Err(e) => return bad(e),
    };
    let admitted = match parse_admitted(q.admitted.as_deref()) {
        Ok(a) => a,
        Err(e) => return bad(e),
    };
    let limit = match parse_count("limit", q.limit.as_deref(), 1, MAX_LIMIT) {
        Ok(n) => n.unwrap_or(DEFAULT_LIMIT),
        Err(e) => return bad(e),
    };
    let offset = match parse_count("offset", q.offset.as_deref(), 0, usize::MAX) {
        Ok(n) => n.unwrap_or(0),
        Err(e) => return bad(e),
    };
    let selection = Selection {
        q: q.q.clone(),
        admitted,
    };

    let Some(source) = state.occurrence_source() else {
        return occurrence_no_daemon();
    };
    let week_is_current = week.is_none();
    let week = week.unwrap_or_else(current_model_week);
    let names = state.names_handle();
    let also = crate::routes::pages::name_search::also_species(&names, q.q.as_deref());
    let also: std::collections::HashSet<String> = also.into_iter().collect();

    // The model runs on the blocking pool: it is an inference, however small.
    let report = state.run_blocking(move |_| source.report(week)).await;
    let report = match report {
        Ok(report) => report,
        Err(reason) => return occurrence_unavailable(&reason),
    };

    let picked = select(&report, &selection, &also);
    let rows: Vec<Value> = picked
        .species
        .iter()
        .skip(offset)
        .take(limit)
        .map(|s| {
            json!({
                "scientific_name": s.scientific_name,
                "common_name": s.common_name,
                "display_name": names.name(&s.scientific_name, &s.common_name),
                "probability": crate::occurrence::json_number(s.probability),
                "admitted": s.admitted,
                "verdict": crate::occurrence::Verdict::of(s, report.threshold).key(),
                "explanation": crate::occurrence::sentence(s, &report),
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "location": {
                "latitude": round_to_tenth(report.latitude),
                "longitude": round_to_tenth(report.longitude),
            },
            "week": report.week,
            "week_is_current": week_is_current,
            "threshold": crate::occurrence::json_number(report.threshold),
            "scored": picked.scored,
            "admitted_total": picked.admitted,
            "matched": picked.matched,
            "limit": limit,
            "offset": offset,
            "species": rows,
        })),
    )
}

/// A coordinate to one decimal place: about 11 km, enough to say which
/// region's birds were meant and no more than a public endpoint should say.
pub(crate) fn round_to_tenth(degrees: f64) -> f64 {
    (degrees * 10.0).round() / 10.0
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
            state.with_read_db(|conn| birdnet_db::sqlite::top_species(conn, limit))
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
            state.with_read_db(|conn| birdnet_db::sqlite::search_species(conn, &search, limit))
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
        state.with_read_db(|conn| {
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
            state.with_read_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &date))
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
