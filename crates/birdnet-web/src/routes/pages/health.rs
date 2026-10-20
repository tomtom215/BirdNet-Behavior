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
        if let Some(temp) = crate::system_info::cpu_temperature() {
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

/// HTMX partial: the header badge, on every page, refreshed every 30 s.
///
/// For a non-technical operator this badge *is* the answer to "is my station
/// working?". It reads the same issue list as the banner on the Station Health
/// tab ([`super::station_issues`]), so the header can never say Healthy while
/// the page it links to says Needs attention. It used to grade three things
/// of its own, and a station with one microphone up and two cameras stalled
/// read green.
///
/// # What it costs
///
/// Every page load and every 30 s per open tab. The facts are a handful of
/// small indexed reads, two `statvfs` calls and some in-memory gauges; the
/// integrity verdict is the one the daily maintenance pass recorded, never a
/// fresh scan (see [`super::station_issues::recorded_integrity`]).
async fn health_badge_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let graded = tokio::task::spawn_blocking(move || {
        let (issues, integrity) = super::station_issues::gather_for_badge(&state);
        super::station_issues::badge(&issues, integrity)
    })
    .await;

    let badge = graded.unwrap_or_else(|_| super::station_issues::Badge {
        pill: "rare",
        dot: "rare",
        label: "Error",
        token: "err",
        title: "the health check itself failed".to_owned(),
    });

    // A link, not a span. The badge names a problem on every page of the site
    // and, until 0.15.0, that was all it did: the reason lived in a `title`,
    // which no phone and no tablet can show, and the one screen that explains
    // the problem had to be found by guesswork. `/station` is the public Health
    // tab, so this is reachable for a viewer as well as an admin.
    //
    // `aria-label` carries the reason too, because a `title` on a control is
    // not a reliable accessible name and a screen-reader user was hearing the
    // same two words with none of the explanation.
    let html = super::station_issues::badge_html(&badge);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
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
        // This tile sits on the health page, which is read by the person who
        // owns the station rather than the one who compiles it. A start-up
        // flag and a build flag are not things they can act on, and naming
        // them reads as a fault they have caused.
        "The extra behaviour insights are available but not switched on. \
         Everything else keeps recording as normal."
    } else {
        "This build does not include the extra behaviour insights. Everything \
         else keeps recording as normal."
    };

    let html = format!(
        r#"<div class="value"><span class="dot {css_class}"></span> {status}</div>
<div class="label">Analytics Engine</div>
<p class="he-hint">{hint}</p>"#,
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
