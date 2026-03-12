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

/// Page and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Full pages
        .route("/", get(dashboard_page))
        .route("/species", get(species_page))
        // HTMX partials
        .route("/pages/stats", get(stats_partial))
        .route("/pages/detections", get(detections_partial))
        .route("/pages/top-species", get(top_species_partial))
        .route("/pages/species-list", get(species_list_partial))
        .route("/pages/health-badge", get(health_badge_partial))
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
}
