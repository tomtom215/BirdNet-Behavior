//! Dashboard HTMX partials: detection table, top species, charts, most recent.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::Deserialize;

use super::conf_class;
use crate::routes::pages::atoms::{avatar, conf_bar, sparkline, waveform};
use crate::routes::pages::charts::{
    render_confidence_chart, render_daily_chart, render_hourly_chart,
};
use crate::routes::pages::{escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

/// Cheap deterministic seed for a detection's mini-waveform.
fn row_seed(name: &str, time: &str) -> u64 {
    let mut h: u64 = 1_469_598_103_934_665_603;
    for b in name.bytes().chain(time.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(1_099_511_628_211);
    }
    h
}

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
            if detections.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="bnb-meta">No detections yet — your station is listening.</p>"#
                        .to_string(),
                );
            }
            let mut html = String::new();
            for (i, d) in detections.iter().enumerate() {
                render_feed_row(&mut html, d, &first_seen, &today, i == 0);
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

/// Render one live-feed row in the redesigned dashboard style.
fn render_feed_row(
    html: &mut String,
    d: &birdnet_db::sqlite::DetectionRow,
    first_seen: &std::collections::HashMap<String, String>,
    today: &str,
    fresh: bool,
) {
    let enc = simple_url_encode(&d.com_name);
    let time_short = d.time.get(0..5).unwrap_or(&d.time);

    let badge = first_seen.get(&d.sci_name).map_or(String::new(), |fs| {
        if fs == today {
            r#" <span class="bnb-pill moss" style="font-size:9.5px;">first today</span>"#
                .to_string()
        } else if fs == &d.date {
            r#" <span class="bnb-pill rare" style="font-size:9.5px;">rare</span>"#.to_string()
        } else {
            String::new()
        }
    });

    let play = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map_or_else(
            || r#"<span class="bnb-meta">—</span>"#.to_string(),
            |f| {
                let basename = std::path::Path::new(f)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let safe = escape_html(&basename);
                format!(
                    r#"<audio controls preload="none" style="height:30px;width:120px;"><source src="/api/v2/recordings/{safe}" type="audio/wav"></audio>"#
                )
            },
        );

    let fresh_cls = if fresh { " fresh bnb-rise" } else { "" };
    let _ = write!(
        html,
        r#"<div class="feed-row{fresh_cls}"><span class="ago mono">{time_short}</span>{avatar}<div class="who"><div class="name"><a href="/species/detail?name={enc}" style="color:inherit;">{name}</a>{badge}</div><div class="sci mono">{sci}</div></div>{wave}{conf}{play}</div>"#,
        avatar = avatar(&d.com_name, ""),
        name = escape_html(&d.com_name),
        sci = escape_html(&d.sci_name),
        wave = waveform(row_seed(&d.com_name, &d.time), 24),
        conf = conf_bar(d.confidence),
    );
}

// ---------------------------------------------------------------------------
// Top species partial
// ---------------------------------------------------------------------------

pub(super) async fn top_species_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let species = birdnet_db::sqlite::top_species(conn, 6)?;
            let sparklines = birdnet_db::sqlite::species_sparklines(conn, 14).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((species, sparklines))
        })
    })
    .await;

    match result {
        Ok(Ok((species, sparklines))) => {
            if species.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="bnb-meta">No species detected yet.</p>"#.to_string(),
                );
            }
            let mut html = String::new();
            for s in &species {
                let enc = simple_url_encode(&s.com_name);
                let color = crate::routes::pages::atoms::species_color(&s.com_name);
                let spark = sparklines
                    .get(&s.com_name)
                    .map(|data| sparkline(data, 56.0, 16.0, Some(&color)))
                    .unwrap_or_default();
                let _ = write!(
                    html,
                    r#"<div class="list-row" style="grid-template-columns:auto 1fr auto 56px;">{avatar}<div style="min-width:0"><div style="font-weight:500;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;"><a href="/species/detail?name={enc}" style="color:inherit;">{n}</a></div><div class="sci mono bnb-meta" style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">{sci}</div></div><span class="mono tabular" style="font-size:13px;color:var(--fg-2)">{c}</span>{spark}</div>"#,
                    avatar = avatar(&s.com_name, ""),
                    n = escape_html(&s.com_name),
                    sci = escape_html(&s.sci_name),
                    c = s.count,
                );
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
