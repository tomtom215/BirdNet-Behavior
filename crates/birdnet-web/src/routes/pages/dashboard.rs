//! Dashboard page and stats/detection/species HTMX partials.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::charts::{render_confidence_chart, render_daily_chart, render_hourly_chart};
use super::{DASHBOARD_HTML, escape_html, simple_url_encode, today_count, today_date_string};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard_page))
        .route("/kiosk", get(kiosk_page))
        .route("/pages/stats", get(stats_partial))
        .route("/pages/detections", get(detections_partial))
        .route("/pages/top-species", get(top_species_partial))
        .route("/pages/species-list", get(species_list_partial))
        .route("/pages/hourly-chart", get(hourly_chart_partial))
        .route("/pages/daily-chart", get(daily_chart_partial))
        .route("/pages/confidence-chart", get(confidence_chart_partial))
        .route("/pages/kiosk-content", get(kiosk_content_partial))
}

async fn dashboard_page() -> Html<String> {
    super::render_page("Dashboard", DASHBOARD_HTML, "dashboard")
}

/// Kiosk mode page — simplified auto-refreshing display for dedicated screens.
async fn kiosk_page() -> Html<String> {
    Html(KIOSK_HTML.to_string())
}

const KIOSK_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet Kiosk</title>
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { background:#0f172a; color:#e2e8f0; font-family:system-ui,-apple-system,sans-serif; padding:1rem; }
  h1 { font-size:1.5rem; margin-bottom:1rem; text-align:center; color:#89b4fa; }
  .stats { display:flex; gap:1rem; justify-content:center; margin-bottom:1rem; flex-wrap:wrap; }
  .stat { background:#1e293b; border-radius:8px; padding:0.75rem 1.5rem; text-align:center; min-width:120px; }
  .stat .value { font-size:2rem; font-weight:700; color:#89b4fa; }
  .stat .label { font-size:0.75rem; color:#94a3b8; text-transform:uppercase; }
  .recent { max-height:calc(100vh - 12rem); overflow-y:auto; }
  .detection { display:flex; align-items:center; gap:1rem; padding:0.5rem 0; border-bottom:1px solid #1e293b; }
  .detection .name { font-weight:600; font-size:1.1rem; }
  .detection .sci { font-style:italic; color:#94a3b8; font-size:0.85rem; }
  .detection .conf { padding:0.15rem 0.5rem; border-radius:4px; font-size:0.8rem; font-weight:600; }
  .conf.high { background:#166534; color:#4ade80; }
  .conf.mid { background:#854d0e; color:#facc15; }
  .conf.low { background:#991b1b; color:#fca5a5; }
  .detection .time { color:#94a3b8; font-size:0.85rem; margin-left:auto; white-space:nowrap; }
</style>
</head>
<body>
<h1>BirdNet-Behavior</h1>
<div id="kiosk-content"
     hx-get="/pages/kiosk-content"
     hx-trigger="load, every 30s"
     hx-swap="innerHTML">
  <p style="text-align:center;color:#94a3b8;">Loading...</p>
</div>
<script src="/static/htmx.min.js"></script>
</body>
</html>"##;

async fn stats_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let today = today_count(conn);
            let latest = birdnet_db::sqlite::latest_detection(conn).ok().flatten();
            (total, species, today, latest)
        })
    })
    .await;

    match result {
        Ok((total, species, today, latest)) => {
            let latest_html = if let Some((_, time, name)) = latest {
                format!(
                    r#"<div class="stat-card">
    <div class="value" style="font-size: 1.2rem;">{time}</div>
    <div class="label">Last: {name}</div>
</div>"#,
                    time = escape_html(&time),
                    name = escape_html(&name),
                )
            } else {
                r#"<div class="stat-card"><div class="value">--</div><div class="label">No Detections</div></div>"#.to_string()
            };

            let html = format!(
                r#"<div class="stat-card"><div class="value">{total}</div><div class="label">Total Detections</div></div>
<div class="stat-card"><div class="value">{species}</div><div class="label">Unique Species</div></div>
<div class="stat-card"><div class="value">{today}</div><div class="label">Today</div></div>
{latest_html}"#,
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

async fn detections_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let detections = birdnet_db::sqlite::recent_detections(conn, 20)?;
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((detections, first_seen))
        })
    })
    .await;

    match result {
        Ok(Ok((detections, first_seen))) => {
            let mut html = String::from(
                r"<table>
<thead><tr><th>Species</th><th>Confidence</th><th>Time</th><th>Date</th><th>Audio</th></tr></thead>
<tbody>",
            );
            for d in &detections {
                let conf_pct = d.confidence * 100.0;
                let cls = conf_class(conf_pct);
                let enc = simple_url_encode(&d.com_name);

                // Species badge: NEW (first seen today) or RARE (first seen > 30 days ago)
                let badge = first_seen.get(&d.sci_name).map_or(String::new(), |fs| {
                    if fs == &today {
                        r#" <span style="background:#166534;color:#86efac;font-size:.65rem;padding:1px 6px;border-radius:9999px;font-weight:700;vertical-align:middle;">NEW</span>"#.to_string()
                    } else if fs == &d.date && fs != &today {
                        // First seen on the date of this detection (historical new)
                        r#" <span style="background:#164e63;color:#67e8f9;font-size:.65rem;padding:1px 6px;border-radius:9999px;font-weight:700;vertical-align:middle;">RARE</span>"#.to_string()
                    } else {
                        String::new()
                    }
                });

                // Derive recording filename from file_name field if present.
                let audio_cell = d.file_name.as_deref()
                    .filter(|f| !f.is_empty())
                    .map(|f| {
                        let basename = std::path::Path::new(f)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let safe = escape_html(&basename);
                        format!(
                            r#"<audio controls preload="none" style="height:24px;max-width:160px;vertical-align:middle;">
                              <source src="/api/v2/recordings/{safe}" type="audio/wav">
                            </audio>"#
                        )
                    })
                    .unwrap_or_else(|| "—".to_string());
                let _ = write!(
                    html,
                    r#"<tr><td><a href="/species/detail?name={enc}" style="color:inherit;text-decoration:none;">{n}</a>{badge}</td><td><span class="conf {cls}">{conf_pct:.0}%</span></td><td>{t}</td><td>{d2}</td><td>{audio_cell}</td></tr>"#,
                    n = escape_html(&d.com_name),
                    t = escape_html(&d.time),
                    d2 = escape_html(&d.date),
                );
            }
            html.push_str("</tbody></table>");
            if detections.is_empty() {
                html = r#"<p style="color:var(--text-muted)">No detections yet.</p>"#.to_string();
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

async fn top_species_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 10))
    })
    .await;

    match result {
        Ok(Ok(species)) => {
            let mut html = String::new();
            for s in &species {
                let enc = simple_url_encode(&s.com_name);
                let _ = write!(
                    html,
                    r#"<a href="/species/detail?name={enc}" style="text-decoration:none;color:inherit;"><div class="species-item"><span class="species-name">{n}</span><span class="species-count">{c}</span></div></a>"#,
                    n = escape_html(&s.com_name),
                    c = s.count,
                );
            }
            if species.is_empty() {
                html = r#"<p style="color:var(--text-muted)">No species detected yet.</p>"#
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

#[derive(Deserialize)]
struct SpeciesListQuery {
    q: Option<String>,
}

async fn species_list_partial(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<SpeciesListQuery>,
) -> impl axum::response::IntoResponse {
    let search = query.q.unwrap_or_default();
    let search_trimmed = search.trim().to_string();
    let has_search = !search_trimmed.is_empty();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let species = if has_search {
                birdnet_db::sqlite::search_species(conn, &search_trimmed, 500)?
            } else {
                birdnet_db::sqlite::top_species(conn, 500)?
            };
            let sparklines = birdnet_db::sqlite::species_sparklines(conn, 7).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((species, sparklines))
        })
    })
    .await;

    match result {
        Ok(Ok((species, sparklines))) => {
            if species.is_empty() {
                let msg = if has_search {
                    "No matching species found."
                } else {
                    "No species detected yet."
                };
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    format!(r#"<p style="color:var(--text-muted)">{msg}</p>"#),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Species</th><th>7-Day</th><th>Detections</th><th>Avg Confidence</th></tr></thead><tbody>",
            );
            for s in &species {
                let conf_pct = s.avg_confidence * 100.0;
                let cls = conf_class(conf_pct);
                let enc = simple_url_encode(&s.com_name);
                let spark = sparklines
                    .get(&s.com_name)
                    .map(|data| render_sparkline_svg(data))
                    .unwrap_or_default();
                let _ = write!(
                    html,
                    r#"<tr><td><a href="/species/detail?name={enc}" style="color:inherit;text-decoration:none;">{n}</a></td><td>{spark}</td><td>{c}</td><td><span class="conf {cls}">{conf_pct:.0}%</span></td></tr>"#,
                    n = escape_html(&s.com_name),
                    c = s.count,
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading species list</p>".to_string(),
        ),
    }
}

/// Render an inline SVG sparkline from daily count data.
fn render_sparkline_svg(data: &[i64]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let w = 60.0_f64;
    let h = 20.0_f64;
    let max_val = data.iter().copied().max().unwrap_or(1).max(1) as f64;
    let n = data.len();

    let mut points = String::new();
    for (i, &val) in data.iter().enumerate() {
        let x = if n > 1 {
            (i as f64) / ((n - 1) as f64) * w
        } else {
            w / 2.0
        };
        let y = h - (val as f64 / max_val * (h - 2.0)) - 1.0;
        if !points.is_empty() {
            points.push(' ');
        }
        let _ = write!(points, "{x:.1},{y:.1}");
    }

    format!(
        r#"<svg width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" style="vertical-align:middle;"><polyline points="{points}" fill="none" stroke="var(--accent,#89b4fa)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round"/></svg>"#,
    )
}

async fn hourly_chart_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &today))
    })
    .await;
    match result {
        Ok(Ok(hours)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_hourly_chart(&hours),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

async fn daily_chart_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::daily_counts(conn, 7))
    })
    .await;
    match result {
        Ok(Ok(days)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_daily_chart(&days),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

async fn confidence_chart_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(birdnet_db::sqlite::confidence_distribution)
    })
    .await;
    match result {
        Ok(Ok(buckets)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_confidence_chart(&buckets),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

/// Kiosk content partial — stats + recent detections for auto-refresh.
async fn kiosk_content_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today = today_date_string();
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let today_count =
                birdnet_db::sqlite::todays_detection_count(conn, &today, None).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let recent = birdnet_db::sqlite::recent_detections(conn, 15).unwrap_or_default();
            (total, today_count, species, recent)
        })
    })
    .await;

    match result {
        Ok((total, today_n, species_n, recent)) => {
            let mut html = String::with_capacity(4096);
            let _ = write!(
                html,
                r#"<div class="stats">
  <div class="stat"><div class="value">{today_n}</div><div class="label">Today</div></div>
  <div class="stat"><div class="value">{total}</div><div class="label">Total</div></div>
  <div class="stat"><div class="value">{species_n}</div><div class="label">Species</div></div>
</div>
<div class="recent">"#,
            );

            for d in &recent {
                let conf_pct = d.confidence * 100.0;
                let cls = conf_class(conf_pct);
                let _ = write!(
                    html,
                    r#"<div class="detection">
  <div><div class="name">{com}</div><div class="sci">{sci}</div></div>
  <span class="conf {cls}">{conf_pct:.0}%</span>
  <span class="time">{time} &middot; {date}</span>
</div>"#,
                    com = escape_html(&d.com_name),
                    sci = escape_html(&d.sci_name),
                    time = escape_html(&d.time),
                    date = escape_html(&d.date),
                );
            }

            html.push_str("</div>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading kiosk data</p>".to_string(),
        ),
    }
}

fn conf_class(pct: f64) -> &'static str {
    if pct >= 80.0 {
        "high"
    } else if pct >= 50.0 {
        "mid"
    } else {
        "low"
    }
}
