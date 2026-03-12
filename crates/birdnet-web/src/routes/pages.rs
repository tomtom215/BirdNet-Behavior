//! HTMX page and partial routes.
//!
//! Serves full HTML pages (dashboard, species) and HTMX partials
//! (stats, detection table, species list, health badge) that are
//! fetched dynamically for live updates.

use std::fmt::Write;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};

use crate::state::AppState;

// Embedded HTML templates (compiled into the binary).
const LAYOUT_HTML: &str = include_str!("../../templates/layout.html");
const DASHBOARD_HTML: &str = include_str!("../../templates/dashboard.html");
const SPECIES_PAGE_HTML: &str = include_str!("../../templates/species.html");
const ANALYTICS_PAGE_HTML: &str = include_str!("../../templates/analytics.html");

/// Page and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Full pages
        .route("/", get(dashboard_page))
        .route("/species", get(species_page))
        .route("/analytics", get(analytics_page))
        // HTMX partials
        .route("/pages/stats", get(stats_partial))
        .route("/pages/detections", get(detections_partial))
        .route("/pages/top-species", get(top_species_partial))
        .route("/pages/species-list", get(species_list_partial))
        .route("/pages/health-badge", get(health_badge_partial))
        .route("/pages/analytics-status", get(analytics_status_partial))
        .route("/pages/analytics-sessions", get(analytics_sessions_partial))
        .route(
            "/pages/analytics-retention",
            get(analytics_retention_partial),
        )
        .route("/pages/analytics-next", get(analytics_next_partial))
        .route("/pages/analytics-config", get(analytics_config_partial))
        // Dashboard charts
        .route("/pages/hourly-chart", get(hourly_chart_partial))
        .route("/pages/daily-chart", get(daily_chart_partial))
}

/// Render a full page by inserting content into the layout template.
fn render_page(title: &str, content: &str, active_nav: &str) -> Html<String> {
    let version = env!("CARGO_PKG_VERSION");
    let html = LAYOUT_HTML
        .replace("{{title}}", title)
        .replace("{{content}}", content)
        .replace("{{version}}", version)
        .replace(
            "{{nav_dashboard}}",
            if active_nav == "dashboard" {
                "active"
            } else {
                ""
            },
        )
        .replace(
            "{{nav_species}}",
            if active_nav == "species" {
                "active"
            } else {
                ""
            },
        )
        .replace(
            "{{nav_analytics}}",
            if active_nav == "analytics" {
                "active"
            } else {
                ""
            },
        );
    Html(html)
}

/// Dashboard page (full HTML).
async fn dashboard_page() -> Html<String> {
    render_page("Dashboard", DASHBOARD_HTML, "dashboard")
}

/// Species page (full HTML).
async fn species_page() -> Html<String> {
    render_page("Species", SPECIES_PAGE_HTML, "species")
}

/// Analytics page (full HTML).
async fn analytics_page() -> Html<String> {
    render_page("Analytics", ANALYTICS_PAGE_HTML, "analytics")
}

/// HTMX partial: stats cards.
async fn stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let today = today_count(conn);
            (total, species, today)
        })
    })
    .await;

    match result {
        Ok((total, species, today)) => {
            let html = format!(
                r#"<div class="stat-card">
    <div class="value">{total}</div>
    <div class="label">Total Detections</div>
</div>
<div class="stat-card">
    <div class="value">{species}</div>
    <div class="label">Unique Species</div>
</div>
<div class="stat-card">
    <div class="value">{today}</div>
    <div class="label">Today</div>
</div>"#,
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading stats</p>".to_string(),
        ),
    }
}

/// HTMX partial: recent detections table.
async fn detections_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::recent_detections(conn, 20))
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            let mut html = String::from(
                r"<table>
<thead><tr><th>Species</th><th>Confidence</th><th>Time</th><th>Date</th></tr></thead>
<tbody>",
            );

            for d in &detections {
                let conf_pct = d.confidence * 100.0;
                let conf_class = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr>
    <td class="species-name">{com_name}</td>
    <td><span class="conf {conf_class}">{conf_pct:.0}%</span></td>
    <td>{time}</td>
    <td>{date}</td>
</tr>"#,
                    com_name = escape_html(&d.com_name),
                    time = escape_html(&d.time),
                    date = escape_html(&d.date),
                );
            }

            html.push_str("</tbody></table>");

            if detections.is_empty() {
                html = "<p style=\"color: var(--text-muted)\">No detections yet.</p>".to_string();
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading detections</p>".to_string(),
        ),
    }
}

/// HTMX partial: top species sidebar.
async fn top_species_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 10))
    })
    .await;

    match result {
        Ok(Ok(species)) => {
            let mut html = String::new();

            for s in &species {
                let _ = write!(
                    html,
                    r#"<div class="species-item">
    <span class="species-name">{name}</span>
    <span class="species-count">{count}</span>
</div>"#,
                    name = escape_html(&s.com_name),
                    count = s.count,
                );
            }

            if species.is_empty() {
                html = "<p style=\"color: var(--text-muted)\">No species detected yet.</p>"
                    .to_string();
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading species</p>".to_string(),
        ),
    }
}

/// HTMX partial: full species list with confidence stats.
async fn species_list_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 500))
    })
    .await;

    match result {
        Ok(Ok(species)) => {
            let mut html = String::from(
                r"<table>
<thead><tr><th>Species</th><th>Detections</th><th>Avg Confidence</th></tr></thead>
<tbody>",
            );

            for s in &species {
                let conf_pct = s.avg_confidence * 100.0;
                let conf_class = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr>
    <td class="species-name">{name}</td>
    <td>{count}</td>
    <td><span class="conf {conf_class}">{conf_pct:.0}%</span></td>
</tr>"#,
                    name = escape_html(&s.com_name),
                    count = s.count,
                );
            }

            html.push_str("</tbody></table>");

            if species.is_empty() {
                html = "<p style=\"color: var(--text-muted)\">No species detected yet.</p>"
                    .to_string();
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading species list</p>".to_string(),
        ),
    }
}

/// HTMX partial: health badge in navigation.
async fn health_badge_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
    })
    .await;

    let (dot_class, label) = match result {
        Ok(true) => ("ok", "Healthy"),
        Ok(false) => ("err", "Degraded"),
        Err(_) => ("err", "Error"),
    };

    let html = format!(r#"<span class="dot {dot_class}"></span> {label}"#);

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: hourly activity SVG bar chart for today.
async fn hourly_chart_partial(State(state): State<AppState>) -> impl IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &today))
    })
    .await;

    match result {
        Ok(Ok(hours)) => {
            let html = render_hourly_chart(&hours);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

/// HTMX partial: 7-day daily trend SVG bar chart.
async fn daily_chart_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::daily_counts(conn, 7))
    })
    .await;

    match result {
        Ok(Ok(days)) => {
            let html = render_daily_chart(&days);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

/// Render an SVG bar chart showing hourly detection counts.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
fn render_hourly_chart(hours: &[birdnet_db::sqlite::HourlyCount]) -> String {
    // Build a full 24-hour array
    let mut counts = [0_i64; 24];
    for h in hours {
        if let Ok(hour) = h.hour.parse::<usize>() {
            if hour < 24 {
                counts[hour] = h.count;
            }
        }
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);

    if counts.iter().all(|&c| c == 0) {
        return r#"<p style="color: var(--text-muted)">No detections today yet.</p>"#.to_string();
    }

    // SVG dimensions
    let chart_w = 700;
    let chart_h = 120;
    let bar_w = 25;
    let gap = 4;
    let left_pad = 5;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" style="width: 100%; height: auto; display: block;" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 20,
    );

    for (i, &count) in counts.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = if max_count > 0 {
            (count as f64 / max_count as f64 * chart_h as f64) as i32
        } else {
            0
        };
        let y = chart_h - bar_h;

        // Bar color: accent for bars with data, dimmer for zero
        let color = if count > 0 { "#38bdf8" } else { "#1e293b" };

        let _ = write!(
            svg,
            r#"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="{color}"/>"#,
        );

        // Count label above bar (only for non-zero)
        if count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#94a3b8" font-size="9" font-family="sans-serif">{count}</text>"##,
                tx = x + bar_w / 2,
                ty = y - 3,
            );
        }

        // Hour label below (every third hour to avoid crowding)
        if i % 3 == 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#64748b" font-size="9" font-family="sans-serif">{i:02}</text>"##,
                tx = x + bar_w / 2,
                ty = chart_h + 14,
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

/// Render an SVG bar chart showing daily detection counts.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
fn render_daily_chart(days: &[birdnet_db::sqlite::DailyCount]) -> String {
    if days.is_empty() {
        return r#"<p style="color: var(--text-muted)">No detection data yet.</p>"#.to_string();
    }

    let max_count = days.iter().map(|d| d.count).max().unwrap_or(1).max(1);

    let chart_w = 280;
    let chart_h = 100;
    let bar_w = 32;
    let gap = 6;
    let left_pad = 5;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" style="width: 100%; height: auto; display: block;" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 22,
    );

    for (i, day) in days.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (day.count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;

        let _ = write!(
            svg,
            r##"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="#38bdf8"/>"##,
        );

        // Count above bar
        if day.count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#94a3b8" font-size="9" font-family="sans-serif">{count}</text>"##,
                tx = x + bar_w / 2,
                ty = y - 3,
                count = day.count,
            );
        }

        // Date label (MM-DD)
        let date_label = day.date.get(5..).unwrap_or(&day.date);
        let _ = write!(
            svg,
            r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#64748b" font-size="8" font-family="sans-serif">{label}</text>"##,
            tx = x + bar_w / 2,
            ty = chart_h + 14,
            label = escape_html(date_label),
        );
    }

    svg.push_str("</svg>");
    svg
}

/// HTMX partial: analytics status card.
async fn analytics_status_partial(State(state): State<AppState>) -> impl IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    let (status, css_class) = if configured {
        ("Active", "ok")
    } else if compiled {
        ("Not Configured", "warn")
    } else {
        ("Not Compiled", "err")
    };

    let hint = if configured {
        "DuckDB behavioral analytics are active."
    } else if compiled {
        "Start with <code>--analytics-db</code> to enable."
    } else {
        "Rebuild with <code>--features analytics</code> to enable."
    };

    let html = format!(
        r#"<div class="value"><span class="dot {css_class}"></span> {status}</div>
<div class="label">Analytics Engine</div>
<p style="color: var(--text-muted); font-size: 0.8rem; margin-top: 0.5rem;">{hint}</p>"#,
    );

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: activity sessions table.
#[cfg(feature = "analytics")]
async fn analytics_sessions_partial(State(state): State<AppState>) -> impl IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Activity sessions");
    }

    let params = birdnet_behavioral::types::SessionizeParams::default();

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sessionize(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(sessions)) => {
            if sessions.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p style="color: var(--text-muted)">No activity sessions detected yet. Sessions appear after enough detections are recorded.</p>"#.to_string(),
                );
            }

            let mut html = String::from(
                r"<table>
<thead><tr><th>Species</th><th>Detections</th><th>Start</th><th>Duration</th></tr></thead>
<tbody>",
            );

            for s in sessions.iter().take(20) {
                let duration = format_duration(s.duration_secs);
                let start = escape_html(&s.start_time);
                let _ = write!(
                    html,
                    r#"<tr>
    <td class="species-name">{species}</td>
    <td>{count}</td>
    <td>{start}</td>
    <td>{duration}</td>
</tr>"#,
                    species = escape_html(&s.species),
                    count = s.detection_count,
                );
            }

            html.push_str("</tbody></table>");

            if sessions.len() > 20 {
                let _ = write!(
                    html,
                    r#"<p style="color: var(--text-muted); font-size: 0.8rem; margin-top: 0.5rem;">Showing 20 of {} sessions.</p>"#,
                    sessions.len(),
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("sessions", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading sessions</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn analytics_sessions_partial(State(_state): State<AppState>) -> impl IntoResponse {
    analytics_unavailable_html("Activity sessions")
}

/// HTMX partial: species retention data.
#[cfg(feature = "analytics")]
async fn analytics_retention_partial(State(state): State<AppState>) -> impl IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Species retention");
    }

    let params = birdnet_behavioral::types::RetentionParams::default();

    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.retention(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(retention)) => {
            if retention.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p style="color: var(--text-muted)">No retention data yet. Retention is calculated after species are detected on multiple days.</p>"#.to_string(),
                );
            }

            let mut html = String::from(
                r"<table>
<thead><tr><th>Species</th><th>Classification</th><th>Day 1</th><th>Day 7</th><th>Day 30</th></tr></thead>
<tbody>",
            );

            for r in &retention {
                let classification = match r.classification {
                    birdnet_behavioral::types::ResidencyType::Resident => "Resident",
                    birdnet_behavioral::types::ResidencyType::Regular => "Regular",
                    birdnet_behavioral::types::ResidencyType::Migrant => "Migrant",
                    birdnet_behavioral::types::ResidencyType::Rarity => "Rarity",
                };

                let class_css = match r.classification {
                    birdnet_behavioral::types::ResidencyType::Resident => "high",
                    birdnet_behavioral::types::ResidencyType::Regular => "mid",
                    _ => "low",
                };

                // Find retention rates for day 1, 7, 30
                let day1 = find_rate(&r.retention_rates, 1);
                let day7 = find_rate(&r.retention_rates, 7);
                let day30 = find_rate(&r.retention_rates, 30);

                let _ = write!(
                    html,
                    r#"<tr>
    <td class="species-name">{species}</td>
    <td><span class="conf {class_css}">{classification}</span></td>
    <td>{day1}</td>
    <td>{day7}</td>
    <td>{day30}</td>
</tr>"#,
                    species = escape_html(&r.species),
                );
            }

            html.push_str("</tbody></table>");

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("retention", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading retention data</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn analytics_retention_partial(State(_state): State<AppState>) -> impl IntoResponse {
    analytics_unavailable_html("Species retention")
}

/// HTMX partial: next species predictions.
#[cfg(feature = "analytics")]
async fn analytics_next_partial(State(state): State<AppState>) -> impl IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Next species predictions");
    }

    // Get the most recent species to use as the trigger
    let trigger_result = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_db(|conn| {
                conn.query_row(
                    "SELECT Com_Name FROM detections ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            })
        }
    })
    .await;

    let trigger = match trigger_result {
        Ok(Some(name)) => name,
        _ => {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                r#"<p style="color: var(--text-muted)">No detections yet. Predictions require detection history.</p>"#.to_string(),
            );
        }
    };

    let trigger_display = trigger.clone();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.next_species(&trigger, 60, 5))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(predictions)) => {
            if predictions.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    format!(
                        r#"<p style="color: var(--text-muted)">No predictions available for <strong>{}</strong> yet. More detection data is needed.</p>"#,
                        escape_html(&trigger_display),
                    ),
                );
            }

            let mut html = format!(
                r#"<p style="font-size: 0.85rem; margin-bottom: 0.75rem;">After detecting <strong>{trigger}</strong>, these species are most likely next:</p>
<table>
<thead><tr><th>Species</th><th>Probability</th><th>Observed</th></tr></thead>
<tbody>"#,
                trigger = escape_html(&trigger_display),
            );

            for p in &predictions {
                let pct = p.probability * 100.0;
                let conf_class = if pct >= 50.0 {
                    "high"
                } else if pct >= 20.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr>
    <td class="species-name">{species}</td>
    <td><span class="conf {conf_class}">{pct:.0}%</span></td>
    <td>{freq} times</td>
</tr>"#,
                    species = escape_html(&p.predicted_species),
                    freq = p.frequency,
                );
            }

            html.push_str("</tbody></table>");

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("next_species", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading predictions</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
async fn analytics_next_partial(State(_state): State<AppState>) -> impl IntoResponse {
    analytics_unavailable_html("Next species predictions")
}

/// HTMX partial: analytics configuration info.
async fn analytics_config_partial(State(state): State<AppState>) -> impl IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();
    let db_path = state.db_path().display().to_string();
    let version = env!("CARGO_PKG_VERSION");

    let mut html = String::from(r#"<table style="font-size: 0.85rem;">"#);

    let _ = write!(
        html,
        r#"<tr><td style="font-weight: 600;">Version</td><td>{version}</td></tr>
<tr><td style="font-weight: 600;">SQLite Database</td><td><code>{db_path}</code></td></tr>
<tr><td style="font-weight: 600;">Analytics Compiled</td><td>{compiled}</td></tr>
<tr><td style="font-weight: 600;">Analytics Active</td><td>{configured}</td></tr>"#,
        db_path = escape_html(&db_path),
    );

    if compiled && !configured {
        html.push_str(
            r#"<tr><td colspan="2" style="color: var(--text-muted); padding-top: 0.5rem;">
Start with <code>--analytics-db &lt;path&gt;</code> to enable behavioral analytics.
</td></tr>"#,
        );
    } else if !compiled {
        html.push_str(
            r#"<tr><td colspan="2" style="color: var(--text-muted); padding-top: 0.5rem;">
Rebuild with <code>--features analytics</code> to enable DuckDB behavioral analytics.
</td></tr>"#,
        );
    }

    html.push_str("</table>");

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTML response when analytics is not available (feature disabled or not configured).
fn analytics_unavailable_html(
    feature: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let message = if cfg!(feature = "analytics") {
        format!(
            r#"<p style="color: var(--text-muted)">{feature} requires DuckDB analytics. Start with <code>--analytics-db</code> to enable.</p>"#,
        )
    } else {
        format!(
            r#"<p style="color: var(--text-muted)">{feature} requires the analytics feature. Rebuild with <code>--features analytics</code>.</p>"#,
        )
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        message,
    )
}

/// HTML error response when the behavioral extension failed.
#[cfg(feature = "analytics")]
fn extension_error_html(
    function: &str,
    error: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let html = format!(
        r#"<p style="color: var(--text-muted)">The <code>duckdb-behavioral</code> extension is required for {function}.</p>
<p style="color: var(--text-muted); font-size: 0.8rem;">{error}</p>"#,
        error = escape_html(error),
    );
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/html")],
        html,
    )
}

/// Format a duration in seconds as a human-readable string.
#[cfg(feature = "analytics")]
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Find a retention rate for a specific day interval.
#[cfg(feature = "analytics")]
fn find_rate(rates: &[birdnet_behavioral::types::RetentionRate], days: u32) -> String {
    rates
        .iter()
        .find(|r| r.days == days)
        .map_or_else(|| "—".to_string(), |r| format!("{:.0}%", r.rate * 100.0))
}

/// Count detections for today's date.
fn today_count(conn: &rusqlite::Connection) -> i64 {
    let today = today_date_string();
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date = ?1",
        [&today],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Get today's date as YYYY-MM-DD string.
fn today_date_string() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = secs / 86400;
    let (year, month, day) = days_to_date(days);
    format!("{year}-{month:02}-{day:02}")
}

/// Convert days since Unix epoch to (year, month, day).
///
/// Uses the civil calendar algorithm by Howard Hinnant.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
const fn days_to_date(days_since_epoch: u64) -> (u32, u32, u32) {
    let z = days_since_epoch as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    #[allow(clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    #[allow(clippy::cast_sign_loss, clippy::cast_lossless)]
    let y = (yoe as i64 + era * 400) as u32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Minimal HTML escaping for XSS prevention.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_basic() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("\"hello\""), "&quot;hello&quot;");
    }

    #[test]
    fn days_to_date_epoch() {
        let (y, m, d) = days_to_date(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_date_known_date() {
        // 2026-03-12 = 20524 days since epoch
        let (y, m, d) = days_to_date(20524);
        assert_eq!((y, m, d), (2026, 3, 12));
    }

    #[test]
    fn today_date_string_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }

    #[test]
    fn render_page_substitutes_placeholders() {
        let html = render_page("Test", "<p>Hello</p>", "dashboard");
        assert!(html.0.contains("<title>Test - BirdNet-Behavior</title>"));
        assert!(html.0.contains("<p>Hello</p>"));
        assert!(html.0.contains("class=\"active\""));
    }

    #[test]
    fn render_hourly_chart_empty() {
        let result = render_hourly_chart(&[]);
        assert!(result.contains("No detections today"));
    }

    #[test]
    fn render_hourly_chart_with_data() {
        let hours = vec![
            birdnet_db::sqlite::HourlyCount {
                hour: "06".to_string(),
                count: 5,
            },
            birdnet_db::sqlite::HourlyCount {
                hour: "07".to_string(),
                count: 12,
            },
        ];
        let svg = render_hourly_chart(&hours);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("rect"));
    }

    #[test]
    fn render_daily_chart_empty() {
        let result = render_daily_chart(&[]);
        assert!(result.contains("No detection data"));
    }

    #[test]
    fn render_daily_chart_with_data() {
        let days = vec![
            birdnet_db::sqlite::DailyCount {
                date: "2026-03-10".to_string(),
                count: 15,
            },
            birdnet_db::sqlite::DailyCount {
                date: "2026-03-11".to_string(),
                count: 28,
            },
        ];
        let svg = render_daily_chart(&days);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("03-10"));
        assert!(svg.contains("03-11"));
    }

    #[cfg(feature = "analytics")]
    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(45), "45s");
    }

    #[cfg(feature = "analytics")]
    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(125), "2m 5s");
    }

    #[cfg(feature = "analytics")]
    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3725), "1h 2m");
    }
}
