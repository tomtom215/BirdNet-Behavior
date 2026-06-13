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
        let recording = state.with_db(|conn| {
            crate::routes::pages::today::capture_outage(conn).map(|(_, last)| last)
        });
        let recording_pill = recording.map_or_else(
            || {
                r#"<span class="bnb-pill moss"><span class="bnb-dot live"></span> recording</span>"#
                    .to_string()
            },
            |last| {
                format!(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> stopped · {last}</span>"#
                )
            },
        );

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

async fn health_badge_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
    })
    .await;

    // `pill` drives the visual tone; `state` is a stable machine-readable token.
    let (pill, dot, label, state_token) = match result {
        Ok(true) => ("moss", "live", "Healthy", "ok"),
        Ok(false) => ("dawn", "dawn", "Degraded", "warn"),
        Err(_) => ("rare", "rare", "Error", "err"),
    };

    let html = format!(
        r#"<span class="bnb-pill {pill}" data-health="{state_token}"><span class="bnb-dot {dot}"></span> {label}</span>"#
    );
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
