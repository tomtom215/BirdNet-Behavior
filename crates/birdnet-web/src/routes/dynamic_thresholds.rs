//! What the station has learned about each species (`P-11`): the learned
//! dynamic thresholds, listed, summarised and reset.
//!
//! tphakala/birdnet-go serves `GET /api/v2/dynamic-thresholds`, `…/stats`,
//! `…/{species}` and `…/{species}/events`, and `DELETE`s one species or (with
//! `?confirm=true`) all of them. This is the same surface adapted to what this
//! station keeps:
//!
//! * the rows are the `dynamic_thresholds` table (`birdnet_db::dynamic_thresholds`),
//!   which the detection daemon rewrites whenever a level changes, so a read is
//!   as fresh as the last level change;
//! * a species is named by its **scientific name**, the key the pipeline uses
//!   (upstream names it by common name);
//! * there is **no `…/events` history**: this station stores the current level
//!   and when it was first learned and last confirmed, not every change;
//! * a reset is `POST /api/v2/dynamic-thresholds/reset` behind the bearer token
//!   (see `routes::api_write`) and a form on `/admin/species/manage`, not a
//!   `DELETE`, because the token-gated surface is a table of exact paths.
//!
//! The reads are public, like `species/occurrence`: they say which species the
//! station has confirmed present, which its public detections already say.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use birdnet_db::dynamic_thresholds::LearnedThreshold;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Rows per page when `limit` is not given.
pub const DEFAULT_LIMIT: usize = 50;
/// The most rows one page may ask for.
pub const MAX_LIMIT: usize = 500;

/// The learned-threshold read routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dynamic-thresholds", get(list))
        .route("/dynamic-thresholds/stats", get(stats))
        .route("/dynamic-thresholds/{species}", get(one))
}

/// What a reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetOutcome {
    /// Stored rows deleted.
    pub rows_deleted: usize,
    /// Whether a running detection daemon was asked to forget them as well.
    /// `false` in a process with none (nothing in memory to forget).
    pub daemon_notified: bool,
}

/// Forget what was learned: delete the stored rows and ask the daemon's tracker
/// to forget them too.
///
/// Both, because either alone is undone: the tracker rewrites the whole table
/// from memory on its next level change, and a tracker that was not told keeps
/// judging by a level the table no longer shows. The rows go first, so the page
/// that follows reads right at once; the tracker applies its request before the
/// next detection it judges.
///
/// # Errors
///
/// The database error, if the delete failed. In that case the daemon is **not**
/// asked: a reset that half-happened would show levels in the table that the
/// tracker no longer applies.
pub fn reset(
    state: &AppState,
    which: &birdnet_core::detection::dynamic_threshold::Reset,
) -> Result<ResetOutcome, birdnet_db::sqlite::DbError> {
    use birdnet_core::detection::dynamic_threshold::Reset;
    let rows_deleted = state.with_db(|conn| match which {
        Reset::Species(name) => birdnet_db::dynamic_thresholds::delete_one(conn, name),
        Reset::All => birdnet_db::dynamic_thresholds::delete_all(conn),
    })?;
    let daemon_notified = state.dynamic_threshold_handle().is_some_and(|h| {
        h.request(which.clone());
        true
    });
    Ok(ResetOutcome {
        rows_deleted,
        daemon_notified,
    })
}

/// One learned level, as the API and the admin page show it.
#[derive(Debug, Clone)]
pub struct Learned {
    /// The stored row.
    pub row: LearnedThreshold,
    /// Whether the level is still in force (`now` is before its expiry).
    pub active: bool,
    /// Multiplier the level applies to the base threshold.
    pub multiplier: f64,
    /// The threshold the species is held to now, when the base is known.
    pub effective_threshold: f64,
    /// The base before the level: the operator's threshold for the species, or
    /// the station's.
    pub base_threshold: f64,
    /// Whether the base is the species' own threshold (`true`) or the
    /// station's (`false`).
    pub base_is_species: bool,
}

/// Everything the three reads and the admin page need, read in one go.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Every stored row, with what it means now.
    pub rows: Vec<Learned>,
    /// The running tracker's configuration, `None` without a daemon.
    pub config: Option<birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig>,
}

/// Read the table, the operator's thresholds and the station's, and the
/// tracker's configuration.
///
/// # Errors
///
/// The database error: an unreadable table is not an empty one, and callers
/// answer it as a failure.
pub fn snapshot(state: &AppState, now_ms: i64) -> Result<Snapshot, birdnet_db::sqlite::DbError> {
    let (rows, species, floor) = state.with_read_db(|conn| {
        let rows = birdnet_db::dynamic_thresholds::load_all(conn)?;
        let species = birdnet_db::sqlite::get_species_threshold_map(conn)?;
        Ok::<_, birdnet_db::sqlite::DbError>((
            rows,
            species,
            crate::routes::pages::confidence::station_floor(conn),
        ))
    })?;
    let config = state.dynamic_threshold_handle().map(|h| h.config());
    let min = config.map(|c| f64::from(c.min));
    let rows = rows
        .into_iter()
        .map(|row| {
            let active = now_ms < row.expires_at_ms;
            let multiplier =
                birdnet_core::detection::dynamic_threshold::level_multiplier(row.level);
            let (base_threshold, base_is_species) = match species.get(&row.sci_name) {
                Some(&t) => (t, true),
                None => (floor, false),
            };
            // The tracker's own rule (`Adjustment::apply`): multiply, never
            // below the floor, never above the base.
            let mut effective = base_threshold * multiplier;
            if let Some(min) = min {
                effective = effective.max(min);
            }
            let effective_threshold = effective.min(base_threshold);
            Learned {
                row,
                active,
                multiplier,
                effective_threshold,
                base_threshold,
                base_is_species,
            }
        })
        .collect();
    Ok(Snapshot { rows, config })
}

/// Four decimals: the configuration is stored as `f32`, and `0.3` widened to
/// `f64` prints as `0.30000001192092896`. The judging path uses the widened value;
/// what is shown to a person (or a script) is the number they typed.
fn r4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn row_json(state: &AppState, l: &Learned) -> Value {
    let r = &l.row;
    let common = state.names().name(&r.sci_name, &r.sci_name).to_owned();
    let secs = |ms: i64| birdnet_core::civil::rfc3339_utc(ms.div_euclid(1000));
    json!({
        "scientific_name": r.sci_name,
        "display_name": common,
        "level": r.level,
        "multiplier": r4(l.multiplier),
        "confirmations": r.confirmations,
        "first_learned": secs(r.first_learned_ms),
        "last_confirmed": secs(r.last_confirmed_ms),
        "expires_at": secs(r.expires_at_ms),
        "active": l.active,
        "base_threshold": r4(l.base_threshold),
        "base_is": if l.base_is_species { "species" } else { "station" },
        "effective_threshold": if l.active { json!(r4(l.effective_threshold)) } else { Value::Null },
    })
}

fn config_json(
    c: Option<birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig>,
) -> Value {
    c.map_or(Value::Null, |c| {
        json!({
            "trigger": r4(f64::from(c.trigger)),
            "min": r4(f64::from(c.min)),
            "valid_hours": c.valid_hours,
        })
    })
}

fn unavailable(what: &str, err: &birdnet_db::sqlite::DbError) -> (StatusCode, Json<Value>) {
    let msg = crate::routes::log_internal(what, err);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
}

/// Query for [`list`].
#[derive(Deserialize)]
struct ListQuery {
    limit: Option<String>,
    offset: Option<String>,
    /// `true` for only the levels still in force, `false` for only lapsed ones.
    active: Option<String>,
}

fn parse_count(
    name: &str,
    raw: Option<&str>,
    min: usize,
    max: usize,
) -> Result<Option<usize>, String> {
    let Some(raw) = raw else { return Ok(None) };
    match raw.trim().parse::<usize>() {
        Ok(n) if (min..=max).contains(&n) => Ok(Some(n)),
        _ => Err(format!("{name} must be a whole number from {min} to {max}")),
    }
}

/// `GET /api/v2/dynamic-thresholds` — what the station has learned, per species.
async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> (StatusCode, Json<Value>) {
    let bad = |m: String| (StatusCode::BAD_REQUEST, Json(json!({ "error": m })));
    let limit = match parse_count("limit", q.limit.as_deref(), 1, MAX_LIMIT) {
        Ok(n) => n.unwrap_or(DEFAULT_LIMIT),
        Err(e) => return bad(e),
    };
    let offset = match parse_count("offset", q.offset.as_deref(), 0, usize::MAX) {
        Ok(n) => n.unwrap_or(0),
        Err(e) => return bad(e),
    };
    let active = match q.active.as_deref() {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => return bad("active must be true or false".to_owned()),
    };
    state
        .run_blocking(move |state| {
            let now_ms = epoch_ms();
            let snap = match snapshot(state, now_ms) {
                Ok(s) => s,
                Err(e) => return unavailable("dynamic thresholds not readable", &e),
            };
            let picked: Vec<&Learned> = snap
                .rows
                .iter()
                .filter(|l| active.is_none_or(|a| l.active == a))
                .collect();
            let data: Vec<Value> = picked
                .iter()
                .skip(offset)
                .take(limit)
                .map(|l| row_json(state, l))
                .collect();
            (
                StatusCode::OK,
                Json(json!({
                    "enabled": snap.config.map(|c| c.enabled),
                    "config": config_json(snap.config),
                    "total": picked.len(),
                    "limit": limit,
                    "offset": offset,
                    "data": data,
                })),
            )
        })
        .await
}

/// `GET /api/v2/dynamic-thresholds/stats` — counts by level.
async fn stats(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    state
        .run_blocking(move |state| {
            let snap = match snapshot(state, epoch_ms()) {
                Ok(s) => s,
                Err(e) => return unavailable("dynamic thresholds not readable", &e),
            };
            let active: Vec<&Learned> = snap.rows.iter().filter(|l| l.active).collect();
            let at_max = active
                .iter()
                .filter(|l| l.row.level >= birdnet_core::detection::dynamic_threshold::MAX_LEVEL)
                .count();
            let distribution: Vec<Value> = (0
                ..=birdnet_core::detection::dynamic_threshold::MAX_LEVEL)
                .map(|level| {
                    json!({
                        "level": level,
                        "count": active.iter().filter(|l| l.row.level == level).count(),
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({
                    "enabled": snap.config.map(|c| c.enabled),
                    "config": config_json(snap.config),
                    "total_count": snap.rows.len(),
                    "active_count": active.len(),
                    "at_maximum_count": at_max,
                    "level_distribution": distribution,
                })),
            )
        })
        .await
}

/// `GET /api/v2/dynamic-thresholds/{species}` — one species, by scientific name.
async fn one(
    State(state): State<AppState>,
    Path(species): Path<String>,
) -> (StatusCode, Json<Value>) {
    state
        .run_blocking(move |state| {
            let snap = match snapshot(state, epoch_ms()) {
                Ok(s) => s,
                Err(e) => return unavailable("dynamic thresholds not readable", &e),
            };
            let found = snap
                .rows
                .iter()
                .find(|l| l.row.sci_name.eq_ignore_ascii_case(species.trim()));
            match found {
                Some(l) => (StatusCode::OK, Json(row_json(state, l))),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "error": "nothing is stored for that species; it has no learned level \
                                  (or the level was reset). Species are named by scientific name."
                    })),
                ),
            }
        })
        .await
}

/// The wall clock, in epoch milliseconds.
fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
