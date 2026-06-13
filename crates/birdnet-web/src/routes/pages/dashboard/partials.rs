//! Dashboard HTMX partials: detection table, top species, charts, most recent.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::Deserialize;

use super::conf_class;
use crate::routes::pages::atoms::{avatar, conf_bar, sparkline, species_color, waveform};
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
                    crate::routes::pages::empty_states::quiet_yard(),
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
    let date_enc = simple_url_encode(&d.date);
    let time_enc = simple_url_encode(&d.time);
    let time_short = d.time.get(0..5).unwrap_or(&d.time);

    let badge = first_seen.get(&d.sci_name).map_or(String::new(), |fs| {
        if fs == today {
            r#" <span class="bnb-pill moss dp-badge">first today</span>"#.to_string()
        } else if fs == &d.date {
            r#" <span class="bnb-pill rare dp-badge">rare</span>"#.to_string()
        } else {
            String::new()
        }
    });

    // Fixed-size play affordance (shared clip player) replacing the native
    // <audio> controls, which rendered at different widths per row so the
    // feed never aligned (v3 spine, Today_home.html).
    let play = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map_or_else(
            || r#"<span class="bnb-meta dp-noclip">—</span>"#.to_string(),
            |f| {
                let basename = std::path::Path::new(f)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let safe = escape_html(&basename);
                format!(
                    r#"<button type="button" class="x-fplay" data-play-src="/api/v2/recordings/{safe}" title="Play clip" aria-label="Play clip">▶</button>"#
                )
            },
        );

    let fresh_cls = if fresh { " fresh bnb-rise" } else { "" };
    let _ = write!(
        html,
        r#"<div class="feed-row{fresh_cls}"><a class="ago mono dp-ago" href="/detections/detail?date={date_enc}&time={time_enc}&name={enc}" title="Open detection detail">{time_short}</a>{avatar}<div class="who"><div class="name"><a href="/species/detail?name={enc}" class="dp-link">{name}</a>{badge}</div><div class="sci mono">{sci}</div></div>{wave}{conf}{play}</div>"#,
        avatar = avatar(&d.com_name, ""),
        name = escape_html(&d.com_name),
        sci = escape_html(&d.sci_name),
        wave = waveform(row_seed(&d.com_name, &d.time), 24),
        conf = conf_bar(d.confidence),
    );
}

// ---------------------------------------------------------------------------
// Best recordings partial (BirdNET-Pi-style at-a-glance)
// ---------------------------------------------------------------------------

/// The day's highest-confidence detections that have a playable clip.
///
/// Brings back the BirdNET-Pi "best recordings" overview so the most confident
/// captures of the day are one glance away on the dashboard rather than a hunt
/// through the recordings browser. Reuses the live-feed row so the look matches.
pub(super) async fn best_detections_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let today = today_date_string();
    let today_for_query = today.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let best = birdnet_db::sqlite::best_detections_for_date(conn, &today_for_query, 5)?;
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((best, first_seen))
        })
    })
    .await;

    match result {
        Ok(Ok((best, first_seen))) => {
            if best.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="bnb-meta">No recordings yet today — best captures appear here as detections come in.</p>"#
                        .to_string(),
                );
            }
            let mut html = String::new();
            for d in &best {
                render_best_row(&mut html, d, &first_seen, &today);
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading best recordings</p>".to_string(),
        ),
    }
}

/// One compact best-recordings row — rail-scaled (avatar · name · time ·
/// confidence · first/rare tag · play), NOT a full feed row (v3 spine).
fn render_best_row(
    html: &mut String,
    d: &birdnet_db::sqlite::DetectionRow,
    first_seen: &std::collections::HashMap<String, String>,
    today: &str,
) {
    let enc = simple_url_encode(&d.com_name);
    let time_short = d.time.get(0..5).unwrap_or(&d.time);
    let tag = first_seen.get(&d.sci_name).map_or("", |fs| {
        if fs == today {
            r#" · <span class="x-tag-first">first today</span>"#
        } else if fs == &d.date {
            r#" · <span class="x-tag-rare">rare</span>"#
        } else {
            ""
        }
    });
    let play = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| {
            let basename = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let safe = escape_html(&basename);
            format!(
                r#"<button type="button" class="x-play" data-play-src="/api/v2/recordings/{safe}" title="Play clip" aria-label="Play clip">▶</button>"#
            )
        })
        .unwrap_or_default();
    let _ = write!(
        html,
        r#"<div class="x-best">{avatar}<div class="x-best-main"><div class="nm"><a href="/species/detail?name={enc}" class="t dp-link">{name}</a></div><div class="mt">{time_short} · {conf:.2}{tag}</div></div>{play}</div>"#,
        avatar = avatar(&d.com_name, ""),
        name = escape_html(&d.com_name),
        conf = d.confidence,
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
                // Banding code under the name (not the scientific name) — the
                // rail teaches the codes the rest of the UI speaks (v3 spine).
                let _ = write!(
                    html,
                    r#"<a class="x-top" href="/species/detail?name={enc}">{avatar}<div class="nm"><div class="t">{n}</div><div class="sc">{code}</div></div><span class="ct">{c}</span>{spark}</a>"#,
                    avatar = avatar(&s.com_name, ""),
                    n = escape_html(&s.com_name),
                    code = crate::routes::pages::atoms::species_code(&s.com_name),
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
                let body = if has_search {
                    r#"<p class="bnb-meta">No matching species found.</p>"#.to_string()
                } else {
                    crate::routes::pages::empty_states::no_species()
                };
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body);
            }
            let mut html = String::from(
                r#"<table><thead><tr><th class="dp-th-num">#</th><th>Species</th><th>14-day</th><th>Detections</th><th>Confidence</th></tr></thead><tbody>"#,
            );
            for (i, s) in species.iter().enumerate() {
                let enc = simple_url_encode(&s.com_name);
                let color = species_color(&s.com_name);
                let spark = sparklines
                    .get(&s.com_name)
                    .map(|data| sparkline(data, 84.0, 22.0, Some(&color)))
                    .unwrap_or_default();
                let _ = write!(
                    html,
                    r#"<tr><td class="mono dp-rank">{rank}</td><td><div class="dp-cell">{avatar}<div class="dp-min0"><div class="dp-name-strong"><a href="/species/detail?name={enc}" class="dp-link">{n}</a></div><div class="sci mono bnb-meta">{sci}</div></div></div></td><td>{spark}</td><td class="mono tabular">{c}</td><td>{conf}</td></tr>"#,
                    rank = i + 1,
                    avatar = avatar(&s.com_name, ""),
                    n = escape_html(&s.com_name),
                    sci = escape_html(&s.sci_name),
                    c = s.count,
                    conf = conf_bar(s.avg_confidence),
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
            "<p class=\"dp-empty\">No detections yet.</p>".to_string(),
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
                    class=\"dp-audio\">\
                  <source src=\"/api/v2/recordings/{safe_b}\" type=\"audio/wav\">\
                </audio>",
            )
        })
        .unwrap_or_default();

    let html = format!(
        "<div class=\"dp-recent\">\
           <div class=\"dp-recent-main\">\
             <div class=\"dp-recent-head\">\
               <a href=\"/species/detail?name={enc}\" \
                  class=\"dp-recent-name\">{com_safe}</a>\
               <span class=\"conf {cls}\">{conf_pct:.0}%</span>\
             </div>\
             <div class=\"dp-recent-sci\">{sci_safe}</div>\
             <div class=\"dp-recent-date\">\
               {date_safe} &nbsp;&#9679;&nbsp; {time_safe}\
             </div>\
             {audio_html}\
           </div>\
         </div>",
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
