//! Life List page: a birding journal showing every species ever detected,
//! with first/last seen dates, total count, and monthly discovery timeline.
//!
//! | Path                      | Purpose                                 |
//! |---------------------------|-----------------------------------------|
//! | `GET /life-list`          | Full life list page                     |
//! | `GET /pages/life-table`   | Life list table partial (HTMX)          |
//! | `GET /pages/life-stats`   | Summary stats partial (HTMX)            |
//! | `GET /pages/life-timeline`| Monthly discovery timeline (HTMX)       |

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, render_page, simple_url_encode};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/life-list", get(life_list_page))
        .route("/pages/life-table", get(life_table_partial))
        .route("/pages/life-stats", get(life_stats_partial))
        .route("/pages/life-timeline", get(life_timeline_partial))
}

async fn life_list_page() -> Html<String> {
    render_page("Life List", LIFE_LIST_HTML, "life-list")
}

#[derive(Deserialize)]
struct LifeListQuery {
    sort: Option<String>,
    q: Option<String>,
}

/// HTMX partial: life list summary stats.
async fn life_stats_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total_species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let total_detections = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let dates = birdnet_db::sqlite::distinct_detection_dates(conn).unwrap_or_default();
            let days_active = dates.len();
            (total_species, total_detections, days_active)
        })
    })
    .await;

    match result {
        Ok((species, detections, days)) => {
            let html = format!(
                "<div class=\"stat-card\">\
                   <div class=\"value\">{species}</div>\
                   <div class=\"label\">Life List Species</div>\
                 </div>\
                 <div class=\"stat-card\">\
                   <div class=\"value\">{detections}</div>\
                   <div class=\"label\">Total Detections</div>\
                 </div>\
                 <div class=\"stat-card\">\
                   <div class=\"value\">{days}</div>\
                   <div class=\"label\">Active Days</div>\
                 </div>",
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

/// HTMX partial: life list table with all species.
async fn life_table_partial(
    State(state): State<AppState>,
    Query(params): Query<LifeListQuery>,
) -> impl axum::response::IntoResponse {
    let sort = params.sort.unwrap_or_default();
    let search = params.q.unwrap_or_default();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            let species = birdnet_db::sqlite::top_species(conn, 10000)?;
            Ok::<_, birdnet_db::sqlite::DbError>((species, first_seen))
        })
    })
    .await;

    match result {
        Ok(Ok((mut species, first_seen))) => {
            // Filter by search term
            let search_lower = search.trim().to_lowercase();
            if !search_lower.is_empty() {
                species.retain(|s| s.com_name.to_lowercase().contains(&search_lower));
            }

            // Sort
            match sort.as_str() {
                "name" => species.sort_by(|a, b| a.com_name.cmp(&b.com_name)),
                "newest" => {
                    species.sort_by(|a, b| {
                        let fa = first_seen.get(&a.sci_name).cloned().unwrap_or_default();
                        let fb = first_seen.get(&b.sci_name).cloned().unwrap_or_default();
                        fb.cmp(&fa)
                    });
                }
                _ => {} // default: by count (already sorted)
            }

            if species.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p style="color:var(--text-muted);text-align:center;padding:2rem;">No species found.</p>"#.to_string(),
                );
            }

            let mut html = String::with_capacity(species.len() * 200);
            html.push_str(
                "<table><thead><tr>\
                 <th>#</th>\
                 <th>Species</th>\
                 <th>First Seen</th>\
                 <th>Detections</th>\
                 <th>Avg Confidence</th>\
                 </tr></thead><tbody>",
            );

            for (i, s) in species.iter().enumerate() {
                let enc = simple_url_encode(&s.com_name);
                let first = first_seen
                    .get(&s.sci_name)
                    .map_or_else(|| "\u{2014}".to_string(), |d| escape_html(d));
                let conf_pct = s.avg_confidence * 100.0;
                let cls = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    "<tr>\
                     <td style=\"color:var(--text-muted);\">{num}</td>\
                     <td><a href=\"/species/detail?name={enc}\" style=\"color:inherit;\">{name}</a></td>\
                     <td style=\"font-size:0.85rem;color:var(--text-muted);\">{first}</td>\
                     <td>{count}</td>\
                     <td><span class=\"conf {cls}\">{conf_pct:.0}%</span></td>\
                     </tr>",
                    num = i + 1,
                    name = escape_html(&s.com_name),
                    count = s.count,
                );
            }

            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading life list</p>".to_string(),
        ),
    }
}

/// HTMX partial: monthly species discovery timeline (bar chart).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
async fn life_timeline_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            // Count new species per month from first_seen dates
            let mut monthly: std::collections::BTreeMap<String, u32> =
                std::collections::BTreeMap::new();
            for date in first_seen.values() {
                if date.len() >= 7 {
                    let month = &date[..7]; // YYYY-MM
                    *monthly.entry(month.to_string()).or_default() += 1;
                }
            }
            monthly
        })
    })
    .await;

    let Ok(monthly) = result else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading timeline</p>".to_string(),
        );
    };

    if monthly.is_empty() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p style="color:var(--text-muted);text-align:center;">No discovery data yet.</p>"#
                .to_string(),
        );
    }

    let max_count = monthly.values().copied().max().unwrap_or(1).max(1);
    let bar_count = monthly.len();
    let bar_w = 32_i32;
    let gap = 4_i32;
    let chart_h = 100_i32;
    // Ensure minimum width so the SVG doesn't become absurdly tall
    let content_w = (bar_w + gap) * bar_count as i32 + 10;
    let svg_w = content_w.max(400);

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {sh}" style="width:100%;max-height:180px;display:block;" xmlns="http://www.w3.org/2000/svg">"#,
        sh = chart_h + 22,
    );

    for (i, (month, &count)) in monthly.iter().enumerate() {
        let x = 5 + i as i32 * (bar_w + gap);
        let bar_h = (count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;

        let _ = write!(
            svg,
            r##"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="#38bdf8"/>"##,
        );

        if count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#94a3b8" font-size="8" font-family="sans-serif">{count}</text>"##,
                tx = x + bar_w / 2,
                ty = y - 3,
            );
        }

        // Label every bar if <=12, otherwise every other
        let show_label = bar_count <= 12 || i % 2 == 0;
        if show_label {
            let label = month.get(2..).unwrap_or(month);
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#64748b" font-size="7" font-family="sans-serif">{label}</text>"##,
                tx = x + bar_w / 2,
                ty = chart_h + 14,
            );
        }
    }

    svg.push_str("</svg>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], svg)
}

const LIFE_LIST_HTML: &str = r##"<h1 style="margin-bottom:0.5rem;">Life List</h1>
<p style="color:var(--text-muted);margin-bottom:1.5rem;">Every species ever detected at this station.</p>

<div class="stats-grid" hx-get="/pages/life-stats" hx-trigger="load" hx-swap="innerHTML">
    <div class="stat-card"><div class="value">--</div><div class="label">Loading...</div></div>
</div>

<div class="card" style="margin-bottom:1rem;">
    <h2>New Species Over Time</h2>
    <p style="color:var(--text-muted);font-size:0.85rem;margin-bottom:0.75rem;">
        Number of new species discovered each month.
    </p>
    <div hx-get="/pages/life-timeline" hx-trigger="load" hx-swap="innerHTML">
        <p style="color:var(--text-muted);">Loading timeline...</p>
    </div>
</div>

<div class="card">
    <div style="display:flex;align-items:center;gap:1rem;margin-bottom:1rem;flex-wrap:wrap;">
        <h2 style="margin-bottom:0;">All Species</h2>
        <input type="text" id="life-search" name="q" placeholder="Search species..."
               hx-get="/pages/life-table" hx-trigger="keyup changed delay:300ms"
               hx-target="#life-table-body" hx-swap="innerHTML"
               hx-include="#life-sort"
               style="flex:1;min-width:200px;padding:0.4rem 0.75rem;border:1px solid var(--border);border-radius:var(--radius);background:var(--input-bg);color:var(--text);font-size:0.9rem;">
        <select id="life-sort" name="sort"
                hx-get="/pages/life-table" hx-trigger="change"
                hx-target="#life-table-body" hx-swap="innerHTML"
                hx-include="#life-search"
                style="padding:0.4rem 0.75rem;border:1px solid var(--border);border-radius:var(--radius);background:var(--input-bg);color:var(--text);font-size:0.85rem;">
            <option value="count">Most Detections</option>
            <option value="name">Alphabetical</option>
            <option value="newest">Newest First</option>
        </select>
    </div>
    <div id="life-table-body" hx-get="/pages/life-table" hx-trigger="load" hx-swap="innerHTML">
        <p style="color:var(--text-muted);">Loading life list...</p>
    </div>
</div>"##;
