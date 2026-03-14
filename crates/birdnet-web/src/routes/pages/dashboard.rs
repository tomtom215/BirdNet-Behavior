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
        .route("/pages/stats", get(stats_partial))
        .route("/pages/detections", get(detections_partial))
        .route("/pages/top-species", get(top_species_partial))
        .route("/pages/species-list", get(species_list_partial))
        .route("/pages/hourly-chart", get(hourly_chart_partial))
        .route("/pages/daily-chart", get(daily_chart_partial))
        .route("/pages/confidence-chart", get(confidence_chart_partial))
}

async fn dashboard_page() -> Html<String> {
    super::render_page("Dashboard", DASHBOARD_HTML, "dashboard")
}

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
            if has_search {
                birdnet_db::sqlite::search_species(conn, &search_trimmed, 500)
            } else {
                birdnet_db::sqlite::top_species(conn, 500)
            }
        })
    })
    .await;

    match result {
        Ok(Ok(species)) => {
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
                r"<table><thead><tr><th>Species</th><th>Detections</th><th>Avg Confidence</th></tr></thead><tbody>",
            );
            for s in &species {
                let conf_pct = s.avg_confidence * 100.0;
                let cls = conf_class(conf_pct);
                let enc = simple_url_encode(&s.com_name);
                let _ = write!(
                    html,
                    r#"<tr><td><a href="/species/detail?name={enc}" style="color:inherit;text-decoration:none;">{n}</a></td><td>{c}</td><td><span class="conf {cls}">{conf_pct:.0}%</span></td></tr>"#,
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

fn conf_class(pct: f64) -> &'static str {
    if pct >= 80.0 {
        "high"
    } else if pct >= 50.0 {
        "mid"
    } else {
        "low"
    }
}
