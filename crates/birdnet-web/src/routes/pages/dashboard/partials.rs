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
        state.with_read_db(|conn| {
            let detections = birdnet_db::sqlite::recent_detections(conn, 20)?;
            let first_seen = birdnet_db::sqlite::species_first_detection(conn).unwrap_or_default();
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

    // Marks the one detection that *was* the species' first ever — not every
    // detection of a species whose first was recent. `first_seen` used to hold
    // a date, so `fs == today` was true for all 133 of today's blackcap rows
    // and badged every one of them; it now holds the first-ever instant, and
    // this compares the row's own.
    let badge = first_seen.get(&d.sci_name).map_or(String::new(), |fs| {
        if *fs != format!("{} {}", d.date, d.time) {
            String::new()
        } else if d.date == today {
            // "first ever" rather than "first today": now that exactly one row
            // can carry it, "today" only reintroduced the ambiguity between
            // "first time ever, and that was today" and "first one so far
            // today". This badge has always meant the former.
            r#" <span class="bnb-pill moss dp-badge">first ever</span>"#.to_string()
        } else {
            // Same fact, seen while browsing a past day.
            r#" <span class="bnb-pill rare dp-badge">rare</span>"#.to_string()
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
        state.with_read_db(|conn| {
            let best = birdnet_db::sqlite::best_detections_for_date(conn, &today_for_query, 5)?;
            let first_seen = birdnet_db::sqlite::species_first_detection(conn).unwrap_or_default();
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
    // Same exact-instant rule as the feed row above.
    let tag = first_seen.get(&d.sci_name).map_or("", |fs| {
        if *fs != format!("{} {}", d.date, d.time) {
            ""
        } else if d.date == today {
            r#" · <span class="x-tag-first">first ever</span>"#
        } else {
            r#" · <span class="x-tag-rare">rare</span>"#
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

/// How many species the Today card shows.
const TOP_SPECIES_ROWS: usize = 6;

/// The species heard **on `date`**, commonest first.
///
/// # Why this is not `top_species`
///
/// The card this fills is headed `Today · Top species`
/// (`templates/today.html:110`) and it was filled by
/// `birdnet_db::sqlite::top_species`, which reads the maintained
/// `species_summary` rollup. That rollup is keyed `(Com_Name, Sci_Name, hour)` —
/// it has **no date dimension at all**, so it cannot answer a question about
/// today, and what the card showed was the station's all-time totals under a
/// heading that said otherwise.
///
/// It is visible in the shipped documentation screenshot: the header above reads
/// "30 detections · 12 species" and the card beneath it reads 1444 / 1332 / 1207.
///
/// `species_for_date` is date-scoped and lands on `idx_detections_date`, so this
/// is also cheaper than what it replaces: measured on a three-year, 3.285 M-row
/// database, the date-scoped group-by is ~1 ms.
///
/// The equivalent card in the weekly report was already correct
/// (`weekly_report.rs` uses `weekly_top_species`), so this was one card rather
/// than a pattern.
fn todays_top_species(
    conn: &rusqlite::Connection,
    date: &str,
    limit: usize,
) -> Result<Vec<(String, String, i64)>, birdnet_db::sqlite::DbError> {
    let mut rows = birdnet_db::sqlite::species_for_date(conn, date)?;
    rows.truncate(limit);
    Ok(rows)
}

pub(super) async fn top_species_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let species = todays_top_species(conn, &today, TOP_SPECIES_ROWS)?;
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
                    r#"<p class="bnb-meta">Nothing heard yet today.</p>"#.to_string(),
                );
            }
            let mut html = String::new();
            for (com_name, _sci_name, count) in &species {
                let enc = simple_url_encode(com_name);
                let color = crate::routes::pages::atoms::species_color(com_name);
                let spark = sparklines
                    .get(com_name)
                    .map(|data| sparkline(data, 56.0, 16.0, Some(&color)))
                    .unwrap_or_default();
                // Banding code under the name (not the scientific name) — the
                // rail teaches the codes the rest of the UI speaks (v3 spine).
                let _ = write!(
                    html,
                    r#"<a class="x-top" href="/species/detail?name={enc}">{avatar}<div class="nm"><div class="t">{n}</div><div class="sc">{code}</div></div><span class="ct">{c}</span>{spark}</a>"#,
                    avatar = avatar(com_name, ""),
                    n = escape_html(com_name),
                    code = crate::routes::pages::atoms::species_code(com_name),
                    c = count,
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
        state.with_read_db(|conn| {
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
        state.with_read_db(|conn| birdnet_db::sqlite::hourly_activity(conn, &today))
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
        state.with_read_db(|conn| birdnet_db::sqlite::daily_counts(conn, 7))
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
        state.with_read_db(birdnet_db::sqlite::confidence_distribution)
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
        state.with_read_db(birdnet_db::sqlite::latest_detection_full)
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

#[cfg(test)]
mod tests {
    use super::{TOP_SPECIES_ROWS, todays_top_species};

    /// A station with history: yesterday was busy with one species, today with
    /// another. The Today card must not answer with yesterday's.
    fn two_day_station() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let insert = |date: &str, time: &str, com: &str, sci: &str| {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, ?3, ?4, 0.9)",
                rusqlite::params![date, time, sci, com],
            )
            .unwrap();
        };
        for i in 0..40 {
            insert(
                "2026-06-14",
                &format!("07:{:02}:00", i % 60),
                "Yesterday Bird",
                "Aves hesterna",
            );
        }
        for i in 0..3 {
            insert(
                "2026-06-15",
                &format!("06:{i:02}:00"),
                "Today Bird",
                "Aves hodierna",
            );
        }
        conn
    }

    /// Observed failing against the shipped implementation — restoring the body
    /// to `birdnet_db::sqlite::top_species(conn, 6)`, which reads the dateless
    /// `species_summary` rollup, yields
    /// `the Today card must lead with today's commonest species: "Yesterday Bird"`.
    #[test]
    fn the_today_card_shows_today_not_all_time() {
        let conn = two_day_station();
        let rows = todays_top_species(&conn, "2026-06-15", TOP_SPECIES_ROWS).unwrap();

        assert_eq!(rows.len(), 1, "only one species was heard today");
        assert_eq!(
            rows[0].0, "Today Bird",
            "the Today card must lead with today's commonest species"
        );
        assert_eq!(rows[0].2, 3, "and with today's count, not the all-time one");
    }

    /// The counterpart, so the gate above discriminates rather than merely
    /// alarming: asked about yesterday, the same function answers with yesterday.
    #[test]
    fn the_same_query_answers_correctly_for_another_day() {
        let conn = two_day_station();
        let rows = todays_top_species(&conn, "2026-06-14", TOP_SPECIES_ROWS).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "Yesterday Bird");
        assert_eq!(rows[0].2, 40);
    }

    /// A day with nothing on it is empty, not "the all-time list" — which is what
    /// a dateless rollup would return for a station that has simply not heard
    /// anything yet this morning.
    #[test]
    fn a_quiet_day_is_empty() {
        let conn = two_day_station();
        assert!(
            todays_top_species(&conn, "2026-06-16", TOP_SPECIES_ROWS)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_card_is_capped_at_its_row_count() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        for i in 0..(TOP_SPECIES_ROWS + 4) {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-06-15', ?1, ?2, ?3, 0.9)",
                rusqlite::params![
                    format!("06:{i:02}:00"),
                    format!("Genus sp{i}"),
                    format!("Bird {i}")
                ],
            )
            .unwrap();
        }
        assert_eq!(
            todays_top_species(&conn, "2026-06-15", TOP_SPECIES_ROWS)
                .unwrap()
                .len(),
            TOP_SPECIES_ROWS
        );
    }
}
