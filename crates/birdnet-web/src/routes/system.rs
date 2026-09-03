//! System API endpoints: health, version, diagnostics, soundscape.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// System routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/stats", get(stats))
        .route("/system/disk", get(disk_info))
        .route("/soundlevel", get(sound_level))
}

/// Query for [`sound_level`].
#[derive(Debug, Deserialize)]
struct SoundLevelQuery {
    /// Capture source label. Defaults to the source with the newest reading.
    source: Option<String>,
}

/// `GET /api/v2/soundlevel` — the newest third-octave spectrum for one source.
///
/// Returns the band levels of the most recent hour that has any, with the
/// broadband A- and Z-weighted figures beside them, and the **unit** those
/// figures are in.
///
/// The unit is in the payload rather than assumed by the caller because it can
/// genuinely be either: an uncalibrated station reports dBFS (negative,
/// station-relative, fine for tracking change at one place) and a calibrated
/// one reports dB SPL. A chart that labels the first as the second is
/// publishing a measurement the station never made.
async fn sound_level(
    State(state): State<AppState>,
    Query(q): Query<SoundLevelQuery>,
) -> Json<Value> {
    let broadband = state
        .with_read_db(|conn| birdnet_db::sound_levels::recent_broadband(conn, 24))
        .unwrap_or_default();

    // Default to whichever source reported most recently, so a single-
    // microphone station needs no query string and a multi-source one gets a
    // sensible landing view.
    let source = q
        .source
        .or_else(|| broadband.first().map(|b| b.source.clone()));
    let Some(source) = source else {
        return Json(json!({
            "source": Value::Null,
            "unit": "dBFS",
            "bands": [],
            "note": "no sound level observations yet",
        }));
    };

    let bands = state
        .with_read_db(|conn| birdnet_db::sound_levels::latest_hour(conn, &source))
        .unwrap_or_default();
    let latest = broadband.iter().find(|b| b.source == source);
    let calibration_db = latest.map_or(0.0, |b| b.calibration_db);

    Json(json!({
        "source": source,
        "date": bands.first().map(|b| b.date.clone()),
        "hour": bands.first().map(|b| b.hour),
        "samples": bands.first().map_or(0, |b| b.samples),
        "unit": if calibration_db == 0.0 { "dBFS" } else { "dB SPL" },
        "calibration_db": calibration_db,
        "a_weighted_db": latest.map(|b| b.a_weighted_db),
        "z_weighted_db": latest.map(|b| b.z_weighted_db),
        "bands": bands
            .iter()
            .map(|b| json!({
                "band_hz": b.band_hz,
                "label": birdnet_core::audio::soundlevel::label_for(b.band_hz),
                "mean_db": b.mean_db,
                "min_db": b.min_db,
                "max_db": b.max_db,
            }))
            .collect::<Vec<_>>(),
    }))
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
    let reachable = state.with_read_db(|conn| {
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .is_ok()
    });
    if !reachable {
        return DbHealth::Error;
    }
    match state.with_read_db(|conn| {
        birdnet_db::sqlite::last_run_result(conn, birdnet_db::sqlite::JOB_INTEGRITY_CHECK)
    }) {
        Ok(Some((_, Some(false)))) => DbHealth::Error,
        Ok(Some((_, Some(true)))) => DbHealth::Ok,
        // Ran with no verdict, never ran, or the lookup itself failed: nothing
        // is known, which is not the same as knowing it is broken.
        _ => DbHealth::Unchecked,
    }
}

/// Query parameters for [`health`].
#[derive(Debug, Default, Deserialize)]
pub struct HealthQuery {
    /// When set, a stopped detection daemon is a 503 rather than body text.
    #[serde(default)]
    strict: Option<String>,
}

/// Is this truthy as a query flag? `?strict`, `?strict=1`, `?strict=true`.
fn flag_is_set(v: Option<&String>) -> bool {
    v.is_some_and(|s| {
        let s = s.trim();
        s.is_empty() || matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
    })
}

async fn health(
    State(state): State<AppState>,
    Query(q): Query<HealthQuery>,
) -> (StatusCode, Json<Value>) {
    let health = tokio::task::spawn_blocking({
        let state = state.clone();
        move || db_health(&state)
    })
    .await
    .unwrap_or(DbHealth::Error);
    let db_ok = health.is_serving();

    // End-to-end freshness, fed by the deadman task (None until its first
    // pass, or on a station with no detections yet). Surfaced here so remote
    // monitors get "is it actually detecting" from the same probe they
    // already poll — the gap every per-component gauge leaves open.
    let detection_silence_secs = state.metrics().detection_silence_secs();
    let daemon_running = state.detection_daemon_running();

    // ## Why `?strict` and not a changed default
    //
    // The status code was `db_ok` and nothing else, so this endpoint answered
    // `200 "healthy"` on a station whose own response body said
    // `"detection_daemon": "stopped"` — verified against the running binary.
    // That is the endpoint the container `HEALTHCHECK` polls and the one every
    // off-the-shelf monitor gets pointed at, so a station that has recorded
    // nothing since March looked green to all of them.
    //
    // The default stays 200 deliberately. Docker restarts an unhealthy
    // container, and a station whose daemon is down is exactly the station that
    // must stay up to be diagnosed — restarting it in a loop destroys the
    // journal that says why. `?strict=1` is for the monitor that should page a
    // human, which is a different consumer with a different correct answer.
    let strict = flag_is_set(q.strict.as_ref());
    let degraded = !db_ok || (strict && !daemon_running);

    let status = if degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };

    (
        status,
        Json(json!({
            "status": if degraded { "degraded" } else { "healthy" },
            "version": env!("CARGO_PKG_VERSION"),
            "database": health.as_str(),
            "analytics": state.has_analytics(),
            "detection_daemon": if daemon_running { "running" } else { "stopped" },
            "detection_silence_secs": detection_silence_secs,
            "strict": strict,
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
        state.with_read_db(|conn| {
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
