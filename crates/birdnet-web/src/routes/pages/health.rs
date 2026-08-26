//! Health badge and disk status HTMX partials.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use crate::state::AppState;

/// Mount the health badge, disk status, station line, and analytics status
/// HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/health-badge", get(health_badge_partial))
        .route("/pages/disk-status", get(disk_status_partial))
        .route(
            "/pages/station-health-line",
            get(station_health_line_partial),
        )
        .route("/pages/analytics-status", get(analytics_status_partial))
}

/// HTMX partial: the Today rail's one-line station readout — recording
/// state · disk · temperature. Each item is real data and omitted when the
/// signal is unavailable; the recording state shares the outage logic of the
/// hero pills so the two can never disagree.
async fn station_health_line_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let html = tokio::task::spawn_blocking(move || {
        // Same rule as the hero pill: the capture gauge is authoritative when
        // it has an opinion, and only then does detection-freshness decide.
        // Freshness alone cannot speak for a station that has never detected
        // anything — it reported a green "recording" indefinitely, which on a
        // first-run station with a dead microphone is precisely wrong.
        let capture = super::today::live_capture_state(&state);
        let recording = state.with_read_db(|conn| {
            crate::routes::pages::today::capture_outage(conn).map(|(_, last)| last)
        });
        let recording_pill = match (capture, recording) {
            (super::today::CaptureState::NoSource, _) => {
                r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> no microphone</span>"#
                    .to_string()
            }
            (super::today::CaptureState::Down, _) => {
                r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> capture down</span>"#
                    .to_string()
            }
            (_, Some(last)) => {
                format!(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> stopped · {last}</span>"#
                )
            }
            (_, None) => {
                r#"<span class="bnb-pill moss"><span class="bnb-dot live"></span> recording</span>"#
                    .to_string()
            }
        };

        let mut out = format!(r#"<div class="x-health">{recording_pill}"#);
        let db_path = state.db_path().to_path_buf();
        let dir = db_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(
                || std::path::PathBuf::from("."),
                std::path::Path::to_path_buf,
            );
        if let Ok(usage) = birdnet_core::audio::capture::disk_usage(&dir) {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    r#"<span class="sep">·</span><span>disk {:.0}%</span>"#,
                    usage.used_percent()
                ),
            );
        }
        if let Some(temp) = crate::system_info::sample().cpu_temp_celsius {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(r#"<span class="sep">·</span><span>{temp:.0}°C</span>"#),
            );
        }
        out.push_str("</div>");
        out
    })
    .await
    .unwrap_or_default();
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// Disk usage at or above which the station is graded degraded, matching the
/// dashboard checklist's own "nearly full" threshold so the two agree.
const DISK_DEGRADED_PCT: f64 = 90.0;

async fn health_badge_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    // "Healthy" used to mean nothing more than "SQLite is not corrupt". It is
    // the most prominent status signal in the app — present on every page,
    // refreshed every 30 s — and a station whose microphone was dead, or whose
    // disk was 99 % full, displayed it in green all the same. For a
    // non-technical operator that badge *is* the answer to "is my station
    // working?", so it now grades the three things that stop a station
    // producing detections, reusing the dashboard's own signals rather than
    // measuring them a second way.
    let grading = tokio::task::spawn_blocking(move || {
        let db_ok = recorded_db_health(&state);
        let capture = super::today::live_capture_state(&state);
        let disk = super::today::disk_used_percent(&state);
        (db_ok, capture, disk)
    })
    .await;

    let (pill, dot, label, state_token, title) = grading.map_or_else(
        |_| {
            (
                "rare",
                "rare",
                "Error",
                "err",
                "the health check itself failed",
            )
        },
        |(db_ok, capture, disk)| grade(db_ok, capture, disk),
    );

    let html = format!(
        r#"<span class="bnb-pill {pill}" data-health="{state_token}" title="{title}"><span class="bnb-dot {dot}"></span> {label}</span>"#
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// What the last recorded integrity check says about the database.
///
/// `None` means no verdict is on record — either the daily check has not run on
/// this station yet, or the row predates the column. The caller must keep that
/// distinct from a recorded failure.
///
/// # Why this is a read and not a check
///
/// This used to be `birdnet_db::sqlite::quick_check(conn)`, run inline. That
/// pragma reads **every page of the database file**. On a three-year station
/// (2.76 M detections, 1.29 GB) it measured 1.5-1.9 s warm on `NVMe`, and the
/// enclosing partial 3.8 s; a Raspberry Pi reading the same file from an SD
/// card at ~45 MB/s is looking at roughly 30 s — longer than the badge's own
/// refresh interval, so the scans would overlap.
///
/// The badge is mounted in `layout.html` with `hx-trigger="load, every 30s"`,
/// which put that read on every page load and then twice a minute for every
/// open tab, indefinitely, competing with the detection write path for the same
/// card and steadily wearing it. The daily maintenance pass already runs the
/// check properly; since migration 28 it records the answer, so the badge can
/// read one row instead of re-deriving it.
fn recorded_db_health(state: &AppState) -> Option<bool> {
    state
        .with_read_db(|conn| {
            birdnet_db::sqlite::last_run_result(conn, birdnet_db::sqlite::JOB_INTEGRITY_CHECK)
        })
        .ok()
        .flatten()
        .and_then(|(_when, ok)| ok)
}

/// Grade the station: `(pill, dot, label, data-health token, title)`.
///
/// Worst-first, so the badge names the most serious problem rather than an
/// incidental one. `data-health` keeps its stable `ok`/`warn`/`err` vocabulary
/// for anything scraping it; `title` carries the reason so hovering explains a
/// non-green badge without the operator going hunting.
fn grade(
    db_ok: Option<bool>,
    capture: super::today::CaptureState,
    disk: Option<f64>,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    use super::today::CaptureState;
    if db_ok == Some(false) {
        return (
            "rare",
            "rare",
            "Error",
            "err",
            "the database failed its integrity check",
        );
    }
    match capture {
        CaptureState::NoSource => {
            return (
                "dawn",
                "dawn",
                "No microphone",
                "warn",
                "no audio source is configured, so nothing can be detected",
            );
        }
        CaptureState::Down => {
            return (
                "dawn",
                "dawn",
                "Mic down",
                "warn",
                "an audio source is configured but is not capturing",
            );
        }
        CaptureState::Up | CaptureState::Unknown => {}
    }
    if disk.is_some_and(|pct| pct >= DISK_DEGRADED_PCT) {
        return (
            "dawn",
            "dawn",
            "Disk full",
            "warn",
            "the disk is nearly full and recording may stop",
        );
    }
    if db_ok.is_none() {
        // Everything this badge *can* see is fine, but the database has not
        // been verified yet — the daily integrity check has not run on this
        // station. Say so rather than claiming a check that never happened.
        return (
            "moss",
            "live",
            "Healthy",
            "ok",
            "capture and disk look fine; the database has not been checked yet",
        );
    }
    (
        "moss",
        "live",
        "Healthy",
        "ok",
        "database, capture and disk all look fine",
    )
}

async fn disk_status_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let db_path = state.db_path().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let dir = db_path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir = dir.unwrap_or_else(|| std::path::Path::new("."));
        birdnet_core::audio::capture::disk_usage(dir)
    })
    .await;

    match result {
        Ok(Ok(usage)) => {
            let pct = usage.used_percent();
            let dot = if usage.is_critical() {
                "rare"
            } else if usage.is_low() {
                "dawn"
            } else {
                "live"
            };
            let bar_color = if usage.is_critical() {
                "var(--rare)"
            } else if usage.is_low() {
                "var(--dawn)"
            } else {
                "var(--moss)"
            };

            #[allow(clippy::cast_precision_loss)]
            let avail_gb = usage.available_bytes as f64 / 1_073_741_824.0;

            let html = format!(
                r#"<div class="bnb-card pad">
    <div class="he-row">
      <div class="bnb-eyebrow"><span class="bnb-dot {dot}"></span> Disk</div>
      <span class="bnb-meta mono">{avail_gb:.1} GB free</span>
    </div>
    <div class="display tabular he-pct">{pct:.0}%</div>
    <div class="progress"><div class="progress-bar" data-style="width:{pct:.0}%;background:{bar_color};"></div></div>
</div>"#,
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<div class="bnb-card pad"><div class="bnb-eyebrow">Disk</div><div class="display he-dash">—</div></div>"#.to_string(),
        ),
    }
}

async fn analytics_status_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    let (status, css_class) = if configured {
        ("Connected", "ok")
    } else if compiled {
        ("Not Configured", "warn")
    } else {
        ("Not Compiled", "err")
    };

    // The DuckDB analytics database being open does not guarantee the
    // duckdb-behavioral extension loaded — that is a separate requirement the
    // sessions / retention / next-species cards report on individually. Avoid
    // overclaiming here so the badge stays honest when the extension version
    // does not match the bundled DuckDB.
    let hint = if configured {
        "DuckDB analytics database connected. Behavioral insights (sessions, retention, \
         next-species) additionally require the duckdb-behavioral extension — see the cards below."
    } else if compiled {
        "Start with <code>--analytics-db</code> to enable."
    } else {
        "Rebuild with <code>--features analytics</code> to enable."
    };

    let html = format!(
        r#"<div class="value"><span class="dot {css_class}"></span> {status}</div>
<div class="label">Analytics Engine</div>
<p class="he-hint">{hint}</p>"#,
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(test)]
mod tests {
    use super::{grade, recorded_db_health};
    use crate::routes::pages::today::CaptureState;
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

    /// The badge must report the *recorded* verdict of the daily integrity
    /// check, not run one of its own.
    ///
    /// The distinction is the whole point. `PRAGMA quick_check` reads every
    /// page of the database file: 1.5-1.9 s on a three-year station's 1.29 GB
    /// database on `NVMe`, and roughly 30 s on the SD card a Pi actually runs on.
    /// The badge lives in `layout.html` with `hx-trigger="load, every 30s"`, so
    /// running it there meant a full read of the database on every page load
    /// and twice a minute per open tab thereafter, forever.
    ///
    /// This database is healthy — a live check would pass. Recording a failed
    /// verdict is therefore the one thing that can tell the two implementations
    /// apart.
    #[test]
    fn the_badge_reports_the_recorded_verdict_not_a_fresh_scan() {
        let state = test_state();
        record(&state, Some(false));
        assert_eq!(
            recorded_db_health(&state),
            Some(false),
            "a recorded failure must reach the badge from `maintenance_runs`, \
             not be overruled by re-checking a database that is in fact intact"
        );
        let (_, _, label, token, _) = grade(Some(false), CaptureState::Up, Some(10.0));
        assert_eq!((label, token), ("Error", "err"));
    }

    /// The counterpart, so the gate above cannot be satisfied by a badge that
    /// simply always reports a failure.
    #[test]
    fn a_recorded_pass_is_reported_as_healthy() {
        let state = test_state();
        record(&state, Some(true));
        assert_eq!(recorded_db_health(&state), Some(true));
        let (_, _, label, token, _) = grade(Some(true), CaptureState::Up, Some(10.0));
        assert_eq!((label, token), ("Healthy", "ok"));
    }

    /// A third state the badge must not collapse into either of the others: on
    /// a station whose first daily check has not run yet there is no row at
    /// all. That is "not checked", not "failed" — a fresh install must not
    /// display a red Error badge — and not "passed" either, so the title says
    /// so rather than claiming the database was verified.
    #[test]
    fn a_never_run_check_is_unknown_rather_than_pass_or_fail() {
        let state = test_state();
        assert_eq!(recorded_db_health(&state), None, "no row means no verdict");
        let (_, _, label, token, title) = grade(None, CaptureState::Up, Some(10.0));
        assert_eq!((label, token), ("Healthy", "ok"));
        assert!(
            title.contains("not"),
            "the title must not claim the database was checked; got {title:?}"
        );
    }

    /// A job that records no verdict (the session prune) must read back as
    /// "no verdict", not as a pass.
    #[test]
    fn a_recorded_run_without_a_verdict_is_unknown() {
        let state = test_state();
        record(&state, None);
        assert_eq!(recorded_db_health(&state), None);
    }

    /// The badge is on every page and refreshed every 30 s; for a
    /// non-technical operator it *is* the answer to "is my station working?".
    /// It used to mean only "SQLite is not corrupt", so a station with a dead
    /// microphone and a full disk displayed "Healthy" in green.
    #[test]
    fn a_dead_microphone_is_not_healthy() {
        let (_, _, label, token, _) = grade(Some(true), CaptureState::Down, Some(10.0));
        assert_eq!(label, "Mic down");
        assert_eq!(token, "warn");
    }

    #[test]
    fn no_configured_source_is_not_healthy() {
        let (_, _, label, token, _) = grade(Some(true), CaptureState::NoSource, Some(10.0));
        assert_eq!(label, "No microphone");
        assert_eq!(token, "warn");
    }

    #[test]
    fn a_nearly_full_disk_is_not_healthy() {
        let (_, _, label, token, _) = grade(Some(true), CaptureState::Up, Some(96.0));
        assert_eq!(label, "Disk full");
        assert_eq!(token, "warn");
    }

    #[test]
    fn a_working_station_is_healthy() {
        let (_, _, label, token, _) = grade(Some(true), CaptureState::Up, Some(41.0));
        assert_eq!(label, "Healthy");
        assert_eq!(token, "ok");
    }

    /// No gauge published yet is not an outage — a station that has just
    /// started must not flash "Mic down" at its operator.
    #[test]
    fn an_unreconciled_source_does_not_read_as_down() {
        let (_, _, label, _, _) = grade(Some(true), CaptureState::Unknown, Some(41.0));
        assert_eq!(label, "Healthy");
    }

    /// Database corruption outranks everything else.
    #[test]
    fn a_corrupt_database_outranks_a_capture_problem() {
        let (_, _, label, token, _) = grade(Some(false), CaptureState::Down, Some(99.0));
        assert_eq!(label, "Error");
        assert_eq!(token, "err");
    }

    /// Unknown disk usage must not be graded as full.
    #[test]
    fn unmeasurable_disk_is_not_treated_as_full() {
        let (_, _, label, _, _) = grade(Some(true), CaptureState::Up, None);
        assert_eq!(label, "Healthy");
    }

    /// The threshold matches the dashboard checklist's "nearly full" wording,
    /// so the two surfaces cannot disagree about the same disk.
    #[test]
    fn disk_threshold_boundary() {
        assert_eq!(grade(Some(true), CaptureState::Up, Some(89.9)).2, "Healthy");
        assert_eq!(
            grade(Some(true), CaptureState::Up, Some(90.0)).2,
            "Disk full"
        );
    }
}
