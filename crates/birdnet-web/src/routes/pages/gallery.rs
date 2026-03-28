//! Species photo gallery page with grid layout and filtering.
//!
//! | Path                        | Purpose                           |
//! |-----------------------------|-----------------------------------|
//! | `GET /gallery`              | Full gallery page                 |
//! | `GET /pages/gallery-grid`   | Photo grid partial (HTMX)         |

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
        .route("/gallery", get(gallery_page))
        .route("/pages/gallery-grid", get(gallery_grid_partial))
}

async fn gallery_page() -> Html<String> {
    render_page("Species Gallery", GALLERY_HTML, "species")
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
                    r#"<p style="color:var(--text-muted);text-align:center;padding:3rem;">No species found.</p>"#.to_string(),
                );
            }

            let mut html = String::with_capacity(species.len() * 300);
            html.push_str(
                "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(180px,1fr));gap:1rem;\">",
            );

            for s in &species {
                let enc = simple_url_encode(&s.com_name);
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
                    "<a href=\"/species/detail?name={enc}\" style=\"text-decoration:none;color:inherit;\">\
                     <div class=\"card\" style=\"padding:0;overflow:hidden;transition:transform 0.15s,box-shadow 0.15s;cursor:pointer;\" \
                          onmouseover=\"this.style.transform='translateY(-2px)';this.style.boxShadow='0 4px 12px rgba(0,0,0,0.15)'\" \
                          onmouseout=\"this.style.transform='';this.style.boxShadow=''\">\
                       <div style=\"height:120px;background:var(--bg-hover);display:flex;align-items:center;justify-content:center;overflow:hidden;\">\
                         <img src=\"/api/v2/images/species/{enc}\" alt=\"{name}\" \
                              loading=\"lazy\" \
                              style=\"width:100%;height:100%;object-fit:cover;\" \
                              onerror=\"this.style.display='none';this.parentElement.innerHTML='<div style=\\'font-size:2rem;color:var(--text-muted);\\'>&#x1F426;</div>'\">\
                       </div>\
                       <div style=\"padding:0.75rem;\">\
                         <div style=\"font-weight:600;font-size:0.9rem;margin-bottom:0.25rem;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;\">{name}</div>\
                         <div style=\"display:flex;justify-content:space-between;align-items:center;\">\
                           <span style=\"font-size:0.8rem;color:var(--text-muted);\">{count} det.</span>\
                           <span class=\"conf {cls}\">{conf_pct:.0}%</span>\
                         </div>",
                    name = escape_html(&s.com_name),
                    count = s.count,
                );
                if !first.is_empty() {
                    let _ = write!(
                        html,
                        "<div style=\"font-size:0.7rem;color:var(--text-muted);margin-top:0.25rem;\">First: {first}</div>",
                    );
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

const GALLERY_HTML: &str = r##"<h1 style="margin-bottom:0.5rem;">Species Gallery</h1>
<p style="color:var(--text-muted);margin-bottom:1.5rem;">Photo gallery of all detected species.</p>

<div style="display:flex;align-items:center;gap:1rem;margin-bottom:1.5rem;flex-wrap:wrap;">
    <input type="text" id="gallery-search" name="q" placeholder="Search species..."
           hx-get="/pages/gallery-grid" hx-trigger="keyup changed delay:300ms"
           hx-target="#gallery-grid" hx-swap="innerHTML"
           hx-include="#gallery-sort"
           style="flex:1;min-width:200px;">
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
    <p style="color:var(--text-muted);text-align:center;padding:3rem;">Loading gallery...</p>
</div>"##;
