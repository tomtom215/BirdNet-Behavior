//! Migration (phenology) page + partials.
//!
//! Mounts:
//!   GET  /migration                            full page
//!   GET  /pages/migration-stats                4 KPI tiles
//!   GET  /pages/migration-ridgeline            main SVG (per-species ridge)
//!   GET  /pages/migration-diversity            bottom diversity bar strip
//!   GET  /pages/migration-card?kind=…          arrived/peaking/missing editorial card
//!
//! All renders are pure SVG strings — no client JS needed beyond htmx.
//! Designed to use only the existing `detections` table; no schema migration.
//!
//! # No panicking operations in this module
//!
//! `[profile.release]` sets `panic = "abort"` and the server mounts no
//! catch-panic layer, so a panic in a request handler does not produce a 500 —
//! it takes the whole process down, web server and detection daemon together,
//! and systemd restarts it. A reachable handler panic is therefore a station
//! outage, not a failed request.
//!
//! This module sorted and compared `f32` with `partial_cmp(..).unwrap()`. The
//! values are sums of integer detection counts, so no reachable input is `NaN`
//! today and it was latent rather than live — but the cost of that assessment
//! being wrong later is the whole station, so the comparisons use
//! [`f32::total_cmp`], which is total by construction. `unwrap`/`expect` are
//! denied here so the class cannot return unnoticed; a genuinely infallible
//! call may re-allow the lint locally with a comment saying why.

#![deny(clippy::unwrap_used, clippy::expect_used)]
// Adapted SVG-rendering module: int<->float coordinate casts, short math
// identifiers, and long path-builder functions are intrinsic to this code.
#![allow(clippy::pedantic, clippy::nursery)]

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

use super::atoms::{species_code, species_color};
use super::escape_html;

const PAGE_HTML: &str = include_str!("../../../templates/migration.html");

/// Empty-state body served (uncached) when the phenology ridgeline has no data.
const RIDGELINE_EMPTY: &str =
    r#"<p class="bnb-meta">No migratory species detected yet this year.</p>"#;
/// Empty-state body served (uncached) when the diversity strip has no data.
const DIVERSITY_EMPTY: &str = r#"<p class="bnb-meta">No data for diversity bars yet.</p>"#;

/// Mount the migration (phenology) page and its HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/migration-stats", get(stats_partial))
        .route("/pages/migration-ridgeline", get(ridgeline_partial))
        .route("/pages/migration-diversity", get(diversity_partial))
        .route("/pages/migration-card", get(card_partial))
}

/// The migration/phenology surface, rendered for embedding by
/// `homes::patterns` ("Migration" tab).
pub(super) fn content() -> String {
    let year = current_year();
    // Skeleton placeholders (O-16) shown until the htmx swap targets load.
    // O-20 help link drops the methodology shortcut into the eyebrow.
    PAGE_HTML
        .replace("{{year}}", &year.to_string())
        .replace("{{skel_migration_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_ridgeline}}", super::skeletons::ridgeline())
        .replace("{{skel_diversity}}", &super::skeletons::diversity_bars())
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::Phenology),
        )
}

// ---------------------------------------------------------------------------
// Data shape
// ---------------------------------------------------------------------------

/// 52-week index per migratory species, normalized to its own peak.
struct SpeciesRidge {
    name: String,
    short: String, // 4-letter alpha code
    color: String, // oklch(…) for fill/stroke
    weekly: [f32; 52],
    peak_week: u8,
}

fn collect_ridges(
    conn: &rusqlite::Connection,
    year: i32,
    max_species: usize,
) -> rusqlite::Result<Vec<SpeciesRidge>> {
    // Migratory species = species that have a >2x ratio between peak month
    // and trough month over a multi-year window. Cheap heuristic — refine
    // with `birdnet_behavioral::ResidencyType::Migrant` when analytics
    // feature is on.
    let mut stmt = conn.prepare(
        "SELECT Com_Name, \
                CAST(strftime('%W', Date) AS INTEGER) AS wk, \
                COUNT(*) AS n \
         FROM detections_analytic \
         WHERE Date LIKE ?1 \
         GROUP BY Com_Name, wk",
    )?;
    let prefix = format!("{year}-%");
    let rows = stmt.query_map([&prefix], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    use std::collections::HashMap;
    let mut by_species: HashMap<String, [f32; 52]> = HashMap::new();
    for row in rows.flatten() {
        let (name, wk, n) = row;
        let w = wk.clamp(0, 51) as usize;
        let entry = by_species.entry(name).or_insert([0.0; 52]);
        entry[w] += n as f32;
    }

    // Filter: pick species whose peak/median ratio > 3 (proxy for migratory).
    let mut ridges: Vec<SpeciesRidge> = by_species
        .into_iter()
        .filter_map(|(name, weekly)| {
            let peak = weekly.iter().cloned().fold(0.0f32, f32::max);
            let mut sorted: Vec<f32> = weekly.iter().cloned().filter(|v| *v > 0.0).collect();
            if sorted.len() < 4 || peak < 5.0 {
                return None;
            }
            sorted.sort_by(f32::total_cmp);
            let median = sorted[sorted.len() / 2];
            if peak / median.max(1.0) < 3.0 {
                return None;
            }
            let peak_week = weekly
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)
                .unwrap_or(0) as u8;
            // Normalize.
            let mut norm = [0.0f32; 52];
            for (i, v) in weekly.iter().enumerate() {
                norm[i] = v / peak;
            }
            Some(SpeciesRidge {
                short: species_code(&name),
                color: species_color(&name),
                name,
                weekly: norm,
                peak_week,
            })
        })
        .collect();

    // Sort by peak week so ridges read left-to-right.
    ridges.sort_by_key(|r| r.peak_week);
    ridges.truncate(max_species);
    Ok(ridges)
}

// ---------------------------------------------------------------------------
// Ridgeline SVG
// ---------------------------------------------------------------------------

fn compute_ridgeline(state: &AppState) -> Option<String> {
    let year = current_year();
    let ridges = state.with_db(|conn| collect_ridges(conn, year, 12)).ok()?;
    if ridges.is_empty() {
        return None;
    }
    Some(render_ridgeline_svg(&ridges, current_week()))
}

async fn ridgeline_partial(State(state): State<AppState>) -> impl IntoResponse {
    let html = cached_fragment(
        &state,
        "migration-ridgeline".to_string(),
        RIDGELINE_EMPTY,
        compute_ridgeline,
    )
    .await;
    ok_html(html)
}

fn render_ridgeline_svg(ridges: &[SpeciesRidge], today_week: u8) -> String {
    const W: f64 = 1240.0;
    const H: f64 = 360.0;
    const PAD_L: f64 = 172.0; // room for the longest common names (e.g. "Ruby-throated Hummingbird")
    const PAD_R: f64 = 16.0;
    const PAD_T: f64 = 24.0;
    const PAD_B: f64 = 32.0;
    let inner_w = W - PAD_L - PAD_R;
    let inner_h = H - PAD_T - PAD_B;
    let weeks = 52.0;
    let x = |w: f64| PAD_L + (w / (weeks - 1.0)) * inner_w;
    let row_h = inner_h / ridges.len() as f64;

    let mut s = String::with_capacity(8 * 1024);
    let _ = write!(
        s,
        r#"<svg viewBox="0 0 {W} {H}" width="100%" height="auto" preserveAspectRatio="none" class="mig-svg">"#,
    );

    // Month gridlines.
    for (label, week) in MONTHS {
        let _ = write!(
            s,
            r#"<line x1="{x:.2}" y1="{y1}" x2="{x:.2}" y2="{y2}" stroke="var(--hairline)"/>"#,
            x = x(*week as f64),
            y1 = PAD_T,
            y2 = PAD_T + inner_h,
        );
        let _ = write!(
            s,
            r#"<text x="{x:.2}" y="{y}" text-anchor="middle" class="mono mig-tick">{label}</text>"#,
            x = x(*week as f64),
            y = PAD_T + inner_h + 18.0,
        );
    }

    // Season bands.
    let _ = write!(
        s,
        r#"<rect x="{x1:.2}" y="{y}" width="{w:.2}" height="{h:.2}" fill="var(--moss-soft)" fill-opacity="0.35"/>"#,
        x1 = x(8.0),
        y = PAD_T,
        w = x(20.0) - x(8.0),
        h = inner_h,
    );
    let _ = write!(
        s,
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono mig-band-spring">spring migration</text>"#,
        x = x(14.0),
        y = PAD_T + 12.0,
    );
    let _ = write!(
        s,
        r#"<rect x="{x1:.2}" y="{y}" width="{w:.2}" height="{h:.2}" fill="var(--dawn-soft)" fill-opacity="0.45"/>"#,
        x1 = x(34.0),
        y = PAD_T,
        w = x(44.0) - x(34.0),
        h = inner_h,
    );
    let _ = write!(
        s,
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono mig-band-fall">fall migration</text>"#,
        x = x(39.0),
        y = PAD_T + 12.0,
    );

    // Ridges.
    for (i, ridge) in ridges.iter().enumerate() {
        let y_base = PAD_T + (i as f64 + 1.0) * row_h - 6.0;
        let amp = row_h - 8.0;
        // Path points.
        let mut pts: Vec<(f64, f64)> = (0..52)
            .map(|w| (x(w as f64), y_base - (ridge.weekly[w] as f64) * amp))
            .collect();
        let mut d = String::from("M");
        for (j, (px, py)) in pts.iter().enumerate() {
            if j == 0 {
                let _ = write!(d, "{px:.2},{py:.2}");
            } else {
                let _ = write!(d, " L{px:.2},{py:.2}");
            }
        }
        // Close to baseline for area fill.
        let _ = write!(
            d,
            " L{:.2},{:.2} L{:.2},{:.2} Z",
            x(51.0),
            y_base,
            x(0.0),
            y_base,
        );

        let grad_id = format!("mg-{i}");
        let _ = write!(
            s,
            r#"<defs><linearGradient id="{grad_id}" x1="0" y1="0" x2="0" y2="1">"#,
        );
        let _ = write!(
            s,
            r#"<stop offset="0%" stop-color="{c}" stop-opacity="0.55"/>"#,
            c = ridge.color,
        );
        let _ = write!(
            s,
            r#"<stop offset="100%" stop-color="{c}" stop-opacity="0.05"/></linearGradient></defs>"#,
            c = ridge.color,
        );
        let _ = write!(
            s,
            r#"<line x1="{x1:.2}" y1="{y:.2}" x2="{x2:.2}" y2="{y:.2}" stroke="var(--hairline)"/>"#,
            x1 = PAD_L,
            y = y_base,
            x2 = PAD_L + inner_w,
        );
        let _ = write!(
            s,
            r#"<g class="ridge-band"><path d="{d}" fill="url(#{grad_id})"/><path d="{ds}" stroke="{c}" stroke-width="1.5" fill="none"/></g>"#,
            ds = {
                // re-derive open path without the close-to-baseline tail
                let mut s2 = String::from("M");
                for (j, (px, py)) in pts.iter().enumerate() {
                    if j == 0 {
                        let _ = write!(s2, "{px:.2},{py:.2}");
                    } else {
                        let _ = write!(s2, " L{px:.2},{py:.2}");
                    }
                }
                s2
            },
            c = ridge.color,
        );

        // Peak marker.
        let peak_w = ridge.peak_week as f64;
        let peak_y = y_base - amp;
        let _ = write!(
            s,
            r#"<line x1="{x:.2}" y1="{y1:.2}" x2="{x:.2}" y2="{y2:.2}" stroke="{c}" stroke-width="0.8" stroke-dasharray="2 2" stroke-opacity="0.4"/>"#,
            x = x(peak_w),
            y1 = peak_y,
            y2 = y_base,
            c = ridge.color,
        );
        let _ = write!(
            s,
            r#"<circle cx="{x:.2}" cy="{y:.2}" r="3" fill="{c}" stroke="var(--surface)" stroke-width="1"/>"#,
            x = x(peak_w),
            y = peak_y,
            c = ridge.color,
        );

        // Species label.
        let _ = write!(
            s,
            r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" class="mig-name">{name}</text>"#,
            x = PAD_L - 8.0,
            y = y_base - 6.0,
            name = escape_html(&ridge.name),
        );
        let _ = write!(
            s,
            r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" class="mono mig-sub">{short} · peak w{pw}</text>"#,
            x = PAD_L - 8.0,
            y = y_base + 8.0,
            short = escape_html(&ridge.short),
            pw = ridge.peak_week + 1,
        );
        // (Re-borrow pts to avoid move warnings.)
        let _ = &mut pts;
    }

    // Today indicator.
    let _ = write!(
        s,
        r#"<g><line x1="{x:.2}" y1="{y1}" x2="{x:.2}" y2="{y2:.2}" stroke="var(--fg)" stroke-width="1" stroke-dasharray="3 3"/>"#,
        x = x(today_week as f64),
        y1 = PAD_T,
        y2 = PAD_T + inner_h,
    );
    let _ = write!(
        s,
        r#"<rect x="{rx:.2}" y="{ry}" width="48" height="14" rx="3" fill="var(--fg)"/>"#,
        rx = x(today_week as f64) - 24.0,
        ry = PAD_T - 2.0,
    );
    let _ = write!(
        s,
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono mig-today">today</text></g>"#,
        x = x(today_week as f64),
        y = PAD_T + 8.0,
    );

    s.push_str("</svg>");
    s
}

// ---------------------------------------------------------------------------
// Diversity strip
// ---------------------------------------------------------------------------

fn compute_diversity(state: &AppState) -> Option<String> {
    let year = current_year();
    let weekly = state
        .with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT CAST(strftime('%W', Date) AS INTEGER) AS wk, \
                        COUNT(DISTINCT Com_Name) \
                 FROM detections_analytic \
                 WHERE Date LIKE ?1 \
                 GROUP BY wk \
                 ORDER BY wk",
            )?;
            let prefix = format!("{year}-%");
            let rows = stmt.query_map([&prefix], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?;
            let mut weekly = [0i64; 52];
            for row in rows.flatten() {
                let (w, n) = row;
                if (0..52).contains(&w) {
                    weekly[w as usize] = n;
                }
            }
            Ok::<_, rusqlite::Error>(weekly)
        })
        .ok()?;
    if weekly.iter().all(|&n| n == 0) {
        return None;
    }
    Some(render_diversity_svg(&weekly, current_week()))
}

async fn diversity_partial(State(state): State<AppState>) -> impl IntoResponse {
    let html = cached_fragment(
        &state,
        "migration-diversity".to_string(),
        DIVERSITY_EMPTY,
        compute_diversity,
    )
    .await;
    ok_html(html)
}

fn render_diversity_svg(weekly: &[i64; 52], today_week: u8) -> String {
    const W: f64 = 1240.0;
    const H: f64 = 70.0;
    const PAD_L: f64 = 92.0; // room for the "species / wk" axis label (mono)
    const PAD_R: f64 = 16.0;
    let inner_w = W - PAD_L - PAD_R;
    let bw = inner_w / 52.0;
    let x = |w: f64| PAD_L + (w / 51.0) * inner_w;
    let max = (*weekly.iter().max().unwrap_or(&1)).max(1) as f64;

    let mut s = String::with_capacity(4 * 1024);
    let _ = write!(
        s,
        r#"<svg viewBox="0 0 {W} {H}" width="100%" height="{H}" preserveAspectRatio="none">"#,
    );
    for (w, v) in weekly.iter().enumerate() {
        let is_spring = (8..=20).contains(&w);
        let is_fall = (34..=44).contains(&w);
        let fill = if is_spring {
            "var(--moss)"
        } else if is_fall {
            "var(--dawn)"
        } else {
            "var(--fg-3)"
        };
        let op = 0.30 + ((*v as f64 / max) * 0.55).min(0.55);
        let h = (*v as f64 / max) * (H - 16.0);
        let _ = write!(
            s,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{bw:.2}" height="{h:.2}" fill="{fill}" opacity="{op:.3}" rx="1"/>"#,
            x = x(w as f64) - bw / 2.0 + 0.5,
            y = H - 14.0 - h,
            bw = bw - 1.0,
        );
    }
    let _ = write!(
        s,
        r#"<line x1="{x1}" y1="{y}" x2="{x2:.2}" y2="{y}" stroke="var(--hairline)"/>"#,
        x1 = PAD_L,
        y = H - 14.0,
        x2 = PAD_L + inner_w,
    );
    let _ = write!(
        s,
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" class="mono mig-sub">species / wk</text>"#,
        x = PAD_L - 8.0,
        y = H - 12.0,
    );
    let _ = write!(
        s,
        r#"<line x1="{x:.2}" y1="2" x2="{x:.2}" y2="{y2:.2}" stroke="var(--fg)" stroke-width="1" stroke-dasharray="3 3"/>"#,
        x = x(today_week as f64),
        y2 = H - 14.0,
    );
    s.push_str("</svg>");
    s
}

// ---------------------------------------------------------------------------
// KPI tiles
// ---------------------------------------------------------------------------

async fn stats_partial(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(html) = state.analytics_cache().get("migration-stats") {
        return ok_html(html);
    }
    let today = crate::routes::pages::today_date_string();
    let year = year_of(&today);
    let prior = year - 1;
    let state_for_blocking = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        state_for_blocking.with_db(|conn| {
            // First-of-year arrivals = species with first_date in this year so far.
            let foy: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ( \
                   SELECT Com_Name, MIN(Date) AS first FROM detections_analytic GROUP BY Com_Name \
                 ) WHERE first LIKE ?1",
                [format!("{year}-%")],
                |r| r.get(0),
            )?;
            // Peak diversity week.
            let (peak_week, peak_n): (i64, i64) = conn
                .query_row(
                    "SELECT CAST(strftime('%W', Date) AS INTEGER) wk, COUNT(DISTINCT Com_Name) n \
                     FROM detections_analytic WHERE Date LIKE ?1 GROUP BY wk ORDER BY n DESC LIMIT 1",
                    [format!("{year}-%")],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap_or((0, 0));

            // Earliest-vs-last-year: of species heard in both years, find the
            // one with the most-negative (earlier) day-of-year delta vs prior.
            // Returns (species_name, delta_days) — `delta_days` is negative
            // when *this* year was earlier than last.
            let earliest: Option<(String, i64)> = conn
                .query_row(
                    "WITH first_this AS ( \
                       SELECT Com_Name, MIN(Date) d \
                       FROM detections_analytic WHERE Date LIKE ?1 GROUP BY Com_Name \
                     ), first_prior AS ( \
                       SELECT Com_Name, MIN(Date) d \
                       FROM detections_analytic WHERE Date LIKE ?2 GROUP BY Com_Name \
                     ) \
                     SELECT t.Com_Name, \
                            CAST(julianday(t.d) - julianday(?3 || substr(p.d, 5)) AS INTEGER) AS delta \
                     FROM first_this t JOIN first_prior p USING (Com_Name) \
                     ORDER BY delta ASC LIMIT 1",
                    [
                        format!("{year}-%"),
                        format!("{prior}-%"),
                        format!("{year}"),
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();

            // Still expected = species that arrived in the prior year within
            // the next six weeks of today, but we haven't heard them yet this
            // year. Six weeks from mid-November lands in January, so the window
            // has to be able to cross the year boundary.
            let still_expected = still_expected_count(conn, &today).unwrap_or(0);

            Ok::<_, rusqlite::Error>((foy, peak_week, peak_n, earliest, still_expected))
        })
    })
    .await;

    let (foy, peak_week, peak_n, earliest, still_expected) = result
        .ok()
        .and_then(Result::ok)
        .unwrap_or((0, 0, 0, None, 0));

    let earliest_html = match earliest {
        Some((species, delta)) if delta < 0 => format!(
            r#"<span class="value">{delta} d</span><span class="bnb-meta">{species}</span>"#,
            species = escape_html(&species),
        ),
        Some((species, 0)) => format!(
            r#"<span class="value">on time</span><span class="bnb-meta">{species}</span>"#,
            species = escape_html(&species),
        ),
        Some((species, delta)) => format!(
            r#"<span class="value">+{delta} d</span><span class="bnb-meta">{species}</span>"#,
            species = escape_html(&species),
        ),
        None => r#"<span class="value">—</span><span class="bnb-meta">no prior-year data</span>"#
            .to_string(),
    };

    let expected_html = if still_expected > 0 {
        format!(
            r#"<span class="value">{still_expected}</span><span class="bnb-meta">arriving within 6 wk</span>"#
        )
    } else {
        r#"<span class="value">0</span><span class="bnb-meta">no overdue migrants</span>"#
            .to_string()
    };

    let html = format!(
        r#"<div class="stat-tile"><span class="label">First-of-year arrivals</span><span class="value">{foy}</span></div>
<div class="stat-tile"><span class="label">Peak diversity week</span><span class="value mig-peak-val">w{peak_week}</span><span class="bnb-meta mono">{peak_n} species</span></div>
<div class="stat-tile"><span class="label">Earliest vs last year</span>{earliest_html}</div>
<div class="stat-tile"><span class="label">Still expected</span>{expected_html}</div>"#,
    );
    state
        .analytics_cache()
        .put("migration-stats".to_string(), html.clone());
    ok_html(html)
}

// ---------------------------------------------------------------------------
// Editorial cards
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CardQuery {
    kind: String,
}

async fn card_partial(
    State(state): State<AppState>,
    Query(q): Query<CardQuery>,
) -> impl IntoResponse {
    let year = current_year();
    let kind = q.kind;
    let kind_db = kind.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| match kind_db.as_str() {
            "arrived" => {
                // most recent species whose first-ever detection is in this year
                conn.query_row(
                    "SELECT Com_Name, MIN(Date) FROM detections_analytic \
                     GROUP BY Com_Name HAVING MIN(Date) LIKE ?1 \
                     ORDER BY MIN(Date) DESC LIMIT 1",
                    [format!("{year}-%")],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .ok()
            }
            "peaking" => conn
                .query_row(
                    "SELECT Com_Name, COUNT(*) AS n FROM detections_analytic \
                     WHERE Date >= date('now','localtime','-7 days') \
                     GROUP BY Com_Name ORDER BY n DESC LIMIT 1",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.to_string())),
                )
                .ok(),
            "missing" => None, // requires comparative model — stubbed.
            _ => None,
        })
    })
    .await;

    let (eyebrow, headline, sub) = match q_kind(&result, &kind) {
        Some((eb, hd, sb)) => (eb, hd, sb),
        None => match kind.as_str() {
            "arrived" => ("Just arrived", "No arrivals yet".into(), "Watching…".into()),
            "peaking" => (
                "Currently peaking",
                "Quiet week".into(),
                "No species over 10 detections".into(),
            ),
            _ => ("Missing", "—".into(), "Forecast model pending".into()),
        },
    };
    let html = format!(
        r#"<div class="bnb-eyebrow" data-style="color:{accent}">{eyebrow}</div>
<div class="display mig-headline">{headline}</div>
<div class="bnb-meta mig-sub-line">{sub}</div>"#,
        accent = card_color(&kind),
        eyebrow = escape_html(eyebrow),
        headline = escape_html(&headline),
        sub = escape_html(&sub),
    );
    ok_html(html)
}

fn q_kind(
    result: &Result<Option<(String, String)>, tokio::task::JoinError>,
    kind: &str,
) -> Option<(&'static str, String, String)> {
    let row = result.as_ref().ok()?.as_ref()?;
    match kind {
        "arrived" => Some((
            "Just arrived",
            row.0.clone(),
            format!("First heard {}", row.1),
        )),
        "peaking" => Some((
            "Currently peaking",
            row.0.clone(),
            format!("{} detections this week", row.1),
        )),
        _ => None,
    }
}

fn card_color(kind: &str) -> &'static str {
    match kind {
        "arrived" => "var(--moss-ink)",
        "peaking" => "var(--dawn-ink)",
        _ => "var(--rare)",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const MONTHS: &[(&str, u8)] = &[
    ("Jan", 0),
    ("Feb", 4),
    ("Mar", 9),
    ("Apr", 13),
    ("May", 17),
    ("Jun", 22),
    ("Jul", 26),
    ("Aug", 30),
    ("Sep", 35),
    ("Oct", 39),
    ("Nov", 44),
    ("Dec", 48),
];

/// How far ahead the "still expected" tile looks.
const WINDOW_DAYS: u64 = 42;

/// Species that arrived in the prior year within the next [`WINDOW_DAYS`] and
/// have not been heard yet this year.
///
/// `today` is the station's **local** date, passed in rather than read from the
/// clock inside the query. The old form compared day-of-year numbers against
/// `strftime('%j','now')` and `strftime('%j','now','+42 days')`, which fails
/// three ways at once:
///
/// - **It empties itself every December.** Once the window's end falls in the
///   next calendar year its day number is *smaller* than the start's — 20
///   November 2026 gives `'324' … '001'` — so `BETWEEN` matches nothing. The
///   tile read a confident zero for the last 42 days of every year, which is
///   precisely when the winter arrivals it exists to announce are still due.
/// - **`'now'` is UTC.** Detections carry a local `Date`, so the window moved
///   off the data by the station's offset for part of every day.
/// - **Day-of-year is not comparable between years.** After February a leap
///   year runs one day ahead, so the window edges landed a day out whenever
///   exactly one of the two years was a leap year.
///
/// All three go away by not doing day-of-year arithmetic at all. The prior
/// year's arrival date is re-based onto this year *and* onto next year — the
/// same `year || substr(date, 5)` idiom the "earliest vs last year" query above
/// already uses — and both candidates are compared against the real window
/// bounds. The wrap is then just the second candidate matching.
///
/// The comparison is lexicographic over ISO dates rather than `julianday`,
/// which is what keeps a 29 February arrival from vanishing: re-basing it onto
/// a common year yields `2027-02-29`, a date that does not exist and that
/// `julianday` would turn into NULL, but that sorts exactly where a reader
/// expects it.
fn still_expected_count(conn: &rusqlite::Connection, today: &str) -> rusqlite::Result<i64> {
    let year = year_of(today);
    let prior = year - 1;
    let next = year + 1;
    conn.query_row(
        "WITH heard_this AS ( \
           SELECT DISTINCT Com_Name FROM detections_analytic WHERE Date LIKE ?1 \
         ), arrived_prior AS ( \
           SELECT Com_Name, MIN(Date) d \
           FROM detections_analytic WHERE Date LIKE ?2 GROUP BY Com_Name \
         ) \
         SELECT COUNT(*) FROM arrived_prior \
         WHERE ((?3 || substr(d, 5)) BETWEEN ?5 AND ?6 \
             OR (?4 || substr(d, 5)) BETWEEN ?5 AND ?6) \
           AND Com_Name NOT IN (SELECT Com_Name FROM heard_this)",
        rusqlite::params![
            format!("{year}-%"),
            format!("{prior}-%"),
            format!("{year}"),
            format!("{next}"),
            today,
            window_end(today, WINDOW_DAYS),
        ],
        |r| r.get(0),
    )
}

/// The date `days` after `today`, as `YYYY-MM-DD`.
fn window_end(today: &str, days: u64) -> String {
    let (y, m, d) =
        crate::routes::pages::days_to_date(crate::routes::pages::date_to_epoch_days(today) + days);
    format!("{y}-{m:02}-{d:02}")
}

/// The four-digit year of a `YYYY-MM-DD` string, or 1970 if there isn't one.
///
/// Degrades rather than panicking: this module aborts the process on a panic,
/// so a corrupt `Date` must not reach one.
fn year_of(date: &str) -> i32 {
    date.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(1970)
}

/// Week-of-year of a `YYYY-MM-DD` date, matching `SQLite`'s `%W`.
///
/// This has to agree with `%W` because that is what every query on this page
/// buckets by, and the value positions the "today" marker against those
/// buckets. It did not: the marker was placed by `(unix_days % 365) / 7`, which
/// is not a week number in any calendar. It ignores leap days, so it had
/// drifted a fortnight by 2026, and it counts from 1 January 1970 rather than
/// from the current year, so on 31 December it returned week 1 and drew the
/// marker at the far left of a chart whose data ends at the far right.
///
/// Clamped to 51 because the callers index 52-slot arrays.
#[allow(clippy::cast_possible_truncation)]
fn week_of_year(date: &str) -> u8 {
    let days = crate::routes::pages::date_to_epoch_days(date);
    // 1970-01-01 was a Thursday, so `(days + 4) % 7` is 0 = Sunday; shift to
    // 0 = Monday, which is the week start `%W` uses.
    let monday_based = (((days + 4) % 7) + 6) % 7;
    let (y, _, _) = crate::routes::pages::days_to_date(days);
    let jan1 = crate::routes::pages::date_to_epoch_days(&format!("{y}-01-01"));
    let day_of_year_zero_based = days.saturating_sub(jan1);
    ((day_of_year_zero_based + 7 - monday_based) / 7).min(51) as u8
}

/// The station's current **local** year.
///
/// Local, not UTC. Every query on this page selects rows with `Date LIKE
/// '{year}-%'` against a locally-dated column, and this is also the year
/// [`still_expected_count`] re-bases prior arrivals onto — a UTC year would put
/// the two out of step for the hours around New Year, which is exactly the
/// boundary the rest of this change is about.
fn current_year() -> i32 {
    year_of(&crate::routes::pages::today_date_string())
}

/// The station's current **local** week-of-year, on `SQLite`'s `%W` scale.
fn current_week() -> u8 {
    week_of_year(&crate::routes::pages::today_date_string())
}

/// AOU-style 4-letter banding code. Re-exports the canonical helper from
/// `atoms.rs` so every page renders the same code for the same species.
/// Only referenced by tests — production paths call `species_code` directly.
#[cfg(test)]
fn alpha_code(name: &str) -> String {
    species_code(name)
}

fn ok_html(body: String) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}

/// Pre-compute and cache the heavy phenology fragments (ridgeline + diversity)
/// so the first visit (and each background refresh) is instant. The stats tiles
/// cache per-request; the editorial cards are cheap single-row lookups.
pub fn prewarm(state: &AppState) {
    let cache = state.analytics_cache();
    if let Some(h) = compute_ridgeline(state) {
        cache.put("migration-ridgeline".to_string(), h);
    }
    if let Some(h) = compute_diversity(state) {
        cache.put("migration-diversity".to_string(), h);
    }
}

#[cfg(test)]
mod tests {
    // The module-level deny exists because a panic in a request handler aborts
    // the whole station. A test is not a request handler, and a failed
    // assertion is how a test reports.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    #[test]
    fn alpha_code_two_words() {
        // Delegates to `species_code` from atoms.rs.
        assert_eq!(alpha_code("Yellow Warbler"), "YEWA");
    }
    #[test]
    fn alpha_code_one_word() {
        assert_eq!(alpha_code("Ovenbird"), "OVEN");
    }

    // -----------------------------------------------------------------------
    // Date handling across the year boundary
    // -----------------------------------------------------------------------

    /// A station holding `(common name, date)` first detections, behind the
    /// same `detections_analytic` view the page reads in production.
    fn station(rows: &[(&str, &str)]) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite");
        conn.execute_batch(
            "CREATE TABLE detections (Date TEXT, Com_Name TEXT, review_verdict TEXT);
             CREATE VIEW detections_analytic AS
                 SELECT * FROM detections WHERE review_verdict IS NOT 'rejected';",
        )
        .expect("schema");
        for (name, date) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Com_Name, review_verdict) VALUES (?1, ?2, NULL)",
                rusqlite::params![date, name],
            )
            .expect("seed");
        }
        conn
    }

    /// The six-week look-ahead must not empty itself in December.
    ///
    /// The window was expressed as a day-of-year `BETWEEN` against
    /// `strftime('%j','now')` and `strftime('%j','now','+42 days')`. From 20
    /// November the end of the window falls in the next calendar year, so its
    /// day number is *smaller* than the start's — 20 November 2026 gives
    /// `'324' … '001'` — and the range matches nothing. "Still expected" was
    /// therefore a structural zero for the last 42 days of every year, which is
    /// exactly the stretch when the winter arrivals it exists to announce are
    /// still to come.
    #[test]
    fn still_expected_survives_the_year_boundary() {
        let conn = station(&[
            // Prior year (2025) first arrivals.
            ("Late Arriver", "2025-12-20"),
            ("New Year Arriver", "2025-01-05"),
            ("Spring Arriver", "2025-05-01"),
            ("Already Heard", "2025-12-22"),
            // Heard again this year, so no longer expected.
            ("Already Heard", "2026-12-06"),
        ]);

        let n = still_expected_count(&conn, "2026-12-05").expect("query runs");
        assert_eq!(
            n, 2,
            "expected Late Arriver (20 Dec, this year) and New Year Arriver \
             (5 Jan, next year); Spring Arriver is out of window and Already \
             Heard has been heard"
        );
    }

    /// The counterpart: the window the old query *did* answer still answers.
    ///
    /// A fix that widened the window until December worked could satisfy the
    /// gate above while counting species that are months away. This case never
    /// crosses a year boundary and its answer must not move.
    #[test]
    fn still_expected_mid_year_window_is_unchanged() {
        let conn = station(&[
            ("In Window", "2025-05-20"),
            ("Too Early", "2025-04-01"),
            ("Too Late", "2025-07-01"),
            ("Already Heard", "2025-05-21"),
            ("Already Heard", "2026-05-02"),
        ]);

        let n = still_expected_count(&conn, "2026-05-01").expect("query runs");
        assert_eq!(n, 1, "only In Window falls in 1 May - 12 June");
    }

    /// 29 February in the prior year has no counterpart in a common year.
    ///
    /// Re-basing it produces `2027-02-29`, a date that does not exist. The
    /// comparison is lexicographic over ISO strings rather than `julianday`,
    /// which places the impossible date exactly where the reader expects it
    /// — after the 28th, before 1 March — instead of turning it into a NULL
    /// that drops the species out of the count.
    #[test]
    fn a_leap_day_arrival_still_counts_in_a_common_year() {
        let conn = station(&[("Leap Arriver", "2024-02-29")]);
        let n = still_expected_count(&conn, "2025-02-20").expect("query runs");
        assert_eq!(n, 1, "29 Feb 2024 falls inside 20 Feb - 3 Apr 2025");
    }

    /// `week_of_year` must agree with the `%W` the queries bucket by.
    ///
    /// The chart's "today" marker was placed by `(unix_days % 365) / 7`, which
    /// is not a week number: it ignores leap days, so it had drifted a fortnight
    /// by 2026, and it is anchored to 1 January 1970 rather than to the current
    /// year, so on 31 December it read week 1 and drew the marker at the far
    /// left of a chart whose data ends at the far right.
    #[test]
    fn week_of_year_agrees_with_sqlite() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory SQLite");
        for date in [
            "2026-01-01",
            "2026-01-04",
            "2026-01-05",
            "2026-08-18",
            "2026-12-31",
            "2024-02-29",
            "2024-12-31",
            "2021-01-03",
        ] {
            let sqlite: i64 = conn
                .query_row("SELECT CAST(strftime('%W', ?1) AS INTEGER)", [date], |r| {
                    r.get(0)
                })
                .expect("strftime");
            // The callers index 52-slot arrays, so weeks 52 and 53 clamp; the
            // agreement being checked is below that.
            assert_eq!(
                i64::from(week_of_year(date)),
                sqlite.min(51),
                "week of {date}"
            );
        }
    }

    /// The look-ahead end date is 42 days on, across a year boundary and a
    /// leap day alike.
    #[test]
    fn window_end_crosses_years_and_leap_days() {
        assert_eq!(window_end("2026-12-05", 42), "2027-01-16");
        assert_eq!(window_end("2026-05-01", 42), "2026-06-12");
        // 2024 is a leap year: 42 days from 20 February includes 29 February.
        assert_eq!(window_end("2024-02-20", 42), "2024-04-02");
        assert_eq!(window_end("2025-02-20", 42), "2025-04-03");
    }

    #[test]
    fn year_of_reads_the_leading_four_digits() {
        assert_eq!(year_of("2026-12-05"), 2026);
        // Degrades rather than panicking: this module aborts the process on a
        // panic, so a corrupt date must not reach one.
        assert_eq!(year_of("oops"), 1970);
    }
}
