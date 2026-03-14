//! Behavioral analytics HTMX partials (requires duckdb-behavioral extension).

#[cfg(feature = "analytics")]
use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};

use super::{ANALYTICS_PAGE_HTML, escape_html};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analytics", get(analytics_page))
        .route("/pages/analytics-sessions", get(analytics_sessions_partial))
        .route(
            "/pages/analytics-retention",
            get(analytics_retention_partial),
        )
        .route("/pages/analytics-next", get(analytics_next_partial))
        .route("/pages/analytics-config", get(analytics_config_partial))
}

async fn analytics_page() -> Html<String> {
    super::render_page("Analytics", ANALYTICS_PAGE_HTML, "analytics")
}

/// HTMX partial: activity sessions table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_sessions_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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
                    r#"<p style="color:var(--text-muted)">No activity sessions detected yet.</p>"#
                        .to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Species</th><th>Detections</th><th>Start</th><th>Duration</th></tr></thead><tbody>",
            );
            for s in sessions.iter().take(20) {
                let duration = format_duration(s.duration_secs);
                let _ = write!(
                    html,
                    r#"<tr><td>{sp}</td><td>{c}</td><td>{st}</td><td>{d}</td></tr>"#,
                    sp = escape_html(&s.species),
                    c = s.detection_count,
                    st = escape_html(&s.start_time),
                    d = duration,
                );
            }
            html.push_str("</tbody></table>");
            if sessions.len() > 20 {
                let _ = write!(
                    html,
                    r#"<p style="color:var(--text-muted);font-size:0.8rem;margin-top:0.5rem;">Showing 20 of {} sessions.</p>"#,
                    sessions.len()
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
pub(super) async fn analytics_sessions_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Activity sessions")
}

/// HTMX partial: species retention table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_retention_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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
                    r#"<p style="color:var(--text-muted)">No retention data yet.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Species</th><th>Classification</th><th>Day 1</th><th>Day 7</th><th>Day 30</th></tr></thead><tbody>",
            );
            for r in &retention {
                let (label, cls) = match r.classification {
                    birdnet_behavioral::types::ResidencyType::Resident => ("Resident", "high"),
                    birdnet_behavioral::types::ResidencyType::Regular => ("Regular", "mid"),
                    birdnet_behavioral::types::ResidencyType::Migrant => ("Migrant", "low"),
                    birdnet_behavioral::types::ResidencyType::Rarity => ("Rarity", "low"),
                };
                let _ = write!(
                    html,
                    r#"<tr><td>{sp}</td><td><span class="conf {cls}">{label}</span></td><td>{d1}</td><td>{d7}</td><td>{d30}</td></tr>"#,
                    sp = escape_html(&r.species),
                    d1 = find_rate(&r.retention_rates, 1),
                    d7 = find_rate(&r.retention_rates, 7),
                    d30 = find_rate(&r.retention_rates, 30),
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("retention", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading retention</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_retention_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Species retention")
}

/// HTMX partial: next-species predictions.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_next_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Next species predictions");
    }
    let trigger_result = tokio::task::spawn_blocking({
        let s = state.clone();
        move || {
            s.with_db(|conn| {
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
                r#"<p style="color:var(--text-muted)">No detections yet.</p>"#.to_string(),
            );
        }
    };

    let display = trigger.clone();
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
                        r#"<p style="color:var(--text-muted)">No predictions for <strong>{}</strong> yet.</p>"#,
                        escape_html(&display)
                    ),
                );
            }
            let mut html = format!(
                r#"<p style="font-size:0.85rem;margin-bottom:0.75rem;">After <strong>{}</strong>:</p><table><thead><tr><th>Species</th><th>Probability</th><th>Observed</th></tr></thead><tbody>"#,
                escape_html(&display),
            );
            for p in &predictions {
                let pct = p.probability * 100.0;
                let cls = if pct >= 50.0 {
                    "high"
                } else if pct >= 20.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr><td>{sp}</td><td><span class="conf {cls}">{pct:.0}%</span></td><td>{f}</td></tr>"#,
                    sp = escape_html(&p.predicted_species),
                    f = p.frequency
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
pub(super) async fn analytics_next_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Next species predictions")
}

async fn analytics_config_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();
    let db_path = escape_html(&state.db_path().display().to_string());
    let version = env!("CARGO_PKG_VERSION");
    let mut html = format!(
        r#"<table style="font-size:0.85rem;"><tr><td style="font-weight:600;">Version</td><td>{version}</td></tr>
<tr><td style="font-weight:600;">SQLite Database</td><td><code>{db_path}</code></td></tr>
<tr><td style="font-weight:600;">Analytics Compiled</td><td>{compiled}</td></tr>
<tr><td style="font-weight:600;">Analytics Active</td><td>{configured}</td></tr>"#,
    );
    if compiled && !configured {
        html.push_str(r#"<tr><td colspan="2" style="color:var(--text-muted);padding-top:0.5rem;">Start with <code>--analytics-db &lt;path&gt;</code> to enable.</td></tr>"#);
    } else if !compiled {
        html.push_str(r#"<tr><td colspan="2" style="color:var(--text-muted);padding-top:0.5rem;">Rebuild with <code>--features analytics</code> to enable.</td></tr>"#);
    }
    html.push_str("</table>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

fn analytics_unavailable_html(
    feature: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let msg = if cfg!(feature = "analytics") {
        format!(
            r#"<p style="color:var(--text-muted)">{feature} requires DuckDB analytics. Start with <code>--analytics-db</code>.</p>"#
        )
    } else {
        format!(
            r#"<p style="color:var(--text-muted)">{feature} requires the analytics feature. Rebuild with <code>--features analytics</code>.</p>"#
        )
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

#[cfg(feature = "analytics")]
fn extension_error_html(
    func: &str,
    error: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let html = format!(
        r#"<p style="color:var(--text-muted)">The <code>duckdb-behavioral</code> extension is required for {func}.</p>
<p style="color:var(--text-muted);font-size:0.8rem;">{error}</p>"#,
        error = escape_html(error),
    );
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/html")],
        html,
    )
}

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

#[cfg(feature = "analytics")]
fn find_rate(rates: &[birdnet_behavioral::types::RetentionRate], days: u32) -> String {
    rates
        .iter()
        .find(|r| r.days == days)
        .map_or_else(|| "—".to_string(), |r| format!("{:.0}%", r.rate * 100.0))
}
