//! Dawn-chorus page + partials.
//!
//! Mounts:
//!   GET /analytics/dawn-chorus          full page
//!   GET /pages/dawn-polar               stacked polar ribbon SVG
//!   GET /pages/dawn-list                right-rail per-species strips
//!
//! Sunrise/sunset are taken from the configured station lat/lon if available;
//! falls back to a conservative 05:30 / 20:00.
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
// identifiers, and long polar-path builders are intrinsic to this code.
#![allow(clippy::pedantic, clippy::nursery)]

use std::f64::consts::PI;
use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::{Router, routing::get};

use crate::state::AppState;

use super::atoms::series_color;
use super::escape_html;

const PAGE_HTML: &str = include_str!("../../../templates/dawn_chorus.html");

/// Mount the Dawn Chorus page and its HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/dawn-polar", get(polar_partial))
        .route("/pages/dawn-list", get(list_partial))
}

/// The dawn-chorus surface, rendered for embedding by `homes::patterns`
/// ("Dawn chorus" tab).
pub(super) fn content() -> String {
    // Skeleton placeholders (O-16) shown until the htmx swap targets load.
    // O-23 moon badge — pure local computation, always safe to show.
    // O-20 help link wires the eyebrow to the mdBook page.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |x| i64::try_from(x.as_secs()).unwrap_or(i64::MAX));
    let moon_badge = super::overlays::moon_badge(now_secs);
    PAGE_HTML
        .replace("{{skel_polar}}", super::skeletons::polar_plot())
        .replace("{{skel_ribbons}}", &super::skeletons::species_ribbons(6))
        .replace("{{moon_badge}}", &moon_badge)
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::DawnChorus),
        )
}

// ---------------------------------------------------------------------------
// Data shape
// ---------------------------------------------------------------------------

struct ChorusRibbon {
    name: String,
    color: String,
    hours: [f32; 24],
    peak_hour: u8,
    total: i64,
}

/// The windowed chorus aggregate.
///
/// A `const` rather than a literal inline below so
/// `dawn_chorus_window_uses_a_date_range_seek` can `EXPLAIN` *this* statement.
/// A test that re-types the SQL asserts the plan of its own copy and stays
/// green while the shipped query regresses — which is exactly what happened to
/// the first draft of that test.
///
/// `INDEXED BY` is not decoration. Left to itself SQLite picks
/// `idx_detections_species` to get `Com_Name` in order, which makes this a full
/// scan of the station's *entire* history to answer a question about the last
/// `days` days — and it builds the temp b-tree anyway, because `hr` is an
/// expression, so the choice buys nothing at all.
///
/// Measured on x86_64 against a synthetic four-year station (1 095 361 rows,
/// 277 MB), 30-day window:
///
/// ```text
/// as shipped                    1613 ms   SCAN … idx_detections_species
/// INDEXED BY (Date, Com_Name)     27 ms   SEARCH … (Date>?)
/// ```
///
/// Identical results (2097 groups) both ways. The cost of the unhinted form
/// scales with total history, not with the window: 72 ms at 60 days, 396 ms at
/// one year, 1711 ms at four — so a permanent station's dawn chorus gets
/// steadily slower every season it runs. `ANALYZE` does not change the plan;
/// this was checked.
///
/// The hint is safe because migrations never alter an existing one (see
/// `MIGRATIONS` in `birdnet-db`).
///
/// # Why the base table rather than `detections_analytic`
///
/// Every other aggregate reads the `detections_analytic` view, which applies
/// the reviewer-verdict exclusion once and centrally. This one cannot:
/// `INDEXED BY` is not valid against a view, and dropping the hint costs a 60x
/// slowdown that grows with the station's history (see the numbers above). So
/// the whole predicate is spelled out inline instead — both of the view's
/// clauses, the null-safe `IS NOT` for the same three-valued-logic reason, and
/// the provenance subquery migration 34 added.
///
/// Two spellings of one rule is the shape this repository keeps paying for, and
/// it charged again here: the verdict clause was copied and the provenance one
/// was not, while this comment went on saying "same predicate the view applies".
/// The chorus was then the single surface still counting another site's records
/// after the operator excluded them. `the_inline_predicate_and_the_view_admit_the_same_rows`
/// now holds the two against each other on the same rows rather than asserting
/// each separately, so a third clause added to the view cannot be missed here.
const CHORUS_SQL: &str = "SELECT Com_Name, CAST(strftime('%H', Time) AS INTEGER) hr, COUNT(*) n \
     FROM detections INDEXED BY idx_detections_date_species \
     WHERE Date >= date('now','localtime', ?1) \
       AND review_verdict IS NOT 'rejected' \
       AND (import_batch_id IS NULL \
            OR NOT EXISTS (SELECT 1 FROM settings \
                            WHERE key = 'analytics_exclude_imports' \
                              AND value = 'true')) \
     GROUP BY Com_Name, hr";

fn collect_chorus(
    conn: &rusqlite::Connection,
    days: i64,
    top_n: usize,
) -> rusqlite::Result<Vec<ChorusRibbon>> {
    let mut stmt = conn.prepare(CHORUS_SQL)?;
    let modifier = format!("-{days} days");
    let rows = stmt.query_map([&modifier], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    use std::collections::HashMap;
    let mut by_species: HashMap<String, [f32; 24]> = HashMap::new();
    let mut totals: HashMap<String, i64> = HashMap::new();
    for row in rows.flatten() {
        let (name, hr, n) = row;
        let h = hr.clamp(0, 23) as usize;
        let entry = by_species.entry(name.clone()).or_insert([0.0; 24]);
        entry[h] += n as f32;
        *totals.entry(name).or_insert(0) += n;
    }

    let mut ribbons: Vec<ChorusRibbon> = by_species
        .into_iter()
        .map(|(name, hours)| {
            let peak_hour = hours
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)
                .unwrap_or(0) as u8;
            let total = *totals.get(&name).unwrap_or(&0);
            ChorusRibbon {
                // Placeholder; replaced by rank below once the ribbons are
                // sorted, because the colour has to be assigned by position in
                // *this* chart. See `atoms::series_color`.
                color: String::new(),
                name,
                hours,
                peak_hour,
                total,
            }
        })
        .collect();

    ribbons.sort_by_key(|r| std::cmp::Reverse(r.total));
    ribbons.truncate(top_n);
    // Colour by rank within the chart, not by hashing the name: several species
    // are drawn as adjacent rings here, and the hash palette put pairs of them
    // 2–3° apart in hue at constant lightness.
    let shown = ribbons.len();
    for (rank, ribbon) in ribbons.iter_mut().enumerate() {
        ribbon.color = series_color(rank, shown);
    }
    Ok(ribbons)
}

// ---------------------------------------------------------------------------
// Polar SVG
// ---------------------------------------------------------------------------

async fn polar_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| (collect_chorus(conn, 60, 8), super::solar_times_local(conn)))
    })
    .await;

    let (ribbons, solar) = match result {
        Ok((Ok(r), solar)) if !r.is_empty() => (r, solar),
        _ => return ok_html(super::empty_states::no_chorus()),
    };

    // `solar_times_local` reads the station's configured coordinates and returns
    // **local** hours — the same clock the ribbons are bucketed in. Without a
    // configured location there is no honest sun position to draw, so the
    // markers are omitted rather than invented: the previous fallback drew a
    // (40.0 N, -74.0 W) sun over every station on Earth.
    let (sunrise, sunset) = match solar {
        Some(pair) => pair,
        None => return ok_html(render_polar_svg(&ribbons, None)),
    };
    ok_html(render_polar_svg(&ribbons, Some((sunrise, sunset))))
}

/// Render the polar chorus clock.
///
/// `solar` is `Some((sunrise_h, sunset_h))` in **local** fractional hours — the
/// same clock the ribbons are bucketed in — or `None` when the station has no
/// configured location. `None` omits the night wedge and both sun markers
/// rather than drawing a default position: this chart is read to decide when a
/// species sings relative to sunrise, and a sun drawn at a coordinate the
/// station is not at answers that question wrongly while looking authoritative.
fn render_polar_svg(ribbons: &[ChorusRibbon], solar: Option<(f64, f64)>) -> String {
    const SIZE: f64 = 520.0;
    const CX: f64 = SIZE / 2.0;
    const CY: f64 = SIZE / 2.0;
    const RING_MIN: f64 = 70.0;
    const RING_MAX: f64 = 220.0;
    let ring_step = (RING_MAX - RING_MIN) / (ribbons.len() as f64 + 1.0);

    let mut s = String::with_capacity(16 * 1024);
    let sun_strs = solar.map(|(rise, set)| (fmt_hour(rise), fmt_hour(set)));

    match sun_strs.as_ref() {
        Some((sr, ss)) => {
            let _ = write!(
                s,
                r#"<svg viewBox="0 0 {SIZE} {SIZE}" width="100%" height="100%" class="dc-polar-svg" data-sunrise="{sr}" data-sunset="{ss}">"#,
            );
        }
        None => {
            let _ = write!(
                s,
                r#"<svg viewBox="0 0 {SIZE} {SIZE}" width="100%" height="100%" class="dc-polar-svg">"#,
            );
        }
    }

    // Night wedge — only when the sun's position is actually known.
    if let Some((sunrise_h, sunset_h)) = solar {
        let a1 = hour_to_angle(sunset_h);
        let a2 = hour_to_angle(sunrise_h + 24.0);
        let r_outer = RING_MAX + 14.0;
        let (x1, y1) = polar(CX, CY, a1, r_outer);
        let (x2, y2) = polar(CX, CY, a2, r_outer);
        let sweep = (a2 - a1).rem_euclid(2.0 * PI);
        let large = if sweep > PI { 1 } else { 0 };
        let _ = write!(
            s,
            r#"<path d="M{CX},{CY} L{x1:.2},{y1:.2} A{r:.2},{r:.2} 0 {large} 1 {x2:.2},{y2:.2} Z" fill="var(--night)" fill-opacity="0.06"/>"#,
            r = r_outer,
        );
    }

    // O-23 follow-up — 4-segment outer moon-phase arc.
    //
    // The four segments map to the cardinal lunar phases (new / waxing
    // half / full / waning half) and sit at the cardinal positions of
    // the polar clock (top / right / bottom / left). The current
    // phase's segment is drawn brightest; the other three fade so the
    // wheel still reads as "where we are in the cycle".
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let phase = super::overlays::moon_phase_at(now_secs);
    let active = super::overlays::MoonCardinal::from_phase(phase);
    let r_moon = RING_MAX + 40.0;
    // Each segment spans 90° minus a small gap so adjacent arcs read
    // as distinct rather than a single continuous ring.
    let gap = 0.08_f64; // ~4.6°
    let half_span = PI / 4.0 - gap / 2.0;
    let centers = [
        (super::overlays::MoonCardinal::New, -PI / 2.0, "○"),
        (super::overlays::MoonCardinal::WaxingHalf, 0.0, "◐"),
        (super::overlays::MoonCardinal::Full, PI / 2.0, "●"),
        (super::overlays::MoonCardinal::WaningHalf, PI, "◑"),
    ];
    for (card, center, glyph) in centers {
        let a_start = center - half_span;
        let a_end = center + half_span;
        let (sx, sy) = polar(CX, CY, a_start, r_moon);
        let (ex, ey) = polar(CX, CY, a_end, r_moon);
        let is_active = card == active;
        // Active segment: bright moon tint; others: muted hairline.
        let (stroke, sw, op) = if is_active {
            ("var(--dawn-ink, var(--fg))", 3.0, 0.92)
        } else {
            ("var(--fg-3)", 1.6, 0.32)
        };
        let _ = write!(
            s,
            r#"<path d="M{sx:.2},{sy:.2} A{r:.2},{r:.2} 0 0 1 {ex:.2},{ey:.2}" stroke="{stroke}" stroke-width="{sw}" stroke-opacity="{op}" fill="none" stroke-linecap="round" data-moon-segment="{seg}"/>"#,
            r = r_moon,
            seg = match card {
                super::overlays::MoonCardinal::New => "new",
                super::overlays::MoonCardinal::WaxingHalf => "waxing-half",
                super::overlays::MoonCardinal::Full => "full",
                super::overlays::MoonCardinal::WaningHalf => "waning-half",
            },
        );
        // Glyph label slightly outside the arc, on the cardinal axis.
        let (gx, gy) = polar(CX, CY, center, r_moon + 16.0);
        let glyph_op = if is_active { 0.95 } else { 0.4 };
        let _ = write!(
            s,
            r#"<text x="{gx:.2}" y="{gy:.2}" text-anchor="middle" dominant-baseline="central" data-style="font-size:13px;fill:var(--fg-2);fill-opacity:{glyph_op};">{glyph}</text>"#,
        );
    }

    // Hour ticks + labels.
    for h in 0..24 {
        let a = hour_to_angle(h as f64);
        let big = h % 6 == 0;
        let (r1, r2) = (RING_MAX + 6.0, RING_MAX + 14.0);
        let (tx1, ty1) = polar(CX, CY, a, r1);
        let (tx2, ty2) = polar(CX, CY, a, r2);
        let _ = write!(
            s,
            r#"<line x1="{tx1:.2}" y1="{ty1:.2}" x2="{tx2:.2}" y2="{ty2:.2}" stroke="{stroke}" stroke-width="{sw}"/>"#,
            stroke = if big { "var(--fg-3)" } else { "var(--border)" },
            sw = if big { 1.0 } else { 0.5 },
        );
    }
    for h in [0, 3, 6, 9, 12, 15, 18, 21] {
        let a = hour_to_angle(h as f64);
        let (lx, ly) = polar(CX, CY, a, RING_MAX + 26.0);
        let label = match h {
            0 => "12a".to_string(),
            12 => "12p".to_string(),
            h if h < 12 => format!("{h}a"),
            h => format!("{}p", h - 12),
        };
        let _ = write!(
            s,
            r#"<text x="{lx:.2}" y="{ly:.2}" text-anchor="middle" dominant-baseline="central" class="mono dcv-hr">{label}</text>"#,
        );
    }

    // Sun markers — omitted with the wedge when there is no configured location.
    if let (Some((sunrise_h, sunset_h)), Some((sunrise_str, sunset_str))) =
        (solar, sun_strs.as_ref())
    {
        write_sun_marker(
            &mut s,
            CX,
            CY,
            sunrise_h,
            RING_MAX + 14.0,
            "rise",
            sunrise_str,
        );
        write_sun_marker(&mut s, CX, CY, sunset_h, RING_MAX + 14.0, "set", sunset_str);
    }

    // Ribbons — outer = first ribbon (highest total).
    for (i, ribbon) in ribbons.iter().enumerate() {
        let base_r = RING_MAX - (i as f64 + 1.0) * ring_step;
        let path = ribbon_path(CX, CY, base_r, &ribbon.hours, ring_step * 1.1);
        let _ = write!(
            s,
            r#"<path d="{path}" fill="{c}" fill-opacity="0.55" stroke="{c}" stroke-opacity="0.85" stroke-width="0.8"/>"#,
            c = ribbon.color,
        );
    }

    // Center disc + label.
    let _ = write!(
        s,
        r#"<circle cx="{CX}" cy="{CY}" r="{r:.2}" fill="var(--surface)" stroke="var(--hairline)"/>"#,
        r = RING_MIN - 8.0,
    );
    let _ = write!(
        s,
        r#"<text x="{CX}" y="{cy:.2}" text-anchor="middle" class="display dcv-ctr">chorus</text>"#,
        cy = CY - 10.0,
    );
    let _ = write!(
        s,
        r#"<text x="{CX}" y="{cy:.2}" text-anchor="middle" class="mono dcv-ctr-sub">24 h</text>"#,
        cy = CY + 10.0,
    );

    // Current-time hand.
    let now_h = current_hour_decimal();
    let a = hour_to_angle(now_h);
    let (hx1, hy1) = polar(CX, CY, a, RING_MIN - 4.0);
    let (hx2, hy2) = polar(CX, CY, a, RING_MAX + 14.0);
    let _ = write!(
        s,
        r#"<line x1="{hx1:.2}" y1="{hy1:.2}" x2="{hx2:.2}" y2="{hy2:.2}" stroke="var(--fg)" stroke-width="1.5" stroke-dasharray="2 3"/>"#,
    );
    let _ = write!(
        s,
        r#"<circle cx="{hx2:.2}" cy="{hy2:.2}" r="3" fill="var(--fg)"/>"#,
    );

    s.push_str("</svg>");
    s
}

fn write_sun_marker(s: &mut String, cx: f64, cy: f64, h: f64, r: f64, kind: &str, label: &str) {
    let a = hour_to_angle(h);
    let (x, y) = polar(cx, cy, a, r);
    let fill = if kind == "rise" {
        "var(--dawn)"
    } else {
        "var(--dawn-ink)"
    };
    let dy = if a.sin() > 0.0 { 16.0 } else { -10.0 };
    let icon = if kind == "rise" { "☼" } else { "☾" };
    let _ = write!(
        s,
        r#"<circle cx="{x:.2}" cy="{y:.2}" r="3.5" fill="{fill}"/>"#,
    );
    let _ = write!(
        s,
        r#"<text x="{x:.2}" y="{ty:.2}" text-anchor="middle" class="mono dcv-sun">{icon} {kind} {label}</text>"#,
        ty = y + dy,
    );
}

fn ribbon_path(cx: f64, cy: f64, base_r: f64, hours: &[f32; 24], amp: f64) -> String {
    let max = hours.iter().cloned().fold(0.001f32, f32::max);
    let subdiv = 4;
    let n_pts = 24 * subdiv + 1;
    let mut outer = Vec::with_capacity(n_pts);
    let mut inner = Vec::with_capacity(n_pts);
    for i in 0..n_pts {
        let h = (i as f64 / subdiv as f64) % 24.0;
        let fl = h.floor() as usize;
        let t = h - fl as f64;
        let v = ((hours[fl % 24] as f64 * (1.0 - t) + hours[(fl + 1) % 24] as f64 * t)
            / max as f64)
            .clamp(0.0, 1.0);
        let a = hour_to_angle(h);
        let r_o = base_r + v * amp * 0.95;
        let r_i = base_r - v * amp * 0.15;
        outer.push(polar(cx, cy, a, r_o));
        inner.push(polar(cx, cy, a, r_i));
    }
    let mut d = String::from("M");
    for (j, (x, y)) in outer.iter().enumerate() {
        if j == 0 {
            d.push_str(&format!("{x:.2},{y:.2}"));
        } else {
            d.push_str(&format!(" L{x:.2},{y:.2}"));
        }
    }
    for (x, y) in inner.iter().rev() {
        d.push_str(&format!(" L{x:.2},{y:.2}"));
    }
    d.push_str(" Z");
    d
}

// ---------------------------------------------------------------------------
// Right-rail list
// ---------------------------------------------------------------------------

async fn list_partial(State(state): State<AppState>) -> impl IntoResponse {
    let result =
        tokio::task::spawn_blocking(move || state.with_db(|conn| collect_chorus(conn, 60, 8)))
            .await;
    let ribbons = match result {
        Ok(Ok(r)) if !r.is_empty() => r,
        _ => return ok_html(super::empty_states::no_chorus()),
    };

    let mut s = String::new();
    for (i, ribbon) in ribbons.iter().enumerate() {
        let row_cls = if i == 0 { "dc-row first" } else { "dc-row" };
        let off_chorus = !(5..=8).contains(&ribbon.peak_hour);
        let peak_label = fmt_hour(ribbon.peak_hour as f64);
        let strip = render_hour_strip(&ribbon.hours, &ribbon.color);
        let _ = write!(
            s,
            r#"<div class="{row_cls}">
<span class="bnb-avatar" data-style="--sp:{color};">{short}</span>
<div class="dc-row-main">
  <div class="dc-row-head">
    <span class="dc-row-name">{name}</span>
    <span class="bnb-meta mono">peak {peak}</span>
    {off_chorus_html}
  </div>
  <div class="dc-strip">{strip}</div>
</div>
<span class="mono tabular bnb-meta">{total}</span>
</div>"#,
            row_cls = row_cls,
            color = ribbon.color,
            short = escape_html(&alpha_code(&ribbon.name)),
            name = escape_html(&ribbon.name),
            peak = peak_label,
            off_chorus_html = if off_chorus {
                r#"<span class="bnb-pill dc-offchorus">off-chorus</span>"#
            } else {
                ""
            },
            strip = strip,
            total = ribbon.total,
        );
    }
    ok_html(s)
}

fn render_hour_strip(hours: &[f32; 24], color: &str) -> String {
    let max = hours.iter().cloned().fold(0.001f32, f32::max);
    let mut s = String::from(
        r#"<svg width="100%" height="14" viewBox="0 0 240 14" preserveAspectRatio="none">"#,
    );
    for (i, v) in hours.iter().enumerate() {
        let op = 0.08 + (*v as f64 / max as f64) * 0.82;
        let x = (i as f64 / 24.0) * 240.0;
        let _ = write!(
            s,
            r#"<rect x="{x:.2}" y="0" width="{w:.2}" height="14" rx="1.5" fill="{c}" fill-opacity="{op:.3}"/>"#,
            w = 240.0 / 24.0 - 0.6,
            c = color,
        );
    }
    s.push_str("</svg>");
    s
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hour_to_angle(h: f64) -> f64 {
    (h / 24.0) * 2.0 * PI - PI / 2.0
}

fn polar(cx: f64, cy: f64, angle: f64, r: f64) -> (f64, f64) {
    (cx + r * angle.cos(), cy + r * angle.sin())
}

fn fmt_hour(h: f64) -> String {
    let total_minutes = (h * 60.0).round() as i32;
    let hh = (total_minutes / 60).rem_euclid(24);
    let mm = total_minutes.rem_euclid(60);
    format!("{hh:02}:{mm:02}")
}

fn current_hour_decimal() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs_today = (secs % 86400) as f64;
    secs_today / 3600.0
}

fn alpha_code(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let code = match words.len() {
        1 => words[0].chars().take(4).collect::<String>(),
        2 => format!(
            "{}{}",
            words[0].chars().take(2).collect::<String>(),
            words[1].chars().take(2).collect::<String>()
        ),
        _ => words
            .iter()
            .take(4)
            .map(|w| w.chars().next().unwrap_or(' '))
            .collect(),
    };
    code.to_uppercase()
}

fn ok_html(body: String) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hour_angles() {
        assert!((hour_to_angle(0.0) - (-PI / 2.0)).abs() < 1e-9);
        assert!((hour_to_angle(6.0) - 0.0).abs() < 1e-9);
        assert!((hour_to_angle(12.0) - (PI / 2.0)).abs() < 1e-9);
    }
    #[test]
    fn fmt_hour_basic() {
        assert_eq!(fmt_hour(5.5), "05:30");
        assert_eq!(fmt_hour(20.13), "20:08");
        assert_eq!(fmt_hour(0.0), "00:00");
    }

    /// The sun markers come from the station's *configured* location, through
    /// the same helper the Today page uses.
    ///
    /// This page used to compute them itself, and that private copy was wrong
    /// three ways at once: it read `BNB_STATION_LAT`/`BNB_STATION_LON` only and
    /// otherwise defaulted to a hard-coded (40.0 N, -74.0 W), so a station that
    /// set its location in the setup wizard got a New Jersey sun; its
    /// day-of-year was `((unix_secs / 86_400) % 365) + 1`, drifting ~1 day/year
    /// (14 days out by 2026, wrapping to January in late December); and it
    /// returned UTC hours while the ribbons it was drawn over are bucketed from
    /// the local `Time` column. Its own tests asserted UTC while its doc comment
    /// claimed "local-civil hours".
    #[test]
    fn the_sun_markers_come_from_the_configured_station_location() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        // No location configured: nothing honest to draw.
        assert!(
            super::super::solar_times_local(&conn).is_none(),
            "with no configured location there is no sun position to report"
        );
        birdnet_db::settings::set(
            &conn,
            "latitude",
            "51.5074",
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("lat");
        birdnet_db::settings::set(
            &conn,
            "longitude",
            "-0.1278",
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("lon");
        let (rise, set) =
            super::super::solar_times_local(&conn).expect("a configured location yields times");
        assert!(
            (0.0..24.0).contains(&rise) && (0.0..24.0).contains(&set),
            "hours must be folded onto the axis the chart draws: rise={rise} set={set}"
        );
        // London is never dark all day nor light all day; the sun rises in the
        // morning half and sets in the afternoon half, in local hours.
        assert!(rise < 12.0, "sunrise should be a morning hour, got {rise}");
        assert!(set > 12.0, "sunset should be an afternoon hour, got {set}");
    }

    /// Without a location the wedge and both markers are omitted rather than
    /// drawn at a default position. This chart is read to decide when a species
    /// sings *relative to sunrise*; a sun drawn where the station is not
    /// answers that wrongly while looking authoritative.
    #[test]
    fn no_location_draws_no_sun() {
        let svg = render_polar_svg(&[], None);
        assert!(!svg.contains("data-sunrise"), "no sunrise attribute");
        assert!(!svg.contains(r#"data-sun="rise""#), "no rise marker");
        assert!(!svg.contains("var(--night)"), "no night wedge");
        // The rest of the clock still renders.
        assert!(svg.contains(r#"data-moon-segment="new""#));
    }

    #[test]
    fn a_known_location_draws_both_markers() {
        let svg = render_polar_svg(&[], Some((6.0, 18.0)));
        assert!(svg.contains(r#"data-sunrise="06:00""#), "{svg:.400}");
        assert!(svg.contains(r#"data-sunset="18:00""#));
        assert!(svg.contains("var(--night)"), "night wedge is drawn");
    }

    #[test]
    fn render_polar_svg_includes_four_moon_segments() {
        // Empty ribbons + arbitrary sunrise/sunset are enough — the
        // moon ring is independent of the chorus data.
        let svg = render_polar_svg(&[], Some((6.0, 18.0)));
        assert!(svg.contains(r#"data-moon-segment="new""#));
        assert!(svg.contains(r#"data-moon-segment="waxing-half""#));
        assert!(svg.contains(r#"data-moon-segment="full""#));
        assert!(svg.contains(r#"data-moon-segment="waning-half""#));
        // Exactly one segment carries the "active" stroke width (3.0).
        // The inactive stroke is 1.6; assert that the active form
        // appears once.
        let active_count = svg.matches("stroke-width=\"3\"").count();
        assert_eq!(active_count, 1, "expected exactly one active moon segment");
    }

    /// The chorus is the one aggregate that cannot read `detections_analytic`
    /// (`INDEXED BY` is invalid on a view), so it spells the reviewer-verdict
    /// exclusion out inline. This holds that copy in step with the view.
    ///
    /// Without it the chorus would be the single surface where a rejected
    /// detection kept counting — the least likely place anyone would look for
    /// the discrepancy, and the most likely to be believed.
    #[test]
    fn dawn_chorus_excludes_rejected_detections() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        for (sci, com, verdict) in [
            ("Turdus merula", "Eurasian Blackbird", None),
            ("Erithacus rubecula", "European Robin", Some("rejected")),
            ("Parus major", "Great Tit", Some("confirmed")),
        ] {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, review_verdict) \
                 VALUES (date('now','localtime'), '06:00:00', ?1, ?2, 0.9, ?3)",
                rusqlite::params![sci, com, verdict],
            )
            .expect("seed");
        }

        let names: Vec<String> = collect_chorus(&conn, 30, 10)
            .expect("chorus")
            .into_iter()
            .map(|r| r.name)
            .collect();

        assert!(
            !names.iter().any(|n| n == "European Robin"),
            "a rejected detection is still in the dawn chorus: {names:?}"
        );
        assert_eq!(
            names.len(),
            2,
            "unreviewed and confirmed detections must both stay: {names:?}"
        );
    }

    /// A station with one detection of its own and one imported from elsewhere.
    fn station_with_an_import(exclude: Option<&str>) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute(
            "INSERT INTO import_batches (id, source_kind, row_count) VALUES (1, 'birdnet-pi', 1)",
            [],
        )
        .expect("batch");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
             VALUES (date('now','localtime'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .expect("seed local");
        conn.execute(
            "INSERT INTO detections \
               (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id) \
             VALUES (date('now','localtime'), '06:00:00', 'Parus major', 'Great Tit', 0.9, 1)",
            [],
        )
        .expect("seed import");
        if let Some(value) = exclude {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('analytics_exclude_imports', ?1)",
                rusqlite::params![value],
            )
            .expect("setting");
        }
        conn
    }

    /// Migration 34 gave `detections_analytic` a *second* rule — imported rows
    /// are excluded when the operator has asked for it — and the inline copy
    /// above was never taught it, while the comment beside it went on claiming
    /// "same predicate the view applies".
    ///
    /// So the dawn chorus was the one surface that kept counting another site's
    /// records after the operator excluded them. `provenance.rs` warns before an
    /// import that this damage "is not detectable after the fact"; a chart that
    /// quietly disagrees with every other chart is exactly that.
    #[test]
    fn dawn_chorus_excludes_an_import_the_operator_excluded() {
        let conn = station_with_an_import(Some("true"));
        let names: Vec<String> = collect_chorus(&conn, 30, 10)
            .expect("chorus")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert!(
            !names.iter().any(|n| n == "Great Tit"),
            "an excluded import is still in the dawn chorus: {names:?}"
        );
        assert_eq!(
            names.len(),
            1,
            "the station's own detection must stay: {names:?}"
        );
    }

    /// The counterpart, and the reason the fix cannot be "drop imported rows".
    ///
    /// Including an import is the default and a legitimate choice — merging two
    /// sites is a thing operators do, and only they know whether these are one
    /// site with a moved GPS fix or two a county apart. Both cases must render.
    #[test]
    fn dawn_chorus_keeps_an_import_the_operator_kept() {
        for setting in [None, Some("false"), Some("yes")] {
            let conn = station_with_an_import(setting);
            let names: Vec<String> = collect_chorus(&conn, 30, 10)
                .expect("chorus")
                .into_iter()
                .map(|r| r.name)
                .collect();
            assert!(
                names.iter().any(|n| n == "Great Tit"),
                "setting {setting:?} is not \"true\", so the import counts: {names:?}"
            );
            assert_eq!(names.len(), 2, "setting {setting:?}: {names:?}");
        }
    }

    /// The two spellings of one rule must agree, on the same rows.
    ///
    /// The gates above pin the chorus. This one pins the chorus *against the
    /// view*, which is what the comment beside `CHORUS_SQL` actually promises —
    /// and is the check that would have caught migration 34 adding a clause to
    /// one and not the other.
    #[test]
    fn the_inline_predicate_and_the_view_admit_the_same_rows() {
        for setting in [None, Some("true"), Some("false")] {
            let conn = station_with_an_import(setting);
            let from_view: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM detections_analytic \
                      WHERE Date >= date('now','localtime','-30 days')",
                    [],
                    |r| r.get(0),
                )
                .expect("view count");
            let from_chorus: i64 = collect_chorus(&conn, 30, 10)
                .expect("chorus")
                .len()
                .try_into()
                .expect("fits");
            assert_eq!(
                from_chorus, from_view,
                "setting {setting:?}: the chorus admitted {from_chorus} species \
                 where detections_analytic admits {from_view} rows (one species each)"
            );
        }
    }

    /// The windowed chorus query must seek a date range, never scan the table.
    ///
    /// Without the `INDEXED BY` hint SQLite scans `idx_detections_species` end
    /// to end, so the cost of a 30-day question grows with the station's whole
    /// history — 72 ms at 60 days, 1711 ms at four years on a synthetic
    /// 1.1 M-row station. This asserts the *plan*, not a duration, because a
    /// timing threshold on shared CI hardware is a flaky test and the plan is
    /// the thing that actually regressed.
    ///
    /// Two rows are enough: the planner makes the same wrong choice on an empty
    /// table as on a million rows (checked both ways), because it is preferring
    /// the species index for GROUP BY ordering rather than reasoning about
    /// selectivity — and it still builds the temp b-tree regardless, so the
    /// preference buys nothing.
    #[test]
    fn dawn_chorus_window_uses_a_date_range_seek() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
             VALUES (date('now','localtime'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
            [],
        )
        .expect("seed");

        // `collect_chorus` still answers correctly through the hint.
        let ribbons = collect_chorus(&conn, 30, 10).expect("chorus");
        assert_eq!(ribbons.len(), 1, "the seeded species is in the window");

        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {CHORUS_SQL}"))
            .expect("prepare")
            .query_map(["-30 days"], |r| r.get::<_, String>(3))
            .expect("plan")
            .filter_map(Result::ok)
            .collect();
        let joined = plan.join(" | ");
        assert!(
            joined.contains("SEARCH") && joined.contains("idx_detections_date_species"),
            "the chorus window no longer seeks the (Date, Com_Name) index — it \
             is scanning the whole history again. Plan was: {joined}"
        );
        assert!(
            !joined.contains("SCAN detections"),
            "the chorus window is scanning the detections table: {joined}"
        );
    }
}
