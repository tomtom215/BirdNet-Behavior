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

// Adapted SVG-rendering module: int<->float coordinate casts, short math
// identifiers, and long path-builder functions are intrinsic to this code.
#![allow(clippy::pedantic, clippy::nursery)]

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::state::AppState;

use super::atoms::{species_code, species_color};
use super::{escape_html, render_page};

const PAGE_HTML: &str = include_str!("../../../templates/migration.html");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/migration", get(migration_page))
        .route("/pages/migration-stats", get(stats_partial))
        .route("/pages/migration-ridgeline", get(ridgeline_partial))
        .route("/pages/migration-diversity", get(diversity_partial))
        .route("/pages/migration-card", get(card_partial))
}

async fn migration_page() -> Html<String> {
    let year = current_year();
    // Skeleton placeholders (O-16) shown until the htmx swap targets load.
    // O-20 help link drops the methodology shortcut into the eyebrow.
    let body = PAGE_HTML
        .replace("{{year}}", &year.to_string())
        .replace("{{skel_migration_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_ridgeline}}", super::skeletons::ridgeline())
        .replace("{{skel_diversity}}", &super::skeletons::diversity_bars())
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::Phenology),
        );
    render_page("Migration", &body, "migration")
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
         FROM detections \
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
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = sorted[sorted.len() / 2];
            if peak / median.max(1.0) < 3.0 {
                return None;
            }
            let peak_week = weekly
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
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

async fn ridgeline_partial(State(state): State<AppState>) -> impl IntoResponse {
    let year = current_year();
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| collect_ridges(conn, year, 12)))
            .await;

    let ridges = match result {
        Ok(Ok(r)) if !r.is_empty() => r,
        _ => return empty_state("No migratory species detected yet this year."),
    };

    let today_week = current_week();
    let svg = render_ridgeline_svg(&ridges, today_week);
    ok_html(svg)
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
        r#"<svg viewBox="0 0 {W} {H}" width="100%" height="auto" preserveAspectRatio="none" style="display:block;">"#,
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
            r#"<text x="{x:.2}" y="{y}" text-anchor="middle" class="mono" style="font-size:11px;fill:var(--fg-3);">{label}</text>"#,
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
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono" style="font-size:10px;fill:var(--moss-ink);">spring migration</text>"#,
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
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono" style="font-size:10px;fill:var(--dawn-ink);">fall migration</text>"#,
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
            r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" style="font-size:12px;fill:var(--fg);font-weight:500;">{name}</text>"#,
            x = PAD_L - 8.0,
            y = y_base - 6.0,
            name = escape_html(&ridge.name),
        );
        let _ = write!(
            s,
            r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" class="mono" style="font-size:9.5px;fill:var(--fg-3);">{short} · peak w{pw}</text>"#,
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
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="middle" class="mono" style="font-size:10px;fill:var(--bg);">today</text></g>"#,
        x = x(today_week as f64),
        y = PAD_T + 8.0,
    );

    s.push_str("</svg>");
    s
}

// ---------------------------------------------------------------------------
// Diversity strip
// ---------------------------------------------------------------------------

async fn diversity_partial(State(state): State<AppState>) -> impl IntoResponse {
    let year = current_year();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT CAST(strftime('%W', Date) AS INTEGER) AS wk, \
                        COUNT(DISTINCT Com_Name) \
                 FROM detections \
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
    })
    .await;

    let weekly = match result {
        Ok(Ok(w)) => w,
        _ => return empty_state("No data for diversity bars yet."),
    };

    let today_week = current_week();
    let svg = render_diversity_svg(&weekly, today_week);
    ok_html(svg)
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
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="end" class="mono" style="font-size:9.5px;fill:var(--fg-3);">species / wk</text>"#,
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
    let year = current_year();
    let prior = year - 1;
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            // First-of-year arrivals = species with first_date in this year so far.
            let foy: i64 = conn.query_row(
                "SELECT COUNT(*) FROM ( \
                   SELECT Com_Name, MIN(Date) AS first FROM detections GROUP BY Com_Name \
                 ) WHERE first LIKE ?1",
                [format!("{year}-%")],
                |r| r.get(0),
            )?;
            // Peak diversity week.
            let (peak_week, peak_n): (i64, i64) = conn
                .query_row(
                    "SELECT CAST(strftime('%W', Date) AS INTEGER) wk, COUNT(DISTINCT Com_Name) n \
                     FROM detections WHERE Date LIKE ?1 GROUP BY wk ORDER BY n DESC LIMIT 1",
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
                       FROM detections WHERE Date LIKE ?1 GROUP BY Com_Name \
                     ), first_prior AS ( \
                       SELECT Com_Name, MIN(Date) d \
                       FROM detections WHERE Date LIKE ?2 GROUP BY Com_Name \
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
            // the next 42 days of today's date-of-year, but we haven't heard
            // them yet this year.
            let still_expected: i64 = conn.query_row(
                "WITH heard_this AS ( \
                   SELECT DISTINCT Com_Name FROM detections WHERE Date LIKE ?1 \
                 ), arrived_prior_window AS ( \
                   SELECT Com_Name, MIN(Date) d \
                   FROM detections WHERE Date LIKE ?2 GROUP BY Com_Name \
                   HAVING strftime('%j', d) >= strftime('%j', 'now') \
                      AND strftime('%j', d) <= strftime('%j', 'now', '+42 days') \
                 ) \
                 SELECT COUNT(*) FROM arrived_prior_window \
                 WHERE Com_Name NOT IN (SELECT Com_Name FROM heard_this)",
                [format!("{year}-%"), format!("{prior}-%")],
                |r| r.get(0),
            ).unwrap_or(0);

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
<div class="stat-tile"><span class="label">Peak diversity week</span><span class="value" style="font-size:28px;">w{peak_week}</span><span class="bnb-meta mono">{peak_n} species</span></div>
<div class="stat-tile"><span class="label">Earliest vs last year</span>{earliest_html}</div>
<div class="stat-tile"><span class="label">Still expected</span>{expected_html}</div>"#,
    );
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
                    "SELECT Com_Name, MIN(Date) FROM detections \
                     GROUP BY Com_Name HAVING MIN(Date) LIKE ?1 \
                     ORDER BY MIN(Date) DESC LIMIT 1",
                    [format!("{year}-%")],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .ok()
            }
            "peaking" => conn
                .query_row(
                    "SELECT Com_Name, COUNT(*) AS n FROM detections \
                     WHERE Date >= date('now','-7 days') \
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
        r#"<div class="bnb-eyebrow" style="color:{accent};">{eyebrow}</div>
<div class="display" style="font-size:20px;margin-top:8px;">{headline}</div>
<div class="bnb-meta" style="margin-top:4px;">{sub}</div>"#,
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

fn current_year() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, _, _) = crate::routes::pages::days_to_date(secs / 86400);
    y as i32
}

fn current_week() -> u8 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Approximate ISO-ish week; SQLite's %W is what the queries use.
    let day_of_year = ((secs / 86400) % 365) as u32;
    (day_of_year / 7).min(51) as u8
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

fn empty_state(msg: &str) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let body = format!(r#"<p class="bnb-meta">{}</p>"#, escape_html(msg));
    ok_html(body)
}

#[cfg(test)]
mod tests {
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
}
