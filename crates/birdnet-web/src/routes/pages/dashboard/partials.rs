//! Dashboard HTMX partials: detection table, top species, charts, most recent.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::Deserialize;

use super::conf_class;
use crate::routes::pages::charts::{
    render_confidence_chart, render_daily_chart, render_hourly_chart,
};
use crate::routes::pages::{escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Detections table partial
// ---------------------------------------------------------------------------

pub(super) async fn detections_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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
                render_detection_row(&mut html, d, &first_seen, &today);
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

fn render_detection_row(
    html: &mut String,
    d: &birdnet_db::sqlite::DetectionRow,
    first_seen: &std::collections::HashMap<String, String>,
    today: &str,
) {
    let conf_pct = d.confidence * 100.0;
    let cls = conf_class(conf_pct);
    let enc = simple_url_encode(&d.com_name);

    let badge = first_seen.get(&d.sci_name).map_or(String::new(), |fs| {
        if fs == today {
            r#" <span style="background:#166534;color:#86efac;font-size:.65rem;padding:1px 6px;border-radius:9999px;font-weight:700;vertical-align:middle;">NEW</span>"#.to_string()
        } else if fs == &d.date && fs != today {
            r#" <span style="background:#164e63;color:#67e8f9;font-size:.65rem;padding:1px 6px;border-radius:9999px;font-weight:700;vertical-align:middle;">RARE</span>"#.to_string()
        } else {
            String::new()
        }
    });

    let audio_cell = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map_or_else(
            || "\u{2014}".to_string(),
            |f| {
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
            },
        );
    let _ = write!(
        html,
        r#"<tr><td><a href="/species/detail?name={enc}" style="color:inherit;text-decoration:none;">{n}</a>{badge}</td><td><span class="conf {cls}">{conf_pct:.0}%</span></td><td>{t}</td><td>{d2}</td><td>{audio_cell}</td></tr>"#,
        n = escape_html(&d.com_name),
        t = escape_html(&d.time),
        d2 = escape_html(&d.date),
    );
}

// ---------------------------------------------------------------------------
// Top species partial
// ---------------------------------------------------------------------------

pub(super) async fn top_species_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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

// ---------------------------------------------------------------------------
// Species list partial (full table with search + sparklines)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct SpeciesListQuery {
    q: Option<String>,
}

pub(super) async fn species_list_partial(
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
#[allow(clippy::many_single_char_names)]
fn render_sparkline_svg(data: &[i64]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let w = 60.0_f64;
    let h = 20.0_f64;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let max_val = data.iter().copied().max().unwrap_or(1).max(1) as f64;
    let n = data.len();

    let mut points = String::new();
    for (i, &val) in data.iter().enumerate() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let x = if n > 1 {
            (i as f64) / ((n - 1) as f64) * w
        } else {
            w / 2.0
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let y = (val as f64 / max_val).mul_add(-(h - 2.0), h) - 1.0;
        if !points.is_empty() {
            points.push(' ');
        }
        let _ = write!(points, "{x:.1},{y:.1}");
    }

    format!(
        r#"<svg width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" style="vertical-align:middle;"><polyline points="{points}" fill="none" stroke="var(--accent,#89b4fa)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round"/></svg>"#,
    )
}

// ---------------------------------------------------------------------------
// Chart partials
// ---------------------------------------------------------------------------

pub(super) async fn hourly_chart_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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

pub(super) async fn daily_chart_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
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

pub(super) async fn confidence_chart_partial(
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

// ---------------------------------------------------------------------------
// Most recent detection card
// ---------------------------------------------------------------------------

pub(super) async fn most_recent_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(birdnet_db::sqlite::latest_detection_full)
    })
    .await;

    let Ok(Ok(Some(det))) = result else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p style=\"color:var(--text-muted);text-align:center;padding:1.5rem 0;\">No detections yet.</p>"
                .to_string(),
        );
    };

    let conf_pct = det.confidence * 100.0;
    let cls = conf_class(conf_pct);
    let com_safe = escape_html(&det.com_name);
    let sci_safe = escape_html(&det.sci_name);
    let date_safe = escape_html(&det.date);
    let time_safe = escape_html(&det.time);
    let enc = simple_url_encode(&det.com_name);

    let audio_html = det
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| {
            let basename = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let safe_b = escape_html(&basename);
            format!(
                "<audio controls preload=\"metadata\" \
                    style=\"width:100%;margin-top:0.6rem;height:32px;\">\
                  <source src=\"/api/v2/recordings/{safe_b}\" type=\"audio/wav\">\
                </audio>",
            )
        })
        .unwrap_or_default();

    let html = format!(
        "<div style=\"display:flex;align-items:flex-start;gap:1rem;flex-wrap:wrap;\">\
           <div style=\"flex:1;min-width:200px;\">\
             <div style=\"display:flex;align-items:center;gap:0.5rem;margin-bottom:0.2rem;\">\
               <a href=\"/species/detail?name={enc}\" \
                  style=\"font-size:1.1rem;font-weight:700;color:var(--text);\">{com_safe}</a>\
               <span class=\"conf {cls}\">{conf_pct:.0}%</span>\
             </div>\
             <div style=\"color:var(--text-muted);font-size:0.85rem;font-style:italic;\">{sci_safe}</div>\
             <div style=\"color:var(--text-muted);font-size:0.8rem;margin-top:0.2rem;\">\
               {date_safe} &nbsp;&#9679;&nbsp; {time_safe}\
             </div>\
             {audio_html}\
           </div>\
         </div>",
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
