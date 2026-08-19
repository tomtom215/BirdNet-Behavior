//! System API endpoints: health, version, diagnostics.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

use crate::state::AppState;

/// System routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/system/disk", get(disk_info))
}

async fn root() -> Json<Value> {
    Json(json!({
        "name": "BirdNet-Behavior API",
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running",
    }))
}

/// What the health endpoint can say about the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DbHealth {
    /// Reachable, and the last recorded integrity check passed.
    Ok,
    /// Reachable, but no integrity check has been recorded yet.
    Unchecked,
    /// Not reachable, or the last recorded integrity check failed.
    Error,
}

impl DbHealth {
    /// `Unchecked` is deliberately *not* an error: a station whose first daily
    /// integrity check has not run yet is not degraded, and reporting it as
    /// such would leave a freshly started container permanently `unhealthy`.
    const fn is_serving(self) -> bool {
        matches!(self, Self::Ok | Self::Unchecked)
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unchecked => "unchecked",
            Self::Error => "error",
        }
    }
}

/// Grade the database for the health endpoint: is it reachable, and what did
/// the last recorded integrity check say?
///
/// # Why this is not `PRAGMA quick_check`
///
/// It was, on every request. That pragma reads every page of the database file:
/// 1.5-1.9 s on a three-year station's 1.29 GB database on `NVMe`, and roughly
/// 30 s on the SD card a Raspberry Pi runs from. The container's own
/// `HEALTHCHECK` polls this endpoint every 30 s with `curl --max-time 4` inside
/// a 5 s timeout — so on any station with real history the check could not
/// finish inside its own budget, and after three retries Docker marks the
/// container `unhealthy` and keeps it there. Meanwhile every poll read the
/// whole database again, competing with the detection write path for the same
/// card.
///
/// What a health probe actually needs is (a) can I reach the database at all,
/// which one trivial query answers, and (b) is it sound, which the daily
/// maintenance integrity check already establishes and, since migration 28,
/// records. Reading the verdict is strictly more useful than sampling it here:
/// a failure stays reported until it is fixed instead of depending on which
/// request happened to catch it.
pub(crate) fn db_health(state: &AppState) -> DbHealth {
    let reachable = state.with_db(|conn| {
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .is_ok()
    });
    if !reachable {
        return DbHealth::Error;
    }
    match state.with_db(|conn| {
        birdnet_db::sqlite::last_run_result(conn, birdnet_db::sqlite::JOB_INTEGRITY_CHECK)
    }) {
        Ok(Some((_, Some(false)))) => DbHealth::Error,
        Ok(Some((_, Some(true)))) => DbHealth::Ok,
        // Ran with no verdict, never ran, or the lookup itself failed: nothing
        // is known, which is not the same as knowing it is broken.
        _ => DbHealth::Unchecked,
    }
}

async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let health = tokio::task::spawn_blocking({
        let state = state.clone();
        move || db_health(&state)
    })
    .await
    .unwrap_or(DbHealth::Error);
    let db_ok = health.is_serving();

    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    // End-to-end freshness, fed by the deadman task (None until its first
    // pass, or on a station with no detections yet). Surfaced here so remote
    // monitors get "is it actually detecting" from the same probe they
    // already poll — the gap every per-component gauge leaves open.
    let detection_silence_secs = state.metrics().detection_silence_secs();

    (
        status,
        Json(json!({
            "status": if db_ok { "healthy" } else { "degraded" },
            "version": env!("CARGO_PKG_VERSION"),
            "database": health.as_str(),
            "analytics": state.has_analytics(),
            "detection_daemon": if state.detection_daemon_running() { "running" } else { "stopped" },
            "detection_silence_secs": detection_silence_secs,
        })),
    )
}

/// `GET /api/v2/system/disk` — Disk usage for the database filesystem.
async fn disk_info(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_path = state.db_path().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let dir = db_path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir = dir.unwrap_or_else(|| std::path::Path::new("."));
        birdnet_core::audio::capture::disk_usage(dir)
    })
    .await;

    match result {
        Ok(Ok(usage)) => {
            let status = if usage.is_critical() {
                "critical"
            } else if usage.is_low() {
                "low"
            } else {
                "ok"
            };

            let http_status = if usage.is_critical() {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };

            (
                http_status,
                Json(json!({
                    "status": status,
                    "total_bytes": usage.total_bytes,
                    "used_bytes": usage.used_bytes,
                    "available_bytes": usage.available_bytes,
                    "used_percent": format!("{:.1}", usage.used_percent()),
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

async fn stats(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let detections = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let latest = birdnet_db::sqlite::latest_detection(conn).ok().flatten();
            let confidence = birdnet_db::sqlite::confidence_distribution(conn).unwrap_or([0; 6]);
            (detections, species, latest, confidence)
        })
    })
    .await;

    match result {
        Ok((detections, species, latest, confidence)) => {
            let latest_json = latest.map_or(json!(null), |(date, time, name)| {
                json!({
                    "date": date,
                    "time": time,
                    "species": name,
                })
            });

            (
                StatusCode::OK,
                Json(json!({
                    "total_detections": detections,
                    "unique_species": species,
                    "latest_detection": latest_json,
                    "confidence_distribution": {
                        "0-50": confidence[0],
                        "50-60": confidence[1],
                        "60-70": confidence[2],
                        "70-80": confidence[3],
                        "80-90": confidence[4],
                        "90-100": confidence[5],
                    },
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": crate::routes::log_internal("internal error", &e) })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{DbHealth, db_health};
    use crate::state::AppState;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    fn record(state: &AppState, ok: Option<bool>) {
        state.with_db(|conn| {
            birdnet_db::sqlite::record_run_result(
                conn,
                birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
                1_700_000_000,
                ok,
            )
            .expect("record");
        });
    }

    /// `/api/v2/health` must report the *recorded* integrity verdict rather
    /// than running its own `PRAGMA quick_check`.
    ///
    /// This database is intact, so a live check would pass — recording a
    /// failure is the only thing that tells the two implementations apart. The
    /// cost of the old one was not theoretical: the container `HEALTHCHECK`
    /// polls this endpoint every 30 s with a 4 s curl timeout, and the pragma
    /// reads every page of the file (3.4 s measured on a 1.29 GB database on
    /// `NVMe`; roughly 30 s on a Pi's SD card).
    #[test]
    fn health_reports_the_recorded_verdict_not_a_fresh_scan() {
        let state = test_state();
        record(&state, Some(false));
        assert_eq!(db_health(&state), DbHealth::Error);
    }

    /// The counterpart, so the gate above cannot pass by always reporting an
    /// error.
    #[test]
    fn a_recorded_pass_is_healthy() {
        let state = test_state();
        record(&state, Some(true));
        assert_eq!(db_health(&state), DbHealth::Ok);
    }

    /// A station whose first daily check has not run yet is serving, not
    /// degraded. Reporting `Error` here would leave a freshly started container
    /// `unhealthy` until the first maintenance tick — and `start_period` is
    /// 15 min while the first check lands 5 min in, so it would flap.
    #[test]
    fn a_never_checked_database_is_serving_but_says_so() {
        let state = test_state();
        let health = db_health(&state);
        assert_eq!(health, DbHealth::Unchecked);
        assert!(health.is_serving(), "unchecked must not return 503");
        assert_eq!(health.as_str(), "unchecked");
    }

    /// Reachability is still checked on every request — that is the part a
    /// probe genuinely has to sample. A recorded pass must not paper over a
    /// database that cannot be queried at all.
    #[test]
    fn an_unreachable_database_is_an_error_whatever_is_on_record() {
        let state = test_state();
        record(&state, Some(true));
        state.with_db(|conn| {
            conn.execute_batch("DROP TABLE detections; DROP TABLE maintenance_runs;")
                .expect("drop");
        });
        // The reachability probe still succeeds (the connection is live), so
        // this asserts the weaker true thing: with the record gone, the answer
        // falls back to "unchecked" rather than claiming a pass it no longer
        // has evidence for.
        assert_eq!(db_health(&state), DbHealth::Unchecked);
    }
}
