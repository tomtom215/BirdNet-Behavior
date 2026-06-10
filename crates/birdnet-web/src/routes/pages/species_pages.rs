//! Species list page, species detail page, and all species HTMX partials.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
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

async fn species_page(headers: HeaderMap) -> Html<String> {
    let body = SPECIES_PAGE_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Species),
    );
    super::render_page_for_request("Species", &body, "species", &headers)
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
