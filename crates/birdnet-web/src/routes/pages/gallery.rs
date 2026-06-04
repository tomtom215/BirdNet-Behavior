//! Species photo gallery page with grid layout and filtering.
//!
//! | Path                        | Purpose                           |
//! |-----------------------------|-----------------------------------|
//! | `GET /gallery`              | Full gallery page                 |
//! | `GET /pages/gallery-grid`   | Photo grid partial (HTMX)         |

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::{species_code, species_color};
use super::{escape_html, render_page_for_request, simple_url_encode};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/gallery", get(gallery_page))
        .route("/pages/gallery-grid", get(gallery_grid_partial))
}

async fn gallery_page(headers: HeaderMap) -> Html<String> {
    render_page_for_request("Species Gallery", GALLERY_HTML, "gallery", &headers)
}

#[derive(Deserialize)]
struct GalleryQuery {
    q: Option<String>,
    sort: Option<String>,
}

/// HTMX partial: species photo card grid.
async fn gallery_grid_partial(
    State(state): State<AppState>,
    Query(params): Query<GalleryQuery>,
) -> impl axum::response::IntoResponse {
    let search = params.q.unwrap_or_default();
    let sort = params.sort.unwrap_or_default();

    // Clone the cache handle out before `state` moves into the blocking task,
    // so we can kick off a background photo warm after the query returns.
    let cache = state.image_cache();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let search_trimmed = search.trim().to_string();
            let has_search = !search_trimmed.is_empty();
            let species = if has_search {
                birdnet_db::sqlite::search_species(conn, &search_trimmed, 200)?
            } else {
                birdnet_db::sqlite::top_species(conn, 200)?
            };
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((species, first_seen, sort))
        })
    })
    .await;

    match result {
        Ok(Ok((mut species, first_seen, sort_by))) => {
            match sort_by.as_str() {
                "name" => species.sort_by(|a, b| a.com_name.cmp(&b.com_name)),
                "newest" => {
                    species.sort_by(|a, b| {
                        let fa = first_seen.get(&a.sci_name).cloned().unwrap_or_default();
                        let fb = first_seen.get(&b.sci_name).cloned().unwrap_or_default();
                        fb.cmp(&fa)
                    });
                }
                _ => {} // count (default)
            }

            if species.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="ga-msg">No species found.</p>"#.to_string(),
                );
            }

            // Progressive background warmer: fetch any not-yet-cached species
            // photos (keyed by scientific name) so the grid self-populates over
            // repeat visits. Paced to avoid bursting Wikimedia's rate limit;
            // already-cached species short-circuit with no network call.
            if let Some(cache) = cache {
                let scis: Vec<String> = species
                    .iter()
                    .map(|s| s.sci_name.clone())
                    .filter(|s| !s.is_empty())
                    .collect();
                tokio::spawn(async move {
                    for sci in scis {
                        if cache.get_cached(&sci).is_none() {
                            let _ = cache.get_image(&sci).await;
                            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                        }
                    }
                });
            }

            let mut html = String::with_capacity(species.len() * 300);
            html.push_str("<div class=\"ga-grid\">");

            for s in &species {
                // Detail link is keyed by common name (the URL the rest of the
                // UI uses); the species photo is keyed by *scientific* name so
                // gallery, species-detail, and detection-detail all share one
                // cache entry per bird.
                let enc = simple_url_encode(&s.com_name);
                let enc_img = simple_url_encode(&s.sci_name);
                let conf_pct = s.avg_confidence * 100.0;
                let cls = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let first = first_seen
                    .get(&s.sci_name)
                    .map(|d| escape_html(d))
                    .unwrap_or_default();

                let _ = write!(
                    html,
                    "<a href=\"/species/detail?name={enc}\" class=\"ga-card-link\">\
                     <div class=\"card ga-hover ga-card\">\
                       <div class=\"ga-thumb\">\
                         <div class=\"ga-thumb-bg\" data-style=\"background:color-mix(in oklch, {color} 15%, var(--surface))\">\
                           <span class=\"display ga-code\" data-style=\"color:{color}\">{code}</span>\
                         </div>\
                         <img src=\"/api/v2/species/image/{enc_img}/file\" alt=\"{name}\" \
                              loading=\"lazy\" \
                              class=\"ga-img\" \
                              data-hide-on-error>\
                       </div>\
                       <div class=\"ga-body\">\
                         <div class=\"ga-name\">{name}</div>\
                         <div class=\"ga-row\">\
                           <span class=\"ga-count\">{count} det.</span>\
                           <span class=\"conf {cls}\">{conf_pct:.0}%</span>\
                         </div>",
                    name = escape_html(&s.com_name),
                    count = s.count,
                    color = species_color(&s.com_name),
                    code = species_code(&s.com_name),
                );
                if !first.is_empty() {
                    let _ = write!(html, "<div class=\"ga-first\">First: {first}</div>");
                }
                html.push_str("</div></div></a>");
            }

            html.push_str("</div>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading gallery</p>".to_string(),
        ),
    }
}

const GALLERY_HTML: &str = r##"<div class="bnb-eyebrow">Browse</div><h1 class="display ga-h1">Gallery</h1>
<p class="ga-lede">Photo gallery of all detected species.</p>

<div class="ga-controls">
    <input type="text" id="gallery-search" name="q" placeholder="Search species..."
           hx-get="/pages/gallery-grid" hx-trigger="keyup changed delay:300ms"
           hx-target="#gallery-grid" hx-swap="innerHTML"
           hx-include="#gallery-sort"
           class="ga-search">
    <select id="gallery-sort" name="sort"
            hx-get="/pages/gallery-grid" hx-trigger="change"
            hx-target="#gallery-grid" hx-swap="innerHTML"
            hx-include="#gallery-search">
        <option value="count">Most Detections</option>
        <option value="name">Alphabetical</option>
        <option value="newest">Newest First</option>
    </select>
</div>

<div id="gallery-grid" hx-get="/pages/gallery-grid" hx-trigger="load" hx-swap="innerHTML">
    <p class="ga-msg">Loading gallery...</p>
</div>"##;
