//! `GET /api/v2/effort` and `GET /api/v2/effort/history` — how long the
//! station listened, and a species' detection history over the hours it did
//! (RES-02).
//!
//! The ledger is audio the detection pipeline analysed, by the local date and
//! hour it was recorded and its source (`analysis_effort`, migration 61; see
//! `src/integrations/effort.rs`). Hours before that migration exist only as
//! daily totals in `recording_effort`.
//!
//! The history is the shape occupancy and hour-of-day analyses start from: one
//! row per local hour with `1` (the species was detected), `0` (the station
//! analysed audio that hour and did not detect it) or `null` (it analysed
//! nothing that hour, so nothing can be said). `null` is not `0`: an hour the
//! station was off is not an hour the bird was absent.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Mount the effort routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/effort", get(effort))
        .route("/effort/history", get(history))
}

/// The longest range one request may ask for, in days.
const MAX_DAYS: i64 = 366;

/// Query for [`effort`].
#[derive(Debug, Deserialize)]
struct EffortQuery {
    from: String,
    to: String,
    /// `hour` or `day` (the default).
    grain: Option<String>,
}

/// Query for [`history`].
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    sci_name: String,
    from: String,
    to: String,
}

/// The day number of a `YYYY-MM-DD` date, or `None`.
fn day_number(date: &str) -> Option<i64> {
    let c = birdnet_core::civil::parse_civil(date, "00:00:00")?;
    (birdnet_core::civil::date_string_from_days(birdnet_core::civil::days_from_civil(
        c.year, c.month, c.day,
    )) == date)
        .then(|| birdnet_core::civil::days_from_civil(c.year, c.month, c.day))
}

/// The range as day numbers, or the reason it is refused.
fn range(from: &str, to: &str) -> Result<(i64, i64), String> {
    let (Some(a), Some(b)) = (day_number(from), day_number(to)) else {
        return Err("from and to must be dates written YYYY-MM-DD".to_owned());
    };
    if b < a {
        return Err("to is before from".to_owned());
    }
    if b - a + 1 > MAX_DAYS {
        return Err(format!("at most {MAX_DAYS} days at a time"));
    }
    Ok((a, b))
}

fn bad_request(why: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": why }))).into_response()
}

fn internal<E: std::fmt::Display>(e: &E) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": crate::routes::log_internal("internal error", e) })),
    )
        .into_response()
}

/// `GET /api/v2/effort?from=&to=&grain=hour|day`.
async fn effort(State(state): State<AppState>, Query(q): Query<EffortQuery>) -> impl IntoResponse {
    if let Err(why) = range(&q.from, &q.to) {
        return bad_request(&why);
    }
    let hourly = match q.grain.as_deref() {
        None | Some("day") => false,
        Some("hour") => true,
        Some(_) => return bad_request("grain is hour or day"),
    };
    let (from, to) = (q.from, q.to);
    let rows = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| -> Result<Vec<Value>, birdnet_db::sqlite::DbError> {
                if hourly {
                    let rows = birdnet_db::sqlite::hourly_effort(conn, &from, &to)?;
                    return Ok(rows
                        .iter()
                        .map(|h| {
                            json!({
                                "date": h.date, "hour": h.hour, "source": h.source,
                                "seconds_analysed": h.seconds_analysed,
                                "segments_analysed": h.segments_analysed,
                                "segments_shed": h.segments_shed,
                                "segments_dropped": h.segments_dropped,
                            })
                        })
                        .collect());
                }
                let mut stmt = conn.prepare(
                    "SELECT e.date, e.source, e.seconds,
                            SUM(a.segments_analysed), SUM(a.segments_shed), SUM(a.segments_dropped)
                       FROM recording_effort e
                       LEFT JOIN analysis_effort a ON a.date = e.date AND a.source = e.source
                      WHERE e.date BETWEEN ?1 AND ?2
                      GROUP BY e.date, e.source
                      ORDER BY e.date, e.source",
                )?;
                let rows = stmt.query_map(rusqlite::params![from, to], |r| {
                    Ok(json!({
                        "date": r.get::<_, String>(0)?,
                        "source": r.get::<_, String>(1)?,
                        "seconds_analysed": r.get::<_, f64>(2)?,
                        "segments_analysed": r.get::<_, Option<i64>>(3)?,
                        "segments_shed": r.get::<_, Option<i64>>(4)?,
                        "segments_dropped": r.get::<_, Option<i64>>(5)?,
                    }))
                })?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
        })
        .await;
    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({ "grain": if hourly { "hour" } else { "day" }, "rows": rows })),
        )
            .into_response(),
        Err(e) => internal(&e),
    }
}

/// `GET /api/v2/effort/history?sci_name=&from=&to=` — one row per local hour.
async fn history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> impl IntoResponse {
    let (a, b) = match range(&q.from, &q.to) {
        Ok(r) => r,
        Err(why) => return bad_request(&why),
    };
    if q.sci_name.trim().is_empty() {
        return bad_request("sci_name is required");
    }
    let (from, to, sci) = (q.from, q.to, q.sci_name);
    let read = state
        .run_blocking(move |s| {
            s.with_read_db(|conn| -> Result<_, birdnet_db::sqlite::DbError> {
                // Seconds analysed per (date, hour), all sources together.
                // From the hourly ledger only: a day known just as a daily
                // total (before migration 61) cannot say which hours were
                // heard, so its hours are null rather than guessed.
                let mut listened = std::collections::HashMap::<(String, u8), f64>::new();
                for h in birdnet_db::sqlite::hourly_effort(conn, &from, &to)? {
                    *listened.entry((h.date, h.hour)).or_default() += h.seconds_analysed;
                }
                let mut detected = std::collections::HashSet::<(String, u8)>::new();
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT Date, CAST(substr(Time, 1, 2) AS INTEGER) FROM detections
                      WHERE Sci_Name = ?1 AND Date BETWEEN ?2 AND ?3",
                )?;
                let hits = stmt.query_map(rusqlite::params![sci, from, to], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, u8>(1)?))
                })?;
                for hit in hits {
                    detected.insert(hit?);
                }
                Ok((listened, detected))
            })
        })
        .await;
    let (listened, detected) = match read {
        Ok(v) => v,
        Err(e) => return internal(&e),
    };
    let mut rows = Vec::new();
    for day in a..=b {
        let date = birdnet_core::civil::date_string_from_days(day);
        for hour in 0..24_u8 {
            let key = (date.clone(), hour);
            let seconds = listened.get(&key).copied().unwrap_or(0.0);
            let value = if seconds <= 0.0 {
                Value::Null
            } else {
                json!(u8::from(detected.contains(&key)))
            };
            rows.push(json!({
                "date": date, "hour": hour, "seconds_analysed": seconds, "detected": value,
            }));
        }
    }
    (StatusCode::OK, Json(json!({ "occasions": rows }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(state: AppState, uri: &str) -> (StatusCode, Value) {
        use tower::ServiceExt as _;
        let app = Router::new().nest("/api/v2", router()).with_state(state);
        let res = app
            .oneshot(
                axum::http::Request::get(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    /// RES-02's gate. The source was down 05:00-07:00 on 1 June: the station
    /// analysed audio at 04:00, 07:00 and 08:00, and detected the species at
    /// 07:10. The history says 0, null, null, 1, 0 for those five hours —
    /// null where nothing was analysed, so the outage is not read as an
    /// absence. A daily ledger cannot say which hours were heard.
    #[tokio::test]
    async fn the_history_is_null_for_hours_the_station_did_not_analyse() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        for hour in [4_u8, 7, 8] {
            birdnet_db::sqlite::add_hourly_effort(
                &conn,
                &birdnet_db::sqlite::HourlyEffort {
                    date: "2026-06-01".into(),
                    hour,
                    source: "local".into(),
                    seconds_analysed: 3600.0,
                    segments_analysed: 240,
                    segments_shed: 0,
                    segments_dropped: 0,
                },
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-06-01', '07:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .unwrap();
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));

        let (status, body) = get(
            state.clone(),
            "/api/v2/effort/history?sci_name=Turdus%20merula&from=2026-06-01&to=2026-06-01",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let occasions = body["occasions"].as_array().expect("occasions");
        assert_eq!(occasions.len(), 24);
        let detected: Vec<Value> = (4..=8).map(|h| occasions[h]["detected"].clone()).collect();
        assert_eq!(
            detected,
            vec![json!(0), Value::Null, Value::Null, json!(1), json!(0)],
            "{occasions:?}"
        );

        // The counterpart: the hourly rows are what /effort?grain=hour returns.
        let (status, body) = get(
            state,
            "/api/v2/effort?from=2026-06-01&to=2026-06-01&grain=hour",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let hours: Vec<u64> = body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["hour"].as_u64().unwrap())
            .collect();
        assert_eq!(hours, vec![4, 7, 8]);
    }

    #[tokio::test]
    async fn a_malformed_range_is_refused() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        for uri in [
            "/api/v2/effort?from=2026-6-1&to=2026-06-02",
            "/api/v2/effort?from=2026-06-02&to=2026-06-01",
            "/api/v2/effort?from=2024-01-01&to=2026-01-01",
            "/api/v2/effort?from=2026-06-01&to=2026-06-01&grain=week",
        ] {
            let (status, _) = get(state.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        }
    }
}
