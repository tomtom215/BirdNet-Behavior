//! Dawn-chorus page + partials.
//!
//! Mounts:
//!   GET /analytics/dawn-chorus          full page
//!   GET /pages/dawn-polar               stacked polar ribbon SVG
//!   GET /pages/dawn-list                right-rail per-species strips
//!
//! Sunrise/sunset are taken from the configured station lat/lon if available;
//! without coordinates the sun markers and night wedge are omitted rather than
//! guessed (see `solar_times_local` below).
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
use super::viz::kit::slot_class;
use super::viz::svg_a11y;

static PAGE_HTML: crate::seal::Sealed =
    crate::seal::Sealed::new(include_str!("../../../templates/dawn_chorus.html"));

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
    let pills = format!(
        r#"<div class="pill-row dc-pills"><span class="bnb-pill">Last 60 days</span><span class="bnb-pill">Per-species normalized</span>{moon_badge}<a class="bnb-pill" href="/patterns?tab=migration">Phenology →</a></div>"#
    );
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::DawnChorus),
        title: "The dawn chorus, <em>by species</em>.",
        lead: "Who sings, and when.",
        lede: r#"Each ring is one species; its ribbon swells outward at the hours it's most active, normalized to its own peak so a quiet wren reads as clearly as a loud cardinal. Midnight is at the top; the line is now. <span id="sun-line" hidden>Sunrise <span id="sun-rise" class="mono"></span>, sunset <span id="sun-set" class="mono"></span>.</span>"#,
        aside: &pills,
    });
    PAGE_HTML
        .replace("{{page_head}}", &head)
        .replace("{{skel_polar}}", super::skeletons::polar_plot())
        .replace("{{skel_ribbons}}", &super::skeletons::species_ribbons(6))
}

// ---------------------------------------------------------------------------
// Data shape
// ---------------------------------------------------------------------------

struct ChorusRibbon {
    name: String,
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
                name,
                hours,
                peak_hour,
                total,
            }
        })
        .collect();

    ribbons.sort_by_key(|r| std::cmp::Reverse(r.total));
    ribbons.truncate(top_n);
    // Colour is the ring's rank in this chart (`viz::kit::slot_class`), not a hash of
    // the name: several species are drawn as adjacent rings, and the hash palette put pairs
    // of them 2-3 degrees apart in hue at constant lightness.
    Ok(ribbons)
}

// ---------------------------------------------------------------------------
// Polar SVG
// ---------------------------------------------------------------------------

/// Days of detections the chorus is drawn from.
const WINDOW_DAYS: i64 = 60;

/// Rings drawn: the busiest species of the window.
const RINGS: usize = 8;

/// What the sun does today at the station, in **local** fractional hours: the same clock the
/// rings are bucketed in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sky {
    rise: f64,
    set: f64,
    /// Civil dawn: first light, the sun 6° below the horizon. `None` where it never gets that far down.
    dawn: Option<f64>,
    /// Civil dusk.
    dusk: Option<f64>,
}

/// The sky over the station today, or `None` without a configured location.
fn sky_over(conn: &rusqlite::Connection) -> Option<Sky> {
    let (rise, set) = super::solar_times_local(conn)?;
    let (dawn, dusk) = super::civil_twilight_local(conn).unwrap_or((None, None));
    Some(Sky {
        rise,
        set,
        dawn,
        dusk,
    })
}

async fn polar_partial(State(state): State<AppState>) -> impl IntoResponse {
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| (collect_chorus(conn, WINDOW_DAYS, RINGS), sky_over(conn)))
    })
    .await;

    let (ribbons, sky) = match result {
        Ok((Ok(r), sky)) if !r.is_empty() => (r, sky),
        _ => return ok_html(super::empty_states::no_chorus()),
    };

    // `sky_over` reads the station's configured coordinates and returns **local** hours, the
    // clock the ribbons are bucketed in. Without a configured location there is no honest sun
    // position to draw, so the sky is omitted rather than invented: the previous fallback drew
    // a (40.0 N, -74.0 W) sun over every station on Earth.
    ok_html(polar_figure(&ribbons, sky, &names))
}

/// The chart and the key that says which ring is which bird.
///
/// The key is buttons in the kit's legend idiom: point at a name (or tab to it) and that ring
/// comes forward and the others fall back; a click holds it. It is beside the chart on a wide
/// screen and under it on a phone, outermost ring first as drawn, with the hour each peaks.
fn polar_figure(
    ribbons: &[ChorusRibbon],
    sky: Option<Sky>,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let mut out = String::from(r#"<div class="dc-polar-col ck">"#);
    out.push_str(r#"<div class="dc-polar-plot">"#);
    out.push_str(&render_polar_svg(ribbons, sky, names));
    out.push_str("</div>");
    out.push_str(r#"<div class="dc-key-col"><p class="bnb-meta dc-key-note">Rings, outermost first. Each ring swells at the hours that bird sings most, scaled to its own busiest hour.</p><div class="viz-legend ck-legend dc-key">"#);
    for (i, ribbon) in ribbons.iter().enumerate() {
        let _ = write!(
            out,
            r#"<button type="button" class="ck-leg dc-leg" data-ck-s="{i}" aria-pressed="false"><span class="viz-swatch ck-sw {slot}"></span><span class="dc-leg-name">{n}</span><span class="bnb-meta mono dc-leg-peak">peaks {p}</span></button>"#,
            slot = slot_class(i),
            n = escape_html(names.name_of_stored(&ribbon.name)),
            p = crate::display::clock(&fmt_hour(f64::from(ribbon.peak_hour))),
        );
    }
    out.push_str("</div>");
    if let Some(s) = sky {
        out.push_str(&sun_key(s));
    }
    out.push_str("</div></div>");
    out
}

/// The sun's day in words: first light, sunrise, sunset, last light, each in the reader's clock.
fn sun_key(s: Sky) -> String {
    let mut out = String::from(r#"<ul class="dc-suntimes bnb-meta">"#);
    let mut row = |class: &str, label: &str, h: f64| {
        let _ = write!(
            out,
            r#"<li><i class="dc-sw {class}"></i>{label} <span class="mono">{t}</span></li>"#,
            t = crate::display::clock(&fmt_hour(h)),
        );
    };
    if let Some(h) = s.dawn {
        row("twilight", "First light", h);
    }
    row("day", "Sunrise", s.rise);
    row("day", "Sunset", s.set);
    if let Some(h) = s.dusk {
        row("twilight", "Last light", h);
    }
    out.push_str("</ul>");
    out
}

/// Radii of the picture, in SVG units. From the middle out: the disc, the rings, the sky strip
/// (night, twilight, day), the hour ticks and the hour names.
const SIZE: f64 = 600.0;
const CX: f64 = SIZE / 2.0;
const CY: f64 = SIZE / 2.0;
const RING_MIN: f64 = 78.0;
const RING_MAX: f64 = 222.0;
const SKY_IN: f64 = RING_MAX + 14.0;
const SKY_OUT: f64 = SKY_IN + 7.0;
const LABEL_R: f64 = SKY_OUT + 30.0;

/// Steps each hour's curve is cut into when it is drawn.
const STEPS: usize = 6;

/// A ring's outline: `(hour, value)` samples all the way round, `value` in `0..=1` of the
/// ring's own busiest hour.
///
/// The count of an hour slot is where the bird was heard through that hour, so the slot's
/// **middle** (`h:30`) is where it is drawn. The ring used to be drawn with the slot's count
/// at the start of the hour, which put every swell half an hour early: a bird heard through
/// 06:00-06:59 had its peak on the 06:00 spoke. Between the middles the curve is a monotone
/// cubic (`kit::monotone_segments`, the one the other charts use), so a quiet hour between two
/// loud ones is not filled in and a peak is never overshot, and the ends meet round the clock
/// because the series is wrapped.
fn ring_samples(hours: &[f32; 24]) -> Vec<(f64, f64)> {
    let max = f64::from(hours.iter().copied().fold(0.0_f32, f32::max)).max(f64::MIN_POSITIVE);
    // Three days end to end, so the curve through the middle one has the right slope at both ends.
    let pts: Vec<(f64, f64)> = (0..72_usize)
        .map(|j| (j as f64 - 24.0 + 0.5, f64::from(hours[j % 24]) / max))
        .collect();
    let segs = super::viz::kit::monotone_segments(&pts);
    let bez = |a: f64, b: f64, c: f64, d: f64, t: f64| {
        let u = 1.0 - t;
        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
    };
    let mut out = Vec::with_capacity(24 * STEPS);
    for j in 24..48 {
        let (p0, p1) = (pts[j], pts[j + 1]);
        let [c0, c1] = segs[j];
        for s in 0..STEPS {
            let t = s as f64 / STEPS as f64;
            out.push((
                bez(p0.0, c0.0, c1.0, p1.0, t),
                bez(p0.1, c0.1, c1.1, p1.1, t).clamp(0.0, 1.0),
            ));
        }
    }
    out
}

/// The most a ring reaches in the clock hour `h` (its samples inside `[h, h + 1]`).
fn slot_peak(samples: &[(f64, f64)], h: usize) -> f64 {
    let (lo, hi) = (h as f64, h as f64 + 1.0);
    samples
        .iter()
        .filter(|(t, _)| *t >= lo - 0.5 / STEPS as f64 && *t <= hi + 0.5 / STEPS as f64)
        .map(|s| s.1)
        .fold(0.0, f64::max)
}

/// Outer edge of a ring's band at value `v`, and its inner edge: the swell is mostly outward.
fn ring_edges(base_r: f64, amp: f64, v: f64) -> (f64, f64) {
    (base_r + v * amp * 0.95, base_r - v * amp * 0.15)
}

/// The closed outline of one ring.
fn ring_outline(base_r: f64, amp: f64, samples: &[(f64, f64)]) -> String {
    let mut d = String::with_capacity(samples.len() * 24);
    for (j, &(t, v)) in samples.iter().enumerate() {
        let (x, y) = polar(CX, CY, hour_to_angle(t), ring_edges(base_r, amp, v).0);
        let _ = write!(d, "{}{x:.1},{y:.1}", if j == 0 { "M" } else { " L" });
    }
    for &(t, v) in samples.iter().rev() {
        let (x, y) = polar(CX, CY, hour_to_angle(t), ring_edges(base_r, amp, v).1);
        let _ = write!(d, " L{x:.1},{y:.1}");
    }
    d.push_str(" Z");
    d
}

/// The angles of an arc that runs clockwise from clock hour `from` to clock hour `to`.
fn clock_arc(from: f64, to: f64) -> (f64, f64) {
    let a0 = hour_to_angle(from);
    (a0, a0 + (to - from).rem_euclid(24.0) / 24.0 * 2.0 * PI)
}

/// Render the polar chorus clock.
///
/// `sky` is the sun's day in **local** fractional hours, the same clock the ribbons are
/// bucketed in, or `None` when the station has no configured location. `None` omits the night
/// shading and the sky strip rather than drawing a default position: this chart is read to
/// decide when a species sings relative to sunrise, and a sun drawn at a coordinate the
/// station is not at answers that question wrongly while looking authoritative.
///
/// What is on it, from the middle out: a disc with the time now and the moon's phase; one ring
/// per species, each swelling outward at the hours it sings, scaled to its own peak; a strip
/// of the sky (night, first light, day) and the hour ticks. Every ring-hour is a mark that
/// says what it holds (`static/chart-kit.js`: point, touch or arrow keys; Left and Right move
/// by the hour, Up and Down from ring to ring), and a ring that is read comes forward.
fn render_polar_svg(
    ribbons: &[ChorusRibbon],
    sky: Option<Sky>,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let n = ribbons.len();
    let ring_step = (RING_MAX - RING_MIN) / (n as f64 + 1.0);
    let amp = ring_step * 1.1;
    let base_of = |i: usize| RING_MAX - (i as f64 + 1.0) * ring_step;
    let samples: Vec<Vec<(f64, f64)>> = ribbons.iter().map(|r| ring_samples(&r.hours)).collect();

    let mut s = String::with_capacity(48 * 1024);
    let start = ribbons.first().map_or(0, |r| usize::from(r.peak_hour));
    let sun_attrs = sky.map_or_else(String::new, |k| {
        format!(
            r#" data-sunrise="{}" data-sunset="{}""#,
            fmt_hour(k.rise),
            fmt_hour(k.set)
        )
    });
    let _ = write!(
        s,
        r#"<svg viewBox="0 0 {SIZE} {SIZE}" width="100%" height="100%" role="img" class="dc-polar-svg ck-in" data-ck="marks" data-ck-dim="series" data-ck-grid="24" data-ck-start="{start}" data-ck-id="dawn-rings" data-ck-label="Left and right arrows move by the hour, up and down from ring to ring; the ring you are on comes forward"{sun_attrs}>"#,
    );
    s.push_str(&svg_a11y(
        "Dawn chorus by species",
        &format!(
            "{n} rings, one per species, round a 24-hour clock with midnight at the top. A ring swells outward at the hours that species is heard most, scaled to its own busiest hour. The outermost ring is the species heard most over the last {WINDOW_DAYS} days."
        ),
    ));

    // The night, as a faint wash behind everything, and the sky strip: night, first light, day.
    if let Some(k) = sky {
        let (a1, a2) = clock_arc(k.set, k.rise);
        let r_outer = SKY_OUT;
        let (x1, y1) = polar(CX, CY, a1, r_outer);
        let (x2, y2) = polar(CX, CY, a2, r_outer);
        let large = i32::from((a2 - a1) > PI);
        let _ = write!(
            s,
            r#"<path d="M{CX},{CY} L{x1:.2},{y1:.2} A{r_outer:.2},{r_outer:.2} 0 {large} 1 {x2:.2},{y2:.2} Z" fill="var(--night)" fill-opacity="0.06"/>"#,
        );
        let band = |from: f64, to: f64, class: &str, label: &str| {
            let (b0, b1) = clock_arc(from, to);
            format!(
                r#"<path class="{class}" d="{d}"><title>{label}</title></path>"#,
                d = super::viz::radial::arc_band_path(CX, CY, SKY_IN, SKY_OUT, b0, b1),
            )
        };
        let dawn = k.dawn.unwrap_or(k.rise);
        let dusk = k.dusk.unwrap_or(k.set);
        s.push_str(&band(
            dusk,
            dawn,
            "dc-sky dc-sky-night",
            "Night: the sun is more than 6 degrees below the horizon",
        ));
        if k.dawn.is_some() {
            s.push_str(&band(
                dawn,
                k.rise,
                "dc-sky dc-sky-twilight",
                "First light, before sunrise",
            ));
        }
        s.push_str(&band(k.rise, k.set, "dc-sky dc-sky-day", "Daylight"));
        if k.dusk.is_some() {
            s.push_str(&band(
                k.set,
                dusk,
                "dc-sky dc-sky-twilight",
                "Last light, after sunset",
            ));
        }
    }

    // Spokes and grooves: the hours every three, and the baseline of each ring, so a quiet hour
    // still shows where its ring is.
    for h in (0..24).step_by(3) {
        let a = hour_to_angle(f64::from(h));
        let (x1, y1) = polar(CX, CY, a, RING_MIN - 8.0);
        let (x2, y2) = polar(CX, CY, a, RING_MAX + 2.0);
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}"/>"#
        );
    }
    for i in 0..n {
        let _ = write!(
            s,
            r#"<circle class="ck-grid" cx="{CX}" cy="{CY}" r="{r:.1}"/>"#,
            r = base_of(i),
        );
    }

    // Hour ticks and names.
    for h in 0..24 {
        let a = hour_to_angle(f64::from(h));
        let big = h % 6 == 0;
        let (x1, y1) = polar(CX, CY, a, SKY_OUT + 3.0);
        let (x2, y2) = polar(CX, CY, a, SKY_OUT + if big { 11.0 } else { 7.0 });
        let _ = write!(
            s,
            r#"<line class="{c}" x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}"/>"#,
            c = if big { "ck-axis" } else { "ck-grid-strong" },
        );
    }
    for h in [0, 3, 6, 9, 12, 15, 18, 21] {
        let (lx, ly) = polar(CX, CY, hour_to_angle(f64::from(h)), LABEL_R);
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

    // The sun, on the strip where the colour changes.
    if let Some(k) = sky {
        for (h, class, label) in [
            (k.rise, "dc-sun dc-sun-rise", "Sunrise"),
            (k.set, "dc-sun dc-sun-set", "Sunset"),
        ] {
            let (x, y) = polar(CX, CY, hour_to_angle(h), (SKY_IN + SKY_OUT) / 2.0);
            let _ = write!(
                s,
                r#"<circle class="{class}" cx="{x:.2}" cy="{y:.2}" r="5.5"><title>{label} {t}</title></circle>"#,
                t = fmt_hour(h),
            );
        }
    }

    // Rings, outermost first, each with the name on it for a pointer and in the key for everyone.
    for (i, ribbon) in ribbons.iter().enumerate() {
        let name = escape_html(names.name_of_stored(&ribbon.name));
        let _ = write!(
            s,
            r#"<path class="dc-ring {slot}" d="{d}" data-ck-s="{i}" data-style="--i:{i}"><title>{name} — peaks {p}</title></path>"#,
            slot = slot_class(i),
            d = ring_outline(base_of(i), amp, &samples[i]),
            p = fmt_hour(f64::from(ribbon.peak_hour)),
        );
    }

    // What each ring-hour holds: a wedge as thick as the ring is at that hour (and no thinner
    // than a finger can find), in the order the rings are painted, so the one that shows is the
    // one that is read.
    for (i, ribbon) in ribbons.iter().enumerate() {
        let name = names.name_of_stored(&ribbon.name);
        let total = ribbon.hours.iter().sum::<f32>().max(1.0);
        let peak = ribbon.hours[usize::from(ribbon.peak_hour)];
        for h in 0..24 {
            let count = ribbon.hours[h];
            let (r_out, _) = ring_edges(base_of(i), amp, slot_peak(&samples[i], h).max(0.12));
            let r_in = base_of(i) - 0.17 * amp - 2.0;
            let (a0, a1) = (hour_to_angle(h as f64), hour_to_angle(h as f64 + 1.0));
            let share = f64::from(count) / f64::from(total);
            let when = format!(
                "{} to {}",
                fmt_hour(h as f64),
                fmt_hour(h as f64 + 1.0 - 1.0 / 60.0)
            );
            let data_tip = super::viz::kit::tip(
                name,
                &[
                    (
                        &when,
                        &super::viz::kit::thousands(count.round() as i64),
                        Some(&ribbon_key(i)),
                    ),
                    (
                        "Its share of the day",
                        &format!("{:.0}%", share * 100.0),
                        None,
                    ),
                    (
                        "Its busiest hour",
                        &format!(
                            "{} ({})",
                            fmt_hour(f64::from(ribbon.peak_hour)),
                            super::viz::kit::thousands(peak.round() as i64)
                        ),
                        None,
                    ),
                ],
                Some(&format!("Detections over the last {WINDOW_DAYS} days.")),
            );
            let _ = write!(
                s,
                r#"<path class="ck-hit" d="{d}" data-tip="{data_tip}" data-ck-i="{k}" data-ck-f="{i}"/>"#,
                d = super::viz::radial::arc_band_path(CX, CY, r_in, r_out + 3.0, a0, a1),
                k = i * 24 + h,
            );
        }
    }

    // The middle: the time now, and the moon.
    let _ = write!(
        s,
        r#"<circle cx="{CX}" cy="{CY}" r="{r:.2}" fill="var(--surface)" stroke="var(--hairline)"/>"#,
        r = RING_MIN - 8.0,
    );
    // The moon's phase: four quarter arcs at the cardinal points, the current one drawn heavier.
    // Where it is not new moon the arcs read as a dial; the badge above the chart says which.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let phase = super::overlays::moon_phase_at(now_secs);
    let active = super::overlays::MoonCardinal::from_phase(phase);
    let r_moon = RING_MIN - 16.0;
    // Each segment spans 90° minus a small gap so adjacent arcs read as distinct.
    let gap = 0.16_f64;
    let half_span = PI / 4.0 - gap / 2.0;
    let centers = [
        (
            super::overlays::MoonCardinal::New,
            -PI / 2.0,
            "new",
            "New moon",
        ),
        (
            super::overlays::MoonCardinal::WaxingHalf,
            0.0,
            "waxing-half",
            "Waxing half moon",
        ),
        (
            super::overlays::MoonCardinal::Full,
            PI / 2.0,
            "full",
            "Full moon",
        ),
        (
            super::overlays::MoonCardinal::WaningHalf,
            PI,
            "waning-half",
            "Waning half moon",
        ),
    ];
    for (card, center, seg, label) in centers {
        let (sx, sy) = polar(CX, CY, center - half_span, r_moon);
        let (ex, ey) = polar(CX, CY, center + half_span, r_moon);
        let is_active = card == active;
        // The current phase carries a width the others do not (3 against 1.6) and the one
        // title; the dial is a decoration for everyone else.
        let (class, sw, title) = if is_active {
            (
                "dc-moon is-now",
                "3",
                format!("<title>Moon now: {label}</title>"),
            )
        } else {
            ("dc-moon", "1.6", String::new())
        };
        let _ = write!(
            s,
            r#"<path class="{class}" d="M{sx:.2},{sy:.2} A{r_moon:.2},{r_moon:.2} 0 0 1 {ex:.2},{ey:.2}" stroke-width="{sw}" fill="none" stroke-linecap="round" data-moon-segment="{seg}">{title}</path>"#,
        );
    }
    let now_h = super::now_hour_local();
    let now_clock = fmt_hour(now_h);
    let _ = write!(
        s,
        r#"<text x="{CX}" y="{y1:.1}" text-anchor="middle" class="dc-ctr-k">NOW</text><text x="{CX}" y="{y2:.1}" text-anchor="middle" class="mono dc-ctr-t" {clk}>{now_clock}</text><text x="{CX}" y="{y3:.1}" text-anchor="middle" class="dc-ctr-s">{WINDOW_DAYS} days</text>"#,
        y1 = CY - 14.0,
        y2 = CY + 6.0,
        y3 = CY + 24.0,
        clk = crate::display::clock_attr(&now_clock),
    );

    // The hand: where the day is now, on the same local clock as the rings and the sun. It was
    // `unix_secs % 86400`, UTC, so at 07:00 on a CEST station it pointed at 05:00. A solid
    // hairline in the annotation ink over a halo, so it reads across every ring colour; it was
    // a dotted black line that vanished into the dark rings.
    let a = hour_to_angle(now_h);
    let (hx1, hy1) = polar(CX, CY, a, RING_MIN - 8.0);
    let (hx2, hy2) = polar(CX, CY, a, SKY_OUT + 1.0);
    let _ = write!(
        s,
        r#"<line class="dc-now-halo" x1="{hx1:.2}" y1="{hy1:.2}" x2="{hx2:.2}" y2="{hy2:.2}"/><line class="dc-now" x1="{hx1:.2}" y1="{hy1:.2}" x2="{hx2:.2}" y2="{hy2:.2}"><title>Now, {now_clock}</title></line><circle class="dc-now-dot" cx="{hx2:.2}" cy="{hy2:.2}" r="4"/>"#,
    );

    s.push_str("</svg>");
    s
}

/// The key colour of ring `i` for a tooltip row: the colour itself, so it follows the theme.
fn ribbon_key(i: usize) -> String {
    series_color(i, RINGS)
}

// ---------------------------------------------------------------------------
// Each bird's day
// ---------------------------------------------------------------------------

async fn list_partial(State(state): State<AppState>) -> impl IntoResponse {
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| (collect_chorus(conn, WINDOW_DAYS, RINGS), sky_over(conn)))
    })
    .await;
    let (ribbons, sky) = match result {
        Ok((Ok(r), sky)) if !r.is_empty() => (r, sky),
        _ => return ok_html(super::empty_states::no_chorus()),
    };
    ok_html(day_strips(&ribbons, sky, &names))
}

/// Every ring's day, one row each, as small columns: the same numbers as the rings, laid
/// flat where an hour is a place on a line and a count is a height, which is the better way
/// to read one bird's day and the worse way to compare a dozen. The dawn window (05:00 to
/// 08:59) is shaded behind them, and the sun's day is a line.
///
/// One picture with one tab stop: Left and Right move by the hour, Up and Down from bird to
/// bird. The numbers are in the table under it.
fn day_strips(
    ribbons: &[ChorusRibbon],
    sky: Option<Sky>,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    // Drawn 560 wide with each bird's name above its row, not beside it: a name column would
    // be most of a phone's width, and the picture has to read there without being enlarged.
    const W: f64 = 560.0;
    const ROW_H: f64 = 58.0;
    const BAR_H: f64 = 26.0;
    const TOP: f64 = 22.0;
    let x0 = 8.0;
    let cw = (W - x0 - 8.0) / 24.0;
    let h = TOP + ROW_H * ribbons.len() as f64;
    let x_of = |hour: f64| x0 + hour * cw;
    let start = ribbons.first().map_or(0, |r| usize::from(r.peak_hour));

    let mut s = format!(
        r#"<div class="ck dc-strips"><svg viewBox="0 0 {W} {h}" width="100%" role="img" class="dc-strips-svg ck-in" data-ck="marks" data-ck-grid="24" data-ck-start="{start}" data-ck-id="dawn-strips" data-ck-label="Left and right arrows move by the hour, up and down from bird to bird">"#,
    );
    s.push_str(&svg_a11y(
        "Each bird's day, hour by hour",
        "One row per species, the 24 hours of the day left to right; a taller column is an hour that species is heard more, scaled to its own busiest hour. The dawn window, 05:00 to 08:59, is shaded.",
    ));
    // The dawn window and the sun are drawn in each row's bar band only: a line through the
    // name above the bars would strike the words out.
    let bands: Vec<(f64, f64)> = (0..ribbons.len())
        .map(|i| {
            let top = TOP + ROW_H * i as f64;
            (top + 19.0, top + ROW_H - 6.0)
        })
        .collect();
    for &(y0, y1) in &bands {
        let _ = write!(
            s,
            r#"<rect class="dc-dawn-band" x="{x:.1}" y="{y0:.1}" width="{w:.1}" height="{hh:.1}"/>"#,
            x = x_of(5.0),
            w = 4.0 * cw,
            hh = y1 - y0,
        );
    }
    for hr in [0, 6, 12, 18] {
        let label = match hr {
            0 => "12a".to_string(),
            12 => "12p".to_string(),
            h if h < 12 => format!("{h}a"),
            h => format!("{}p", h - 12),
        };
        let _ = write!(
            s,
            r#"<text class="ck-lbl dcv-hr mono" x="{x:.1}" y="12" text-anchor="start">{label}</text>"#,
            x = x_of(f64::from(hr)),
        );
        for &(y0, y1) in &bands {
            let _ = write!(
                s,
                r#"<line class="ck-grid" x1="{x:.1}" y1="{y0:.1}" x2="{x:.1}" y2="{y1:.1}"/>"#,
                x = x_of(f64::from(hr)),
            );
        }
    }
    if let Some(k) = sky {
        for (hr, label) in [(k.rise, "Sunrise"), (k.set, "Sunset")] {
            for &(y0, y1) in &bands {
                let _ = write!(
                    s,
                    r#"<line class="dc-sun-line" x1="{x:.1}" y1="{y0:.1}" x2="{x:.1}" y2="{y1:.1}"><title>{label} {t}</title></line>"#,
                    x = x_of(hr),
                    t = fmt_hour(hr),
                );
            }
        }
    }

    for (i, ribbon) in ribbons.iter().enumerate() {
        let top = TOP + ROW_H * i as f64;
        let base = top + ROW_H - 6.0;
        let name = names.name_of_stored(&ribbon.name);
        let shown = if name.chars().count() > 26 {
            format!("{}…", name.chars().take(25).collect::<String>().trim_end())
        } else {
            name.to_owned()
        };
        let off_chorus = !(5..=8).contains(&ribbon.peak_hour);
        let _ = write!(
            s,
            r#"<rect class="ck-fill {slot}" x="{x0}" y="{ky:.1}" width="8" height="8" rx="2"/><text class="dc-strip-name" x="{nx:.1}" y="{ny:.1}">{shown}</text><text class="mono dc-strip-sub" x="{rx:.1}" y="{ny:.1}" text-anchor="end">peaks {p}{off}</text>"#,
            slot = slot_class(i),
            ky = top + 4.0,
            nx = x0 + 14.0,
            ny = top + 13.0,
            rx = W - 8.0,
            shown = escape_html(&shown),
            p = fmt_hour(f64::from(ribbon.peak_hour)),
            off = if off_chorus { " · off-chorus" } else { "" },
        );
        let max = ribbon
            .hours
            .iter()
            .copied()
            .fold(0.0_f32, f32::max)
            .max(f32::MIN_POSITIVE);
        let total = ribbon.hours.iter().sum::<f32>().max(1.0);
        let _ = write!(
            s,
            r#"<line class="ck-axis" x1="{x0:.1}" y1="{base:.1}" x2="{x1:.1}" y2="{base:.1}"/>"#,
            x1 = x_of(24.0),
        );
        for hr in 0..24 {
            let count = ribbon.hours[hr];
            let bar_h = (f64::from(count) / f64::from(max)) * BAR_H;
            let data_tip = super::viz::kit::tip(
                name,
                &[
                    (
                        &format!(
                            "{} to {}",
                            fmt_hour(hr as f64),
                            fmt_hour(hr as f64 + 1.0 - 1.0 / 60.0)
                        ),
                        &super::viz::kit::thousands(count.round() as i64),
                        Some(&ribbon_key(i)),
                    ),
                    (
                        "Its share of the day",
                        &format!("{:.0}%", f64::from(count) / f64::from(total) * 100.0),
                        None,
                    ),
                ],
                Some(&format!("Detections over the last {WINDOW_DAYS} days.")),
            );
            let x = x_of(hr as f64);
            let k = i * 24 + hr;
            if count > 0.0 {
                let bh = bar_h.max(1.5);
                let _ = write!(
                    s,
                    r#"<path class="dc-bar {slot}{peak}" d="{d}" data-ck-i="{k}"/>"#,
                    slot = slot_class(i),
                    peak = if hr == usize::from(ribbon.peak_hour) {
                        " is-peak"
                    } else {
                        ""
                    },
                    d = super::viz::kit::bar_path(x + 1.0, base - bh, cw - 2.0, bh, 2.0),
                );
            }
            let _ = write!(
                s,
                r#"<rect class="ck-hit" x="{x:.1}" y="{top:.1}" width="{cw:.1}" height="{ROW_H}" data-tip="{data_tip}" data-ck-i="{k}"/>"#,
            );
        }
    }
    s.push_str("</svg>");
    s.push_str(&day_table(ribbons, names));
    s.push_str("</div>");
    s
}

/// The numbers behind the rings and the day strips: for each species its busiest hour, its
/// share of the dawn window, its total, and the count in every hour.
fn day_table(ribbons: &[ChorusRibbon], names: &birdnet_core::i18n::NameLocalizer) -> String {
    let mut t = String::from(
        r#"<details class="pt-disc"><summary>See the numbers — detections by hour</summary><div class="viz-table-scroll"><table class="viz-table dc-table"><thead><tr><th scope="col">Species</th><th scope="col" class="num">Busiest hour</th><th scope="col" class="num">At dawn</th><th scope="col" class="num">Total</th>"#,
    );
    for h in 0..24 {
        let _ = write!(t, r#"<th scope="col" class="num">{h:02}</th>"#);
    }
    t.push_str("</tr></thead><tbody>");
    for ribbon in ribbons {
        let total: f32 = ribbon.hours.iter().sum();
        let dawn: f32 = ribbon.hours[5..9].iter().sum();
        let _ = write!(
            t,
            r#"<tr><th scope="row">{name}</th><td class="num">{peak}</td><td class="num">{dawn_pct:.0}%</td><td class="num">{total}</td>"#,
            name = escape_html(names.name_of_stored(&ribbon.name)),
            peak = crate::display::clock(&fmt_hour(f64::from(ribbon.peak_hour))),
            dawn_pct = f64::from(dawn) / f64::from(total.max(1.0)) * 100.0,
            total = ribbon.total,
        );
        for c in &ribbon.hours {
            let _ = write!(t, r#"<td class="num">{}</td>"#, c.round() as i64);
        }
        t.push_str("</tr>");
    }
    t.push_str("</tbody></table></div><p class=\"bnb-meta dc-foot\">The dawn window is 05:00 to 08:59; a species whose busiest hour falls outside it is marked <em>off-chorus</em>. Hours are the station's local clock.</p></details>");
    t
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

fn ok_html(body: String) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> birdnet_core::i18n::NameLocalizer {
        birdnet_core::i18n::NameLocalizer::english()
    }

    fn sky(rise: f64, set: f64) -> Sky {
        Sky {
            rise,
            set,
            dawn: None,
            dusk: None,
        }
    }

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
        let svg = render_polar_svg(&[], None, &english());
        assert!(!svg.contains("data-sunrise"), "no sunrise attribute");
        assert!(!svg.contains(r#"data-sun="rise""#), "no rise marker");
        assert!(!svg.contains("var(--night)"), "no night wedge");
        // The rest of the clock still renders.
        assert!(svg.contains(r#"data-moon-segment="new""#));
    }

    /// The "now" hand is on the same local clock as the ribbons and the sun.
    ///
    /// It was `unix_secs % 86400`, UTC, so at 07:00 on a CEST station it
    /// pointed at 05:00. Re-runs itself under `TZ=Asia/Kolkata` (UTC+5:30, no
    /// daylight saving) — `set_var` is unsafe in this edition — and reads
    /// the hand's angle back out of the drawn SVG.
    #[test]
    fn the_now_hand_is_on_the_local_clock() {
        const MARK: &str = "BNB_NOW_HAND_CHILD";
        if std::env::var_os(MARK).is_none() {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "routes::pages::dawn_chorus::tests::the_now_hand_is_on_the_local_clock",
                    "--nocapture",
                ])
                .env(MARK, "1")
                .env("TZ", "Asia/Kolkata")
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr);
            assert!(
                text.contains("1 passed"),
                "child under TZ=Asia/Kolkata:\n{text}"
            );
            return;
        }
        let local = super::super::now_hour_local();
        let utc = super::super::unix_secs().rem_euclid(86_400) as f64 / 3600.0;
        // Precondition: the zone took effect, or this proves nothing.
        assert!(
            ((local - utc).rem_euclid(24.0) - 5.5).abs() < 0.01,
            "local {local} utc {utc}"
        );

        let svg = render_polar_svg(&[], None, &english());
        let hand = svg
            .split("<line ")
            .find(|l| l.starts_with(r#"class="dc-now" "#))
            .expect("the now hand");
        let attr = |name: &str| -> f64 {
            let at = hand.find(&format!(r#"{name}=""#)).unwrap() + name.len() + 2;
            hand[at..].split('"').next().unwrap().parse().unwrap()
        };
        let angle = (attr("y2") - attr("y1")).atan2(attr("x2") - attr("x1"));
        let drawn = ((angle + PI / 2.0) / (2.0 * PI) * 24.0).rem_euclid(24.0);
        let off = (drawn - local)
            .rem_euclid(24.0)
            .min((local - drawn).rem_euclid(24.0));
        assert!(
            off < 0.05,
            "hand at {drawn:.2} h, local {local:.2} h, UTC {utc:.2} h"
        );
    }

    #[test]
    fn a_known_location_draws_both_markers() {
        let svg = render_polar_svg(&[], Some(sky(6.0, 18.0)), &english());
        assert!(svg.contains(r#"data-sunrise="06:00""#), "{svg:.400}");
        assert!(svg.contains(r#"data-sunset="18:00""#));
        assert!(svg.contains("var(--night)"), "night wedge is drawn");
    }

    #[test]
    fn render_polar_svg_includes_four_moon_segments() {
        // Empty ribbons + arbitrary sunrise/sunset are enough — the
        // moon ring is independent of the chorus data.
        let svg = render_polar_svg(&[], Some(sky(6.0, 18.0)), &english());
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

    fn ribbon(name: &str, hours: [f32; 24]) -> ChorusRibbon {
        let peak_hour = hours
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i as u8);
        ChorusRibbon {
            name: name.to_owned(),
            hours,
            peak_hour,
            total: hours.iter().sum::<f32>() as i64,
        }
    }

    fn morning_singer(name: &str) -> ChorusRibbon {
        let mut h = [0.0_f32; 24];
        for (i, v) in [(5, 8.0), (6, 30.0), (7, 12.0), (8, 3.0)] {
            h[i] = v;
        }
        ribbon(name, h)
    }

    /// A ring swells where the bird was heard, not half an hour before it.
    ///
    /// The count of an hour slot is a count of 06:00-06:59, so its swell belongs to the middle of
    /// the slot (06:30); the ring was drawn with it at the start (06:00), every swell half an hour
    /// early, which on a chart read to decide who sings before sunrise is the margin that matters.
    ///
    /// Observed failing against the old placement (`hours[h]` at the angle of `h:00`): the single
    /// busy hour came out at 6.0, this asserts 6.5.
    #[test]
    fn a_ring_swells_at_the_middle_of_its_hour() {
        let mut hours = [0.0_f32; 24];
        hours[6] = 10.0;
        let samples = ring_samples(&hours);
        let (t, v) = samples
            .iter()
            .copied()
            .fold((0.0, -1.0), |best, s| if s.1 > best.1 { s } else { best });
        assert!((t - 6.5).abs() < 1e-9, "the peak is at {t} h, not 6.5");
        assert!((v - 1.0).abs() < 1e-9, "and it is the whole height");
        // Counterpart: the hour on either side of a busy one is not busier than it.
        let at = |h: f64| {
            samples
                .iter()
                .min_by(|a, b| (a.0 - h).abs().total_cmp(&(b.0 - h).abs()))
                .map_or(0.0, |s| s.1)
        };
        assert!(
            at(5.5) < 1e-9 && at(7.5) < 1e-9,
            "the neighbouring hours are quiet"
        );
    }

    /// Smoothing cannot invent a peak: no sample is above the busiest hour, below nothing, and a
    /// quiet hour between two loud ones is not filled in beyond what its neighbours allow.
    #[test]
    fn a_ring_never_rises_above_its_busiest_hour_or_below_nothing() {
        let r = morning_singer("American Robin");
        for (t, v) in ring_samples(&r.hours) {
            assert!((-1e-12..=1.0 + 1e-12).contains(&v), "{t}: {v}");
        }
        // Midnight wraps: a night singer's swell across 23:00-00:59 is one swell, not two.
        let mut hours = [0.0_f32; 24];
        hours[23] = 8.0;
        hours[0] = 8.0;
        let s = ring_samples(&hours);
        let near_midnight_gap = s
            .iter()
            .filter(|(t, _)| (23.5..24.5).contains(t))
            .map(|x| x.1)
            .fold(1.0, f64::min);
        assert!(
            near_midnight_gap > 0.99,
            "{near_midnight_gap}: the ring is broken at midnight"
        );
    }

    /// Every ring-hour is a mark that says what it holds, and the marks are dense and unique, so
    /// the keyboard can visit each once and Up and Down move from ring to ring.
    #[test]
    fn every_ring_hour_is_a_mark_that_reads() {
        let rings = [
            morning_singer("American Robin"),
            morning_singer("Song Sparrow"),
        ];
        let svg = render_polar_svg(&rings, None, &english());
        assert_eq!(svg.matches(r#"class="ck-hit""#).count(), 2 * 24);
        for k in 0..48 {
            assert!(svg.contains(&format!(r#"data-ck-i="{k}""#)), "mark {k}");
        }
        assert!(svg.contains(r#"data-ck-grid="24""#));
        // The busiest hour of the first ring: its name, the span, the count and the share.
        assert!(svg.contains("American Robin"));
        assert!(svg.contains("06:00 to 06:59"));
        assert!(svg.contains("Its share of the day"));
        assert!(svg.contains("Its busiest hour"));
        // The keyboard starts on the outermost ring's busiest hour.
        assert!(svg.contains(r#"data-ck-start="6""#));
        // A ring comes forward through the kit's focus tokens, one per ring.
        assert_eq!(svg.matches(r#"data-ck-f="0""#).count(), 24);
        assert_eq!(svg.matches(r#"data-ck-f="1""#).count(), 24);
        assert!(svg.contains(r#"data-ck-dim="series""#));
    }

    /// The middle of the chart says the time and the window, and is not the orphan "chorus 24 h".
    #[test]
    fn the_centre_says_something_true() {
        let svg = render_polar_svg(&[morning_singer("A")], None, &english());
        assert!(svg.contains(">NOW<") && svg.contains("60 days"));
        assert!(
            !svg.contains(">24 h<") && !svg.contains(">chorus<"),
            "the orphan centre is back"
        );
        // The time is marked for the reader's clock choice.
        assert!(svg.contains("data-clk="), "{svg:.0}");
    }

    /// The sky strip: night, first light, day, last light, in that order round the clock, each
    /// as long as it lasts, and only when the station has a location.
    #[test]
    fn the_sky_strip_is_night_twilight_and_day() {
        let sky = Sky {
            rise: 7.0,
            set: 17.5,
            dawn: Some(6.2),
            dusk: Some(18.3),
        };
        let svg = render_polar_svg(&[morning_singer("A")], Some(sky), &english());
        assert_eq!(svg.matches("dc-sky-night").count(), 1);
        assert_eq!(svg.matches("dc-sky-twilight").count(), 2);
        assert_eq!(svg.matches("dc-sky-day").count(), 1);
        assert!(
            svg.contains("First light, before sunrise") && svg.contains("Last light, after sunset")
        );
        // Where there is no civil twilight (a midsummer night in the north) there is no strip for it.
        let bright = Sky {
            dawn: None,
            dusk: None,
            ..sky
        };
        let svg = render_polar_svg(&[morning_singer("A")], Some(bright), &english());
        assert_eq!(svg.matches("dc-sky-twilight").count(), 0);
        assert!(svg.contains("dc-sky-night"));
        // Without a location, nothing.
        let svg = render_polar_svg(&[morning_singer("A")], None, &english());
        assert!(!svg.contains("dc-sky"));
        // Arcs run clockwise from the first hour to the second, across midnight too.
        let (a0, a1) = clock_arc(6.0, 18.0);
        assert!((a1 - a0 - PI).abs() < 1e-9, "twelve hours is half a turn");
        let (a0, a1) = clock_arc(22.0, 6.0);
        assert!(
            (a1 - a0 - 2.0 * PI / 3.0).abs() < 1e-9,
            "22:00 to 06:00 is eight hours"
        );
    }

    /// "Each bird's day": a bar per hour with something in it, none taller than the strip, the
    /// busiest of each row full height, and a hit box for every hour of every row.
    #[test]
    fn each_birds_day_draws_the_same_numbers_as_the_rings() {
        let rings = [
            morning_singer("American Robin"),
            morning_singer("Song Sparrow"),
        ];
        let html = day_strips(&rings, None, &english());
        assert_eq!(html.matches(r#"class="ck-hit""#).count(), 48);
        assert_eq!(
            html.matches("dc-bar ").count(),
            2 * 4,
            "four hours each had a detection"
        );
        assert_eq!(html.matches("is-peak").count(), 2);
        // The table has all 24 hours and the totals.
        assert_eq!(html.matches("<tr><th scope=\"row\">").count(), 2);
        assert!(html.contains(">53<"), "8 + 30 + 12 + 3 = 53: {html:.0}");
        assert!(
            !html.contains(" · off-chorus"),
            "peaking at 06:00 is on-chorus"
        );
        let night = ribbon("Barred Owl", {
            let mut h = [0.0_f32; 24];
            h[1] = 9.0;
            h
        });
        assert!(day_strips(&[night], None, &english()).contains(" · off-chorus"));
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
