//! Species list page, species detail page, and all species HTMX partials.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::{avatar, conf_bar, sparkline, species_code, species_color};
use super::charts::{render_daily_chart, render_hourly_chart};
use super::{SPECIES_DETAIL_HTML, escape_html, simple_url_encode};
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct SpeciesQuery {
    pub name: Option<String>,
}

/// The Species home query: which view, plus the List/Photos filter + search.
#[derive(Debug, Default, Deserialize)]
pub(super) struct HomeParams {
    view: Option<String>,
    filter: Option<String>,
    q: Option<String>,
}

/// Mount the species list, species detail, and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species", get(species_page))
        .route("/species/detail", get(species_detail_page))
        .route("/pages/species-summary", get(species_summary_partial))
        .route("/pages/species-hourly", get(species_hourly_partial))
        .route("/pages/species-detections", get(species_detections_partial))
        .route("/pages/species-daily", get(species_daily_partial))
        .route("/pages/species-info", get(species_info_partial))
        .route("/pages/species-companions", get(species_companions_partial))
        .route("/pages/species-hero", get(species_hero_partial))
        .route("/pages/species-status", get(species_status_partial))
}

/// The Species home (`/species?view=list|photos|lifelist`).
///
/// Folds the three pre-spine destinations — `/species` (List), `/gallery`
/// (Photos) and `/life-list` (Life list) — into one home with a view switcher,
/// filter chips and search. `/gallery` and `/life-list` permanently redirect
/// here (see [`crate::routes::redirects`]).
async fn species_page(
    State(state): State<AppState>,
    Query(params): Query<HomeParams>,
    headers: HeaderMap,
) -> Html<String> {
    let view = match params.view.as_deref() {
        Some("photos") => "photos",
        Some("lifelist") => "lifelist",
        _ => "list",
    };
    // The Life list answers a different question (every species ever), so the
    // List/Photos filter chips don't apply there.
    let filter = if params.filter.as_deref() == Some("week") {
        "week"
    } else {
        "all"
    };
    let search = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let st = state.clone();
    let s2 = search.clone();
    let body = tokio::task::spawn_blocking(move || match view {
        "photos" => photos_view(&st, filter, s2.as_deref()),
        "lifelist" => lifelist_view(&st),
        _ => list_view(&st, filter, s2.as_deref()),
    })
    .await
    .unwrap_or_default();

    let help = super::help::help_link(super::help::Topic::Species);
    let head = format!(
        r#"<div class="page-head" data-screen-label="Species head">
  <div>
    <div class="bnb-eyebrow"><span>Species</span>{help}</div>
    <h1 class="display sp-h1">Who you've heard</h1>
    <p class="bnb-meta sp-lede">Every bird your station has identified — browse the list, the photos, or your growing life list.</p>
  </div>
</div>"#
    );
    let content = format!(
        "{head}{controls}{body}",
        controls = controls(view, filter, search.as_deref())
    );
    super::render_page_for_request("Species", &content, "species", &headers)
}

/// The three Species views, in switcher order: `(key, glyph + label)`.
const VIEWS: &[(&str, &str)] = &[
    ("list", "▤ List"),
    ("photos", "▦ Photos"),
    ("lifelist", "✦ Life list"),
];

/// The List/Photos filter chips. Per-species "rare"/"migratory" metadata has no
/// honest source today (the station records detections, not range/status), so
/// only the two real chips ship; the rest are deferred (Wave D).
const FILTERS: &[(&str, &str)] = &[("all", "All"), ("week", "This week")];

/// The controls row: view switcher (`sp-seg`) · filter chips (`sp-chips`, List
/// and Photos only) · search (a GET form, so every view is bookmarkable).
fn controls(view: &str, filter: &str, search: Option<&str>) -> String {
    let mut seg = String::from(r#"<div class="sp-seg" role="tablist" aria-label="View">"#);
    for (key, label) in VIEWS {
        let active = if *key == view { " active" } else { "" };
        let cur = if *key == view {
            r#" aria-current="page""#
        } else {
            ""
        };
        let _ = write!(
            seg,
            r#"<a class="sp-seg-link{active}" href="/species?view={key}"{cur}>{label}</a>"#
        );
    }
    seg.push_str("</div>");

    // The filter chips and search only make sense on the List/Photos grids.
    let (chips, search_form) = if view == "lifelist" {
        (String::new(), String::new())
    } else {
        let mut c = String::from(r#"<div class="sp-chips">"#);
        for (key, label) in FILTERS {
            let active = if *key == filter { " active" } else { "" };
            let _ = write!(
                c,
                r#"<a class="sp-chip{active}" href="/species?view={view}&amp;filter={key}">{label}</a>"#
            );
        }
        c.push_str("</div>");
        let val = search.map(escape_html).unwrap_or_default();
        let form = format!(
            r#"<span class="sp-search"><span class="ico" aria-hidden="true">⌕</span><form method="get" action="/species" role="search"><input type="hidden" name="view" value="{view}"><input type="hidden" name="filter" value="{filter}"><input type="search" name="q" value="{val}" placeholder="Find a species…" aria-label="Find a species"></form></span>"#
        );
        (c, form)
    };
    format!(r#"<div class="sp-controls">{seg}{chips}{search_form}</div>"#)
}

/// The **List** view: `sp-count` headline + the `sp-table` (rank · avatar ·
/// 14-day sparkline · count · Avg confidence), every row a link to its detail.
fn list_view(state: &AppState, filter: &str, search: Option<&str>) -> String {
    let (mut species, sparks) = state.with_db(|conn| {
        let species = search.map_or_else(
            || birdnet_db::sqlite::top_species(conn, 500).unwrap_or_default(),
            |q| birdnet_db::sqlite::search_species(conn, q, 500).unwrap_or_default(),
        );
        let sparks = birdnet_db::sqlite::species_sparklines(conn, 14).unwrap_or_default();
        (species, sparks)
    });
    if filter == "week" {
        species.retain(|s| active_this_week(sparks.get(&s.com_name)));
    }

    let total: i64 = species.iter().map(|s| s.count).sum();
    let mut rows = String::new();
    for (i, s) in species.iter().enumerate() {
        let color = species_color(&s.com_name);
        let spark = sparks
            .get(&s.com_name)
            .map(|d| sparkline(d, 84.0, 22.0, Some(&color)))
            .unwrap_or_default();
        let enc = simple_url_encode(&s.com_name);
        let _ = write!(
            rows,
            r#"<tr><td class="sp-rank">{rank}</td><td><a class="sp-cell" href="/species/detail?name={enc}"><span class="sp-cell-av">{av}</span><span class="sp-cell-tx"><span class="sp-nm">{name}</span><span class="sp-sci">{sci}</span></span></a></td><td>{spark}</td><td class="sp-num">{count}</td><td>{conf}</td></tr>"#,
            rank = i + 1,
            av = avatar(&s.com_name, ""),
            name = escape_html(&s.com_name),
            sci = escape_html(&s.sci_name),
            count = format_count(s.count),
            conf = conf_bar(s.avg_confidence),
        );
    }

    let count_line = species_count_line(species.len(), filter, total);
    if species.is_empty() {
        return format!("{count_line}{}", empty_note(search));
    }
    format!(
        r#"{count_line}<div class="bnb-card pad"><table class="sp-table"><thead><tr><th class="sp-rank">#</th><th>Species</th><th>14-day</th><th>Detections</th><th>Avg confidence</th></tr></thead><tbody>{rows}</tbody></table></div>"#
    )
}

/// The **Photos** view: the gallery grid (`sp-grid` of `sp-photo-card`s) with
/// Wikipedia thumbnails over the gradient banding-code fallback.
fn photos_view(state: &AppState, filter: &str, search: Option<&str>) -> String {
    let (mut species, sparks) = state.with_db(|conn| {
        let species = search.map_or_else(
            || birdnet_db::sqlite::top_species(conn, 200).unwrap_or_default(),
            |q| birdnet_db::sqlite::search_species(conn, q, 200).unwrap_or_default(),
        );
        let sparks = birdnet_db::sqlite::species_sparklines(conn, 14).unwrap_or_default();
        (species, sparks)
    });
    if filter == "week" {
        species.retain(|s| active_this_week(sparks.get(&s.com_name)));
    }

    let mut cards = String::new();
    for s in &species {
        let color = species_color(&s.com_name);
        let code = species_code(&s.com_name);
        let enc = simple_url_encode(&s.com_name);
        let enc_sci = simple_url_encode(&s.sci_name);
        let _ = write!(
            cards,
            r#"<a class="sp-photo-card" href="/species/detail?name={enc}"><div class="bnb-card"><div class="bnb-photo sp-photo"><div class="ga-thumb-bg" data-style="background:color-mix(in oklch, {color} 15%, var(--surface))"><span class="display ga-code" data-style="color:{color}">{code}</span></div><img src="/api/v2/species/image/{enc_sci}/file" alt="{name}" loading="lazy" class="ga-img" data-hide-on-error></div><div class="sp-photo-meta"><div class="nm">{name}</div><div class="sub">{count} detections</div></div></div></a>"#,
            name = escape_html(&s.com_name),
            count = format_count(s.count),
        );
    }
    let count_line = format!(
        r#"<div class="sp-count"><b>{n}</b> species{wk} · click any card for the full detail</div>"#,
        n = species.len(),
        wk = if filter == "week" {
            " · active this week"
        } else {
            ""
        },
    );
    if species.is_empty() {
        return format!("{count_line}{}", empty_note(search));
    }
    format!(r#"{count_line}<div class="sp-grid">{cards}</div>"#)
}

/// The **Life list** view: the big counters, the accumulation curve, and the
/// "New to the list" recent firsts. Every species the station has ever heard.
fn lifelist_view(state: &AppState) -> String {
    let (species_total, det_total, active_days, points, firsts) = state.with_db(|conn| {
        let species_total = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
        let det_total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
        let active_days = birdnet_db::sqlite::distinct_detection_dates(conn).map_or(0, |v| v.len());
        let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
        let points = accumulation_points(&first_seen);
        let new_count = new_this_year(&first_seen);
        // Most-recent firsts: scientific-name keyed first-seen, joined to common
        // names via the top-species list (which carries both).
        let mut named: Vec<(String, String, String)> =
            birdnet_db::sqlite::top_species(conn, 10_000)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| {
                    first_seen
                        .get(&s.sci_name)
                        .map(|d| (s.com_name, s.sci_name, d.clone()))
                })
                .collect();
        named.sort_by(|a, b| b.2.cmp(&a.2));
        named.truncate(6);
        (
            species_total,
            det_total,
            active_days,
            points,
            (new_count, named),
        )
    });
    let (new_count, named) = firsts;

    let curve = super::viz::accumulation_curve(&points);
    let mut firsts_html = String::new();
    for (com, sci, date) in &named {
        let enc = simple_url_encode(com);
        let _ = write!(
            firsts_html,
            r#"<a class="sp-first-row" href="/species/detail?name={enc}">{av}<div class="sp-cell-tx"><div class="sp-nm">{name}</div><div class="sp-sci">{sci}</div></div><span class="when">{date}</span></a>"#,
            av = avatar(com, ""),
            name = escape_html(com),
            sci = escape_html(sci),
            date = escape_html(date),
        );
    }

    format!(
        r#"<div class="sp-life-head">
  <div>
    <div class="sp-life-stat">
      <div><div class="v moss">{species_total}</div><div class="l">species all-time</div></div>
      <div><div class="v">{active_days}</div><div class="l">active days</div></div>
      <div><div class="v">{new_count}</div><div class="l">new this year</div></div>
    </div>
    <p class="bnb-meta sp-life-lede">Every species your station has ever heard — {det} detections in all. The curve climbs fast at first, then each new bird gets rarer, and more exciting.</p>
  </div>
  <div class="bnb-card pad"><div class="bnb-eyebrow">Your growing list</div><div class="sd-viz">{curve}</div></div>
</div>
<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Most recent</div><h3>New to the list</h3></div></div><div class="sp-firsts">{firsts_html}</div></div>"#,
        det = format_count(det_total),
    )
}

/// Whether a species' 14-day sparkline shows any activity in the last 7 days.
fn active_this_week(spark: Option<&Vec<i64>>) -> bool {
    spark.is_some_and(|d| d.iter().rev().take(7).sum::<i64>() > 0)
}

/// Build the cumulative-species accumulation points (`(label, cum)`), binned by
/// month, mirroring the pre-spine life-list page.
fn accumulation_points(
    first_seen: &std::collections::HashMap<String, String>,
) -> Vec<(String, i64)> {
    let mut monthly: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for date in first_seen.values() {
        if let Some(month) = date.get(..7) {
            *monthly.entry(month.to_string()).or_default() += 1;
        }
    }
    let mut cum: i64 = 0;
    monthly
        .iter()
        .map(|(month, &c)| {
            cum += i64::from(c);
            (month.get(2..).unwrap_or(month).to_string(), cum)
        })
        .collect()
}

/// Count of species whose first-ever detection falls in the current year.
fn new_this_year(first_seen: &std::collections::HashMap<String, String>) -> usize {
    let year_prefix = super::today_date_string()
        .get(..4)
        .unwrap_or("")
        .to_string();
    if year_prefix.is_empty() {
        return 0;
    }
    first_seen
        .values()
        .filter(|d| d.starts_with(&year_prefix))
        .count()
}

/// The `sp-count` headline for the list view.
fn species_count_line(n: usize, filter: &str, total: i64) -> String {
    let scope = if filter == "week" {
        " · active this week"
    } else {
        ""
    };
    format!(
        r#"<div class="sp-count"><b>{n}</b> species{scope} · {total} detections all-time</div>"#,
        total = format_count(total),
    )
}

/// An honest empty state for a search / filter that matched nothing.
fn empty_note(search: Option<&str>) -> String {
    let what = search.map_or_else(
        || "No species match this filter yet.".to_string(),
        |q| format!("No species match “{}”.", escape_html(q)),
    );
    format!(r#"<div class="bnb-card pad bnb-meta">{what}</div>"#)
}

/// Group a count with thousands separators (e.g. `3142` → `3,142`).
fn format_count(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(*b));
    }
    if n < 0 { format!("-{out}") } else { out }
}

async fn species_detail_page(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
    headers: HeaderMap,
) -> Html<String> {
    let Some(name) = query.name else {
        return super::render_page_for_request(
            "Species",
            "<p>No species specified.</p>",
            "species",
            &headers,
        );
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
        .replace("{{species_encoded}}", &encoded)
        // Skeleton placeholders (O-16) shown until the htmx swap targets load.
        .replace("{{skel_species_status}}", &super::skeletons::pill_row(3))
        .replace("{{skel_hero}}", super::skeletons::hero_card())
        .replace("{{skel_species_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_circadian}}", &super::skeletons::hourly_bars(24))
        .replace("{{skel_trend}}", super::skeletons::trend_line())
        .replace("{{skel_detections}}", &super::skeletons::list_rows(5));
    super::render_page_for_request(&name, &content, "species", &headers)
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
            r#"<p class="spp-muted">Species not found.</p>"#.to_string(),
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
                    r#"<p class="spp-muted">No detections found.</p>"#.to_string(),
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

    // Species photos are cached by *scientific* name so the gallery,
    // species-detail, and detection-detail pages share one entry per bird
    // (falling back to the common name only if the scientific lookup failed).
    let img_key = if sci_name.is_empty() {
        name.clone()
    } else {
        sci_name.clone()
    };

    // The /file image route is cache-only, so warm this species' photo in the
    // background (non-blocking) on first view — a later view then shows it.
    if let Some(cache) = state.image_cache()
        && !img_key.is_empty()
        && cache.get_cached(&img_key).is_none()
    {
        let key_bg = img_key.clone();
        tokio::spawn(async move {
            let _ = cache.get_image(&key_bg).await;
        });
    }

    if let Some(cache) = state.image_cache()
        && let Some(image) = cache.get_cached(&img_key)
    {
        if image.cached_path.is_some() {
            let enc = simple_url_encode(&img_key);
            let _ = write!(
                html,
                r#"<img src="/api/v2/species/image/{enc}/file" alt="{alt}" class="spp-info-img" />"#,
                alt = escape_html(&name),
            );
        }
        if let Some(desc) = &image.description {
            let _ = write!(html, r#"<p class="spp-desc">{}</p>"#, escape_html(desc));
        }
        if let Some(url) = &image.wiki_url {
            let _ = write!(
                html,
                r#"<p><a href="{}" target="_blank" rel="noopener">View on Wikipedia</a></p>"#,
                escape_html(url),
            );
        }
    }

    if html.is_empty() {
        html = format!(
            r#"<p class="spp-muted">No additional info for <em>{}</em>.</p>
<p class="spp-muted-sm">Enable <code>--image-cache-dir</code> to fetch species images.</p>"#,
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
                    r#"<p class="spp-mt"><a href="https://www.allaboutbirds.org/guide/{encoded_com}" target="_blank" rel="noopener" class="spp-link">View on All About Birds</a></p>"#,
                );
            }
            _ => {
                // Default to eBird
                let _ = write!(
                    html,
                    r#"<p class="spp-mt"><a href="https://ebird.org/species/{encoded_sci}" target="_blank" rel="noopener" class="spp-link">View on eBird</a></p>"#,
                );
            }
        }
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: status pills (detection count, first/last heard, mean
/// confidence) shown under the species headline on the detail page.
async fn species_status_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            String::new(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_summary(conn, &name))
    })
    .await;

    let html = match result {
        Ok(Ok(Some(s))) => {
            let conf_pct = s.avg_confidence * 100.0;
            format!(
                r#"<span class="bnb-pill moss"><span class="bnb-dot"></span> {count} detections</span>
<span class="bnb-pill">First heard {first}</span>
<span class="bnb-pill">Last heard {last}</span>
<span class="bnb-pill">avg {conf_pct:.0}% confidence</span>"#,
                count = s.count,
                first = escape_html(&s.first_seen),
                last = escape_html(&s.last_seen),
            )
        }
        Ok(Ok(None)) => r#"<span class="bnb-pill">No detections yet</span>"#.to_string(),
        _ => String::new(),
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: "best detection" hero card — the highest-confidence clip for
/// the species, with the reference photo, spectrogram, and an audio scrubber.
async fn species_hero_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            String::new(),
        );
    };

    let lookup_name = name.clone();
    let state_clone = state.clone();
    let best = tokio::task::spawn_blocking(move || {
        state_clone.with_db(|conn| {
            conn.query_row(
                "SELECT Date, Time, Confidence, File_Name \
                 FROM detections \
                 WHERE Com_Name = ?1 AND File_Name IS NOT NULL AND File_Name <> '' \
                 ORDER BY Confidence DESC LIMIT 1",
                [&lookup_name],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, f64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .ok()
        })
    })
    .await
    .ok()
    .flatten();

    let Some((date, time, conf, file_name)) = best else {
        let html = r#"<div class="bnb-eyebrow spp-mb8">Best detection</div>
<div class="bnb-photo spp-photo-empty" data-caption="no clip yet"></div>
<p class="bnb-meta spp-mt8">No recording captured for this species yet.</p>"#;
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            html.to_string(),
        );
    };

    let basename = std::path::Path::new(&file_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or(file_name);
    let safe_file = escape_html(&basename);
    let time_short = time.get(0..5).unwrap_or(&time);
    let conf_pct = conf * 100.0;

    // The hero is the *recording* — the spectrogram and audio of the loudest
    // call. The species reference photo lives in the "About this species" card
    // below, so it isn't shown (cropped, and a second time) on the same page.
    let html = format!(
        r#"<div class="bnb-eyebrow spp-mb8">Best detection</div>
<img src="/api/v2/spectrogram/{safe_file}" alt="Spectrogram of the loudest detected call" data-hide-on-error class="spp-spectrogram" />
<audio controls preload="metadata" class="spp-audio"><source src="/api/v2/recordings/{safe_file}" type="audio/wav"></audio>
<div class="bnb-meta mono spp-mt8">{conf_pct:.0}% confidence · {date} {time_short}</div>"#,
    );

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: companion species (co-occurrence).
async fn species_companions_partial(
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
        state.with_db(|conn| birdnet_db::sqlite::companion_species(conn, &name, 30, 10))
    })
    .await;

    match result {
        Ok(Ok(companions)) => {
            if companions.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="spp-muted">No companion species data yet.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Companion</th><th>Co-occurrence Days</th></tr></thead><tbody>",
            );
            for c in &companions {
                let enc = simple_url_encode(&c.companion);
                let _ = write!(
                    html,
                    r#"<tr><td><a href="/species/detail?name={enc}" class="spp-inherit">{name}</a></td><td>{count}</td></tr>"#,
                    name = escape_html(&c.companion),
                    count = c.shared_days,
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading companion species</p>".to_string(),
        ),
    }
}
