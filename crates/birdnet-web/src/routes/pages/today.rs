//! Today's Detections page and HTMX partials.
//!
//! The primary daily-use page showing today's detections in a searchable,
//! paginated list with delete support and auto-refresh.

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::{avatar, conf_bar};
use super::{TODAY_PAGE_HTML, escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/today", get(today_page))
        .route("/pages/today-list", get(today_partial))
        .route("/pages/today-daystrip", get(today_daystrip_partial))
        .route("/pages/today-count", get(today_count_partial))
        .route("/pages/today-delete", axum::routing::post(delete_detection))
        .route(
            "/pages/today-relabel",
            axum::routing::post(relabel_detection),
        )
        .route("/pages/today-lock", axum::routing::post(lock_detection))
        .route("/pages/today-unlock", axum::routing::post(unlock_detection))
}

/// Query parameters for the today list partial.
#[derive(Debug, Deserialize)]
pub struct TodayParams {
    /// Search filter. Prefix with "NOT " for exclusion.
    pub search: Option<String>,
    /// Pagination offset.
    pub offset: Option<u32>,
    /// Items per page (default 40).
    pub limit: Option<u32>,
}

/// Form data for deleting a detection.
#[derive(Debug, Deserialize)]
pub struct DeleteForm {
    pub date: String,
    pub time: String,
    pub sci_name: String,
}

/// Form data for locking/unlocking a detection.
#[derive(Debug, Deserialize)]
pub struct LockForm {
    pub date: String,
    pub time: String,
    pub sci_name: String,
}

/// Form data for re-labeling a detection.
#[derive(Debug, Deserialize)]
pub struct RelabelForm {
    pub date: String,
    pub time: String,
    pub old_sci_name: String,
    pub new_sci_name: String,
    pub new_com_name: String,
}

/// Render the full Today page.
async fn today_page() -> Html<String> {
    // Skeleton placeholders (O-16) shown until the htmx swap targets load.
    let body = TODAY_PAGE_HTML
        .replace("{{skel_daystrip}}", super::skeletons::day_strip())
        .replace("{{skel_today_results}}", &super::skeletons::feed_rows(8));
    super::render_page("Today", &body, "today")
}

/// HTMX partial: today's detection count (for the header badge).
async fn today_count_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let search = params.search.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::todays_detection_count(conn, &today, search.as_deref())
        })
    })
    .await;

    match result {
        Ok(Ok(count)) => {
            let label = if params.search.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                format!("{count} matching detections")
            } else {
                format!("{count} detections today")
            };
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], label)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "Error loading count".to_string(),
        ),
    }
}

/// HTMX partial: paginated list of today's detections as cards.
async fn today_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let limit = params.limit.unwrap_or(40).min(200);
    let offset = params.offset.unwrap_or(0);
    let search = params.search.clone();
    let search2 = params.search.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows = birdnet_db::sqlite::todays_detections(
                conn,
                &today,
                search.as_deref(),
                limit,
                offset,
            )?;
            let total =
                birdnet_db::sqlite::todays_detection_count(conn, &today, search.as_deref())?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total))
        })
    })
    .await;

    match result {
        Ok(Ok((detections, total))) => {
            let mut html = String::with_capacity(4096);

            if detections.is_empty() && offset == 0 {
                html.push_str(
                    "<p style=\"color:var(--text-muted);text-align:center;padding:2rem;\">No detections found today.</p>",
                );
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
            }

            for d in &detections {
                render_detection_card(&mut html, d);
            }

            // "Load more" button if there are more results
            let shown = offset + limit;
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                clippy::cast_possible_wrap,
                clippy::cast_lossless
            )]
            let total_u = total as u32;
            if shown < total_u {
                let search_param = search2
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| format!("&search={}", simple_url_encode(s)))
                    .unwrap_or_default();
                let remaining = total_u.saturating_sub(shown);
                let _ = write!(
                    html,
                    "<div style=\"text-align:center;padding:1rem;\">\
                     <button hx-get=\"/pages/today-list?offset={shown}&limit={limit}{search_param}\" \
                     hx-target=\"#today-results\" hx-swap=\"innerHTML\" \
                     style=\"background:var(--bg-hover);border:1px solid var(--border);color:var(--text);\
                     padding:0.5rem 1.5rem;border-radius:var(--radius);cursor:pointer;font-size:0.9rem;\">\
                     Load {limit} more ({remaining} remaining)\
                     </button></div>",
                );
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

/// HTMX partial: a 24-hour `DayStrip` timeline of today's detections — an
/// hourly histogram with one colour-coded dot per detection (placed by time
/// and confidence), night bands, sunrise/sunset markers and a "now" line.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
async fn today_daystrip_partial(State(state): State<AppState>) -> impl IntoResponse {
    let today = today_date_string();
    let today_for_weather = today.clone();
    let state_for_weather = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::todays_detections(conn, &today, None, 1000, 0))
    })
    .await;

    // O-23 weather samples for today's overlay band. Reads from the
    // `weather` table — empty when the Open-Meteo poll job hasn't
    // populated it yet, in which case `overlays::weather_band` renders
    // its quiet placeholder rather than failing.
    let weather_samples = tokio::task::spawn_blocking(move || {
        let from = format!("{today_for_weather}T00:00:00Z");
        let to = format!("{today_for_weather}T23:59:59Z");
        state_for_weather.with_db(|conn| {
            use birdnet_db::weather::WeatherStore;
            conn.range(&from, &to).unwrap_or_default()
        })
    })
    .await
    .unwrap_or_default();

    // O-23 moon badge is always rendered — its computation is local
    // and the operator deserves the signal even on a quiet day. This
    // sits BEFORE the empty-state early return.
    let now_secs_for_quiet = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |x| i64::try_from(x.as_secs()).unwrap_or(i64::MAX));
    let moon_badge_quiet = super::overlays::moon_badge(now_secs_for_quiet);

    let Ok(Ok(rows)) = result else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading timeline</p>".to_string(),
        );
    };

    if rows.is_empty() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            format!(
                r#"<div class="bnb-meta" style="display:flex;gap:18px;align-items:center;justify-content:center;padding:1rem;">
  <span>No detections yet today.</span>
  {moon_badge_quiet}
</div>"#
            ),
        );
    }

    let mut hourly = [0i64; 24];
    let mut dots: Vec<(f64, String, f64)> = Vec::with_capacity(rows.len());
    for d in &rows {
        let hf = parse_hour_fraction(&d.time);
        let hi = hf as usize;
        if hi < 24 {
            hourly[hi] += 1;
        }
        dots.push((hf, super::atoms::species_color(&d.com_name), d.confidence));
    }

    // Current hour-of-day (UTC) for the "now" marker.
    let now_h = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |x| x.as_secs());
        (secs % 86_400) as f64 / 3600.0
    };

    let total: i64 = hourly.iter().sum();
    let mut peak_hour = 0usize;
    let mut peak = -1i64;
    for (h, &c) in hourly.iter().enumerate() {
        if c > peak {
            peak = c;
            peak_hour = h;
        }
    }
    let dawn: i64 = hourly[4..9].iter().sum();

    // O-23 signal-context overlay: moon badge (no network, always shown)
    // + an SVG weather band wired to the cached Open-Meteo rows. When the
    // weather table is empty (poll job not enabled or first run), the
    // band renders a quiet placeholder so the chrome doesn't shift.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |x| i64::try_from(x.as_secs()).unwrap_or(i64::MAX));
    let moon_badge = super::overlays::moon_badge(now_secs);

    let weather_band_html = {
        let samples: Vec<super::overlays::WeatherSample> = weather_samples
            .iter()
            .filter_map(|row| {
                row.at
                    .get(11..13)
                    .and_then(|hh| hh.parse::<u8>().ok())
                    .map(|hour| super::overlays::WeatherSample {
                        hour,
                        temp_c: row.temp_c,
                        precip_mm: row.precip_mm,
                        wind_kt: row.wind_kt,
                    })
            })
            .collect();
        super::overlays::weather_band(&samples, 1380.0, 22.0)
    };

    let caption = format!(
        r#"<div class="bnb-meta" style="display:flex;gap:18px;flex-wrap:wrap;align-items:center;margin-bottom:10px;"><span class="mono">{peak_hour:02}:00 peak hour</span><span>{dawn} in the dawn chorus</span><span>{total} total today</span>{moon_badge}</div>"#
    );
    let strip = super::viz::day_strip(&hourly, &dots, 6.0, 19.5, now_h);
    let overlay_strip = format!(
        r#"<div class="bnb-overlay-strip" aria-label="signal context"><span class="bnb-meta mono">weather</span><svg width="100%" height="22" viewBox="0 0 1380 22" preserveAspectRatio="none" role="presentation">{weather_band_html}</svg><span class="bnb-meta">{}</span></div>"#,
        if weather_samples.is_empty() {
            "no weather data"
        } else {
            "hourly"
        }
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        format!("{caption}{strip}{overlay_strip}"),
    )
}

/// Parse "HH:MM:SS" into a fractional hour in [0, 24). Best-effort.
fn parse_hour_fraction(t: &str) -> f64 {
    let mut it = t.split(':');
    let h: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let m: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    h + m / 60.0
}

/// Render a single detection row into the HTML buffer.
fn render_detection_card(html: &mut String, d: &birdnet_db::sqlite::DetectionRow) {
    let enc_name = simple_url_encode(&d.com_name);

    // Audio player
    let audio = d
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
                "<audio controls preload=\"none\" style=\"height:30px;width:200px;max-width:100%;margin-top:6px;\">\
                 <source src=\"/api/v2/recordings/{safe}\" type=\"audio/wav\">\
                 </audio>"
            )
        })
        .unwrap_or_default();

    let av = avatar(&d.com_name, "");
    let conf = conf_bar(d.confidence);
    let com_name = escape_html(&d.com_name);
    let sci_name = escape_html(&d.sci_name);
    let time = escape_html(&d.time);
    let date_enc = simple_url_encode(&d.date);
    let time_enc = simple_url_encode(&d.time);
    let date_raw = escape_html(&d.date);
    let time_raw = escape_html(&d.time);
    let sci_name_raw = escape_html(&d.sci_name);

    let _ = write!(
        html,
        "<div class=\"bnb-card\" style=\"display:flex;gap:14px;align-items:center;padding:10px 14px;margin-bottom:8px;\">\
         {av}\
         <div style=\"flex:1;min-width:0;\">\
         <div style=\"display:flex;align-items:center;gap:10px;flex-wrap:wrap;\">\
         <a href=\"/species/detail?name={enc_name}\" style=\"font-weight:500;color:inherit;font-size:14px;\">{com_name}</a>\
         {conf}\
         </div>\
         <div class=\"bnb-meta mono\" style=\"margin-top:2px;\">{sci_name} · \
         <a href=\"/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}\" style=\"color:inherit;\">{time}</a></div>\
         {audio}\
         </div>\
         <div style=\"display:flex;gap:6px;flex-shrink:0;\">\
         <button class=\"bnb-btn ghost\" hx-post=\"/pages/today-lock\" \
         hx-vals='{{\"date\":\"{date_raw}\",\"time\":\"{time_raw}\",\"sci_name\":\"{sci_name_raw}\"}}' \
         hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\" \
         title=\"Lock this detection (protect from auto-purge)\">🔒</button>\
         <button class=\"bnb-btn danger\" hx-post=\"/pages/today-delete\" \
         hx-vals='{{\"date\":\"{date_raw}\",\"time\":\"{time_raw}\",\"sci_name\":\"{sci_name_raw}\"}}' \
         hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\" \
         hx-confirm=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-action=\"hx-post\" \
         data-confirm-url=\"/pages/today-delete\" \
         data-confirm-title=\"Delete detection\" \
         data-confirm-body=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-confirm-label=\"Delete\" \
         data-confirm-style=\"danger\" \
         title=\"Delete this detection\">Delete</button>\
         </div></div>",
    );
}

/// Delete a detection and re-render the list.
async fn delete_detection(
    State(state): State<AppState>,
    Form(form): Form<DeleteForm>,
) -> impl IntoResponse {
    let date = form.date;
    let time = form.time;
    let sci_name = form.sci_name;

    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::delete_detection(conn, &date, &time, &sci_name))
    })
    .await;

    // Return an HTMX trigger to reload the today list
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\"></div>".to_string(),
    )
}

/// Re-label a detection and re-render the list.
async fn relabel_detection(
    State(state): State<AppState>,
    Form(form): Form<RelabelForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::relabel_detection(
                conn,
                &form.date,
                &form.time,
                &form.old_sci_name,
                &form.new_sci_name,
                &form.new_com_name,
            )
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\"></div>".to_string(),
    )
}

/// Lock a detection to protect it from disk purge.
async fn lock_detection(
    State(state): State<AppState>,
    Form(form): Form<LockForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::lock_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\"></div>".to_string(),
    )
}

/// Unlock a detection (allow disk purge again).
async fn unlock_detection(
    State(state): State<AppState>,
    Form(form): Form<LockForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::unlock_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-results\" hx-swap=\"innerHTML\" hx-include=\"#today-search\"></div>".to_string(),
    )
}
