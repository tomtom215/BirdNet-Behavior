//! Species list page, species detail page, and all species HTMX partials.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::charts::{render_daily_chart, render_hourly_chart};
use super::{SPECIES_DETAIL_HTML, SPECIES_PAGE_HTML, escape_html, simple_url_encode};
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct SpeciesQuery {
    pub name: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species", get(species_page))
        .route("/species/detail", get(species_detail_page))
        .route("/pages/species-summary", get(species_summary_partial))
        .route("/pages/species-hourly", get(species_hourly_partial))
        .route("/pages/species-detections", get(species_detections_partial))
        .route("/pages/species-daily", get(species_daily_partial))
        .route("/pages/species-info", get(species_info_partial))
}

async fn species_page() -> Html<String> {
    super::render_page("Species", SPECIES_PAGE_HTML, "species")
}

async fn species_detail_page(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> Html<String> {
    let Some(name) = query.name else {
        return super::render_page("Species", "<p>No species specified.</p>", "species");
    };

    let com_name = name.clone();
    let sci_name = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            conn.query_row(
                "SELECT Sci_Name FROM detections WHERE Com_Name = ?1 LIMIT 1",
                [&com_name],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default()
        })
    })
    .await
    .unwrap_or_default();

    let encoded = simple_url_encode(&name);
    let content = SPECIES_DETAIL_HTML
        .replace("{{species_name}}", &escape_html(&name))
        .replace("{{scientific_name}}", &escape_html(&sci_name))
        .replace("{{species_encoded}}", &encoded);
    super::render_page(&name, &content, "species")
}

async fn species_summary_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_summary(conn, &name))
    })
    .await;

    match result {
        Ok(Ok(Some(summary))) => {
            let conf_pct = summary.avg_confidence * 100.0;
            let html = format!(
                r#"<div class="stat-card"><div class="value">{c}</div><div class="label">Detections</div></div>
<div class="stat-card"><div class="value">{conf_pct:.0}%</div><div class="label">Avg Confidence</div></div>
<div class="stat-card"><div class="value">{f}</div><div class="label">First Seen</div></div>
<div class="stat-card"><div class="value">{l}</div><div class="label">Last Seen</div></div>"#,
                c = summary.count,
                f = escape_html(&summary.first_seen),
                l = escape_html(&summary.last_seen),
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Ok(None)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p style="color:var(--text-muted)">Species not found.</p>"#.to_string(),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading summary</p>".to_string(),
        ),
    }
}

async fn species_hourly_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_hourly_activity(conn, &name))
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

async fn species_daily_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_daily_counts(conn, &name, 14))
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

async fn species_detections_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::detections_by_species(conn, &name, 20))
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            if detections.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p style="color:var(--text-muted)">No detections found.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Confidence</th><th>Time</th><th>Date</th></tr></thead><tbody>",
            );
            for d in &detections {
                let conf_pct = d.confidence * 100.0;
                let cls = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr><td><span class="conf {cls}">{conf_pct:.0}%</span></td><td>{t}</td><td>{dt}</td></tr>"#,
                    t = escape_html(&d.time),
                    dt = escape_html(&d.date),
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading detections</p>".to_string(),
        ),
    }
}

async fn species_info_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };

    let com_name = name.clone();
    let state_clone = state.clone();
    let sci_name = tokio::task::spawn_blocking(move || {
        state_clone.with_db(|conn| {
            conn.query_row(
                "SELECT Sci_Name FROM detections WHERE Com_Name = ?1 LIMIT 1",
                [&com_name],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default()
        })
    })
    .await
    .unwrap_or_default();

    let mut html = String::new();

    if let Some(cache) = state.image_cache() {
        if let Some(image) = cache.get_cached(&sci_name) {
            if image.cached_path.is_some() {
                let enc = simple_url_encode(&sci_name);
                let _ = write!(
                    html,
                    r#"<img src="/api/v2/species/image/{enc}/file" alt="{alt}" style="width:100%;border-radius:var(--radius);margin-bottom:1rem;" />"#,
                    alt = escape_html(&name),
                );
            }
            if let Some(desc) = &image.description {
                let _ = write!(
                    html,
                    r#"<p style="font-size:0.9rem;line-height:1.5;margin-bottom:0.75rem;">{}</p>"#,
                    escape_html(desc),
                );
            }
            if let Some(url) = &image.wiki_url {
                let _ = write!(
                    html,
                    r#"<p><a href="{}" target="_blank" rel="noopener">View on Wikipedia</a></p>"#,
                    escape_html(url),
                );
            }
        }
    }

    if html.is_empty() {
        html = format!(
            r#"<p style="color:var(--text-muted)">No additional info for <em>{}</em>.</p>
<p style="color:var(--text-muted);font-size:0.85rem;">Enable <code>--image-cache-dir</code> to fetch species images.</p>"#,
            escape_html(&name),
        );
    }

    // Add species info links (eBird/AllAboutBirds) — always shown
    let info_site = state.info_site();
    if info_site != "none" {
        let encoded_sci = simple_url_encode(&sci_name);
        let encoded_com = simple_url_encode(&name);
        match info_site {
            "allaboutbirds" => {
                let _ = write!(
                    html,
                    r#"<p style="margin-top:0.75rem;"><a href="https://www.allaboutbirds.org/guide/{encoded_com}" target="_blank" rel="noopener" style="color:var(--accent,#89b4fa);">View on All About Birds</a></p>"#,
                );
            }
            _ => {
                // Default to eBird
                let _ = write!(
                    html,
                    r#"<p style="margin-top:0.75rem;"><a href="https://ebird.org/species/{encoded_sci}" target="_blank" rel="noopener" style="color:var(--accent,#89b4fa);">View on eBird</a></p>"#,
                );
            }
        }
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
