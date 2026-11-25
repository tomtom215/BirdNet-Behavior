//! What every chart renderer shares: scales, round numbers, curves, dates, and the
//! `data-*` attributes `static/chart-kit.js` reads.
//!
//! Nothing here knows about a particular chart. The rules a chart is held to are in
//! `docs/design/charts.md`; the ones that live in code are:
//!
//! * a bar grows from a single baseline and is rounded only at its data end
//!   ([`bar_path`]);
//! * a curve through points never leaves the range of the two neighbours it joins
//!   ([`monotone_path`]), so smoothing cannot invent a peak the data did not have;
//! * an axis tops out on a round number at or above the data ([`count_ceiling`],
//!   [`count_ticks`]), and a count too large to print is compacted ([`compact`]);
//! * a reading the tooltip shows is written once, here, in the form the script parses
//!   ([`tip`], [`cols_attrs`]).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use crate::routes::pages::escape_html;

/// Month names, for axis labels.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// Weekday names, Sunday first (the order the heat map's rows use).
pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// ── dates ───────────────────────────────────────────────────────────────────────

/// The calendar day `days` after the ISO date `end`, as `("14 Oct", "Tue 14 Oct")`.
///
/// Uses the workspace's own civil-date arithmetic rather than a date crate; the
/// workspace carries none. `None` if `end` does not parse, so a malformed date
/// degrades to a chart without dates rather than a panic in a render path
/// (`panic = "abort"`, so a handler panic is a station outage).
pub fn civil_label(end: &str, days: i64) -> Option<(String, String)> {
    let t = birdnet_core::civil::parse_civil(end, "00:00:00")?;
    let secs = birdnet_core::civil::unix_secs_from_civil(&t) + days * 86_400;
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    let month = MONTHS.get((c.month as usize).wrapping_sub(1))?;
    let weekday = WEEKDAYS[weekday_of_secs(secs)];
    let short = format!("{} {month}", c.day);
    let long = format!("{weekday} {short}");
    Some((short, long))
}

/// Sunday = 0 … Saturday = 6, `back` days before the day numbered `epoch_day` (days since 1970-01-01).
pub const fn weekday_back(epoch_day: i64, back: usize) -> usize {
    (epoch_day - back as i64 + 4).rem_euclid(7) as usize
}

/// The hour (0..=23) a fractional hour of the day falls in.
pub const fn hour_floor(hours: f64) -> usize {
    hours.floor().clamp(0.0, 23.0) as usize
}

/// Sunday = 0 … Saturday = 6 for a Unix timestamp (1970-01-01 was a Thursday).
pub const fn weekday_of_secs(secs: i64) -> usize {
    (secs.div_euclid(86_400) + 4).rem_euclid(7) as usize
}

/// `(year, month 1-12, day)` of the ISO date `date` shifted by `days`, or `None`.
pub fn shift_date(date: &str, days: i64) -> Option<(i32, u32, u32)> {
    let t = birdnet_core::civil::parse_civil(date, "00:00:00")?;
    let secs = birdnet_core::civil::unix_secs_from_civil(&t) + days * 86_400;
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    Some((c.year as i32, c.month, c.day))
}

/// Days since the epoch of an ISO date, or `None`.
pub fn epoch_day(date: &str) -> Option<i64> {
    let t = birdnet_core::civil::parse_civil(date, "00:00:00")?;
    Some(birdnet_core::civil::unix_secs_from_civil(&t).div_euclid(86_400))
}

// ── numbers ─────────────────────────────────────────────────────────────────────

/// Gridline values for an axis of whole counts from zero to `top` (itself a
/// [`count_ceiling`]): zero and equal steps of 1, 2 or 5 times a power of ten, never a
/// fraction, so the labels read 0, 10, 20, 30, 40 and never 0, 13, 27, 40 (a rule at 2.5
/// detections is not a thing that was counted). Up to `want` steps, or one more where
/// that is the only way to be round.
pub fn count_ticks(top: f64, want: usize) -> Vec<f64> {
    let top = top.max(1.0);
    if !top.is_finite() {
        return vec![0.0];
    }
    let round = |step: f64| {
        let mag = 10_f64.powf(step.log10().floor());
        let f = (step / mag * 1000.0).round() / 1000.0;
        step >= 1.0 && [1.0, 2.0, 5.0, 10.0].contains(&f)
    };
    let div = (1..=want.max(1) + 1)
        .rev()
        .find(|&d| round(top / d as f64))
        .unwrap_or(1);
    let step = top / div as f64;
    (0..=div)
        .map(|i| (step * i as f64 * 1e9).round() / 1e9)
        .collect()
}

/// The top of an axis of whole counts: `x` rounded up to 1, 2, 3, 4, 5, 6 or 8 times
/// a power of ten (an integer, always): a ceiling that allowed 1.5 or 2.5 would put a
/// fractional rule on a count axis.
pub fn count_ceiling(x: f64) -> f64 {
    if !x.is_finite() || x <= 1.0 {
        return 1.0;
    }
    let mut mag = 10_f64.powf(x.log10().floor());
    if x / mag >= 10.0 {
        mag *= 10.0;
    }
    for m in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0] {
        if m * mag >= x {
            return m * mag;
        }
    }
    10.0 * mag
}

/// A count for an axis or a label: `950`, `1.5K`, `12.9K`, `1.6M`. Whole numbers
/// below a thousand are printed as they are.
pub fn compact(v: f64) -> String {
    let a = v.abs();
    let (scaled, suffix) = if a >= 1e9 {
        (v / 1e9, "B")
    } else if a >= 1e6 {
        (v / 1e6, "M")
    } else if a >= 1e3 {
        (v / 1e3, "K")
    } else {
        return if (v - v.round()).abs() < 0.05 {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.1}")
        };
    };
    // Two significant digits to ten, one decimal to a hundred, whole above.
    let s = if scaled.abs() < 10.0 {
        format!("{scaled:.1}")
    } else {
        format!("{scaled:.0}")
    };
    let s = s.strip_suffix(".0").unwrap_or(&s).to_string();
    format!("{s}{suffix}")
}

/// An integer with thousands separators, for a tooltip or a table.
pub fn thousands(n: i64) -> String {
    crate::routes::pages::group_thousands(n)
}

/// The room an axis's labels need on the left of a plot, given the width of the widest one at
/// its drawn size and the width of the drawing. On a phone the drawing is scaled down to about
/// about 300 px (a card on a 390 px phone) and `chart-text.js` grows any label that would then be
/// under 9.5 px, in proportion to how far it was scaled: a margin cut to the label as drawn makes
/// "1.6M" run off the edge and get cut to "1…". So the margin allows for that growth (at most 3.2 times).
pub fn axis_margin(widest: f64, drawing_width: f64) -> f64 {
    let growth = (0.95 * drawing_width / 300.0).clamp(1.0, 3.2);
    widest * growth + 8.0
}

/// Rough width of `s` set at `font` units: the label planner's measure. Wide on
/// purpose (0.58 em per glyph); an over-estimate drops a label, an under-estimate
/// lets two touch.
pub fn text_width(s: &str, font: f64) -> f64 {
    s.chars().count() as f64 * font * 0.58
}

/// Rough width of a note written in the display face's italic (13 units): that face is
/// narrower than the UI face `text_width` measures, so this is that measure scaled down, and
/// still on the wide side.
pub fn note_width(s: &str) -> f64 {
    text_width(s, 13.0) * 0.78
}

// ── scales ──────────────────────────────────────────────────────────────────────

/// A linear map from a data range to a screen range.
#[derive(Clone, Copy, Debug)]
pub struct Linear {
    d0: f64,
    d1: f64,
    r0: f64,
    r1: f64,
}

impl Linear {
    /// `domain` maps onto `range`; a degenerate domain maps everything to `range.0`.
    pub const fn new(domain: (f64, f64), range: (f64, f64)) -> Self {
        Self {
            d0: domain.0,
            d1: domain.1,
            r0: range.0,
            r1: range.1,
        }
    }

    /// The screen position of `v`.
    pub fn at(self, v: f64) -> f64 {
        let span = self.d1 - self.d0;
        if span.abs() < f64::EPSILON {
            return self.r0;
        }
        self.r0 + (v - self.d0) / span * (self.r1 - self.r0)
    }
}

// ── curves ──────────────────────────────────────────────────────────────────────

/// Tangents for a monotone cubic through `pts` (Fritsch–Carlson, with the harmonic
/// mean of the neighbouring slopes). A flat or reversing neighbourhood gets a zero
/// tangent, so the curve never rises above a peak or dips below a trough that the
/// data does not contain.
fn tangents(pts: &[(f64, f64)]) -> Vec<f64> {
    let n = pts.len();
    let mut m = vec![0.0; n];
    if n < 2 {
        return m;
    }
    let h: Vec<f64> = pts.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let d: Vec<f64> = pts
        .windows(2)
        .zip(&h)
        .map(|(w, h)| {
            if *h == 0.0 {
                0.0
            } else {
                (w[1].1 - w[0].1) / h
            }
        })
        .collect();
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        if d[k - 1] * d[k] <= 0.0 {
            m[k] = 0.0;
        } else {
            let w1 = 2.0 * h[k] + h[k - 1];
            let w2 = h[k] + 2.0 * h[k - 1];
            m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
        }
    }
    m
}

/// The cubic Bézier segments of the monotone curve through `pts`: for each pair of
/// neighbours, the two control points.
pub fn monotone_segments(pts: &[(f64, f64)]) -> Vec<[(f64, f64); 2]> {
    let m = tangents(pts);
    pts.windows(2)
        .enumerate()
        .map(|(k, w)| {
            let h = w[1].0 - w[0].0;
            [
                (w[0].0 + h / 3.0, w[0].1 + m[k] * h / 3.0),
                (w[1].0 - h / 3.0, w[1].1 - m[k + 1] * h / 3.0),
            ]
        })
        .collect()
}

/// An SVG path through `pts` as a monotone cubic, starting with `M` (or `L` when
/// `continue_from_current` is set, for a path that is already under way).
pub fn monotone_path(pts: &[(f64, f64)], continue_from_current: bool) -> String {
    let mut d = String::with_capacity(pts.len() * 28);
    let Some(first) = pts.first() else {
        return d;
    };
    let _ = write!(
        d,
        "{}{:.1},{:.1}",
        if continue_from_current { "L" } else { "M" },
        first.0,
        first.1
    );
    for (w, c) in pts.windows(2).zip(monotone_segments(pts)) {
        let _ = write!(
            d,
            " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            c[0].0, c[0].1, c[1].0, c[1].1, w[1].0, w[1].1
        );
    }
    d
}

/// The same curve walked right to left, continuing the current subpath: for the
/// return leg of a closed band, so the lower edge is the curve the neighbour's
/// upper edge is, and the two never part.
pub fn monotone_path_reversed(pts: &[(f64, f64)]) -> String {
    let mut d = String::with_capacity(pts.len() * 28);
    let Some(last) = pts.last() else {
        return d;
    };
    let _ = write!(d, "L{:.1},{:.1}", last.0, last.1);
    let segs = monotone_segments(pts);
    for (w, c) in pts.windows(2).zip(&segs).rev() {
        let _ = write!(
            d,
            " C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
            c[1].0, c[1].1, c[0].0, c[0].1, w[0].0, w[0].1
        );
    }
    d
}

/// A rendered chart reduced to `(length, FNV-1a 64)`, for the tests that pin a renderer's
/// output byte for byte across a refactor (UIC-18). FNV and not `DefaultHasher`, whose output
/// the standard library does not promise to keep from one release to the next.
/// With `BNB_PIN_DUMP` set to a directory, the drawing is also written there as `{name}.svg`,
/// so the pictures before and after a change can be compared line by line.
#[cfg(test)]
pub fn pin(name: &str, s: &str) -> (usize, u64) {
    if let Some(dir) = std::env::var_os("BNB_PIN_DUMP") {
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.svg")), s);
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (s.len(), h)
}

/// The value of the monotone curve through `pts` at `x` (for tests and for placing
/// a mark on a curve rather than on the polyline). `None` outside the data's span.
#[cfg(test)]
pub fn monotone_at(pts: &[(f64, f64)], x: f64) -> Option<f64> {
    let segs = monotone_segments(pts);
    for (w, c) in pts.windows(2).zip(&segs) {
        if x < w[0].0 || x > w[1].0 {
            continue;
        }
        // Solve x(t) = x by bisection: x(t) is increasing for a monotone-in-x curve.
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        let bez = |a: f64, b: f64, c2: f64, d: f64, t: f64| {
            let u = 1.0 - t;
            u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c2 + t * t * t * d
        };
        for _ in 0..40 {
            let t = lo.midpoint(hi);
            if bez(w[0].0, c[0].0, c[1].0, w[1].0, t) < x {
                lo = t;
            } else {
                hi = t;
            }
        }
        let t = lo.midpoint(hi);
        return Some(bez(w[0].1, c[0].1, c[1].1, w[1].1, t));
    }
    None
}

/// A bar's outline: square at the baseline, `r` units of round on the two corners at
/// its data end. `up` is true for a bar that rises from a baseline below it.
/// `r` shrinks to fit a bar narrower or shorter than twice its radius, so a sliver
/// of one detection is still a sliver and not a blob.
pub fn bar_path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let h = h.max(0.0);
    let r = r.min(w / 2.0).min(h).max(0.0);
    if h <= 0.0 || w <= 0.0 {
        return String::new();
    }
    format!(
        "M{x:.2},{b:.2} L{x:.2},{t:.2} Q{x:.2},{y:.2} {x1:.2},{y:.2} L{x2:.2},{y:.2} Q{xr:.2},{y:.2} {xr:.2},{t:.2} L{xr:.2},{b:.2} Z",
        b = y + h,
        t = y + r,
        x1 = x + r,
        x2 = x + w - r,
        xr = x + w,
    )
}

/// A horizontal bar's outline: square at the left baseline, round at the right end.
pub fn hbar_path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let w = w.max(0.0);
    let r = r.min(h / 2.0).min(w).max(0.0);
    if h <= 0.0 || w <= 0.0 {
        return String::new();
    }
    format!(
        "M{x:.2},{y:.2} L{xa:.2},{y:.2} Q{xr:.2},{y:.2} {xr:.2},{ya:.2} L{xr:.2},{yb:.2} Q{xr:.2},{y2:.2} {xa:.2},{y2:.2} L{x:.2},{y2:.2} Z",
        xa = x + w - r,
        xr = x + w,
        ya = y + r,
        yb = y + h - r,
        y2 = y + h,
    )
}

// ── what the kit script reads ───────────────────────────────────────────────────

/// An attribute value from text: HTML-escaped, with the two control characters the
/// tooltip format uses written as character references so no parser can fold them.
fn attr(s: &str) -> String {
    escape_html(s).replace('\n', "&#10;").replace('\t', "&#9;")
}

/// The value of a `data-tip`: a heading, then one line per row (`label`, `value` and
/// an optional key colour, tab-separated), then optional notes (lines with no tab).
///
/// Every part is untrusted text (species names); the script sets each with
/// `textContent`, and this escapes it for the attribute.
pub fn tip(heading: &str, rows: &[(&str, &str, Option<&str>)], note: Option<&str>) -> String {
    let clean = |s: &str| s.replace(['\n', '\t'], " ");
    let mut v = clean(heading);
    for (l, val, key) in rows {
        let _ = write!(v, "\n{}\t{}", clean(l), clean(val));
        if let Some(k) = key {
            let _ = write!(v, "\t{k}");
        }
    }
    if let Some(n) = note {
        let _ = write!(v, "\n{}", clean(n));
    }
    attr(&v)
}

/// One column of a "cols" chart: its heading, one value per series (`None` where
/// there is no data, which is not the same as zero) and an optional note.
pub struct Col {
    /// Heading, e.g. `Tue 14 Oct`.
    pub label: String,
    /// One value per series.
    pub values: Vec<Option<f64>>,
    /// A line under the values (for instance "Station was off").
    pub note: Option<String>,
}

/// The attributes of an `<svg data-ck="cols">`: columns, series, plot edges, unit.
///
/// `series` is `(name, key colour)`; the colour is a `var(--…)` reference, never a
/// literal. Returns the attribute string, starting with a space.
pub fn cols_attrs(
    cols: &[Col],
    series: &[(String, String)],
    x0: f64,
    x1: f64,
    band: bool,
    unit: &str,
    total_label: Option<&str>,
) -> String {
    let cols_json: Vec<serde_json::Value> = cols
        .iter()
        .map(|c| {
            let vals: Vec<serde_json::Value> = c
                .values
                .iter()
                .map(|v| v.map_or(serde_json::Value::Null, |x| serde_json::json!(x)))
                .collect();
            c.note.as_ref().map_or_else(
                || serde_json::json!([c.label, vals]),
                |n| serde_json::json!([c.label, vals, n]),
            )
        })
        .collect();
    let series_json: Vec<serde_json::Value> = series
        .iter()
        .map(|(n, c)| serde_json::json!([n, c]))
        .collect();
    let mut s = format!(
        r#" data-ck="cols" data-ck-cols="{}" data-ck-series="{}" data-ck-x0="{x0:.1}" data-ck-x1="{x1:.1}" data-ck-pos="{}""#,
        escape_html(&serde_json::Value::Array(cols_json).to_string()),
        escape_html(&serde_json::Value::Array(series_json).to_string()),
        if band { "band" } else { "point" },
    );
    if !unit.is_empty() {
        let _ = write!(s, r#" data-ck-unit="{}""#, escape_html(unit));
    }
    if let Some(t) = total_label {
        let _ = write!(s, r#" data-ck-total="{}""#, escape_html(t));
    }
    s
}

/// `<linearGradient>` defs for the chart's marks: one per colour class, vertical,
/// full colour at the top fading a little toward the card at the base. The stops are
/// painted by `app.css` (`.ck-gr`), so the theme and the colour tokens stay in CSS.
///
/// `classes` are CSS classes that set `--c` (`ck-c1`..`ck-c8`, `ck-cother`,
/// `ck-q1`..`ck-q7`); the gradient for class `ck-c3` has the id `{prefix}-ck-c3`.
pub fn gradient_defs(prefix: &str, classes: &[String]) -> String {
    let mut s = String::from("<defs>");
    // Once per class: several species can share the neutral, and an id is written once.
    for (i, c) in classes.iter().enumerate() {
        if classes[..i].contains(c) {
            continue;
        }
        let _ = write!(
            s,
            r#"<linearGradient id="{prefix}-{c}" class="ck-gr {c}" x1="0" y1="0" x2="0" y2="1"><stop class="a" offset="0"/><stop class="b" offset="1"/></linearGradient>"#
        );
    }
    s.push_str("</defs>");
    s
}

/// A hatch for "no data": diagonal hairlines in grid ink, as a `<pattern>` with the id
/// `{prefix}-nd`. Used where a stretch of the chart is *not known* (the station was off,
/// the sun is unknown), so that it cannot be read as *zero*. The lines are 45 degrees and
/// recessive: this is a statement about the data, not decoration.
pub fn nodata_pattern(prefix: &str) -> String {
    format!(
        r#"<pattern id="{prefix}-nd" patternUnits="userSpaceOnUse" width="6" height="6" patternTransform="rotate(45)"><line class="ck-grid-strong" x1="0" y1="0" x2="0" y2="6" stroke-width="2"/></pattern>"#
    )
}

/// The sky behind a 24-hour axis as a left-to-right gradient `id`, from the station's own
/// sunrise and sunset (fractional hours): night until first light, a dawn wash at sunrise,
/// day, a dusk wash at sunset, night again. The stops are named for the four washes and the
/// stylesheet gives them their colours (`.ck-sky`), so the theme chooses every value.
///
/// Fill it into a rectangle exactly as wide as the plot: the gradient is measured on that
/// rectangle (0 is midnight, 1 the next midnight). Stop offsets never go backwards, so a
/// short winter day at a high latitude squeezes the day wash out instead of inverting it.
pub fn sky_gradient(id: &str, rise: f64, set: f64) -> String {
    let at = |hours: f64| (hours / 24.0).clamp(0.0, 1.0);
    let stops = [
        ("n", 0.0),
        ("n", at(rise - 1.7)),
        ("dw", at(rise - 0.1)),
        ("dy", at(rise + 2.0)),
        ("dy", at(set - 2.0)),
        ("dk", at(set + 0.1)),
        ("n", at(set + 1.7)),
        ("n", 1.0),
    ];
    let mut s = format!(r#"<linearGradient id="{id}" class="ck-sky" x1="0" y1="0" x2="1" y2="0">"#);
    let mut floor = 0.0_f64;
    for (class, offset) in stops {
        floor = floor.max(offset);
        let _ = write!(s, r#"<stop class="{class}" offset="{floor:.4}"/>"#);
    }
    s.push_str("</linearGradient>");
    s
}

/// A vertical gradient `id` that clears the foot of a panel toward the card, so a sky meets
/// the ground at a horizon instead of ending in a hard edge.
pub fn haze_gradient(id: &str) -> String {
    format!(
        r#"<linearGradient id="{id}" class="ck-haze" x1="0" y1="0" x2="0" y2="1"><stop class="a" offset="0"/><stop class="b" offset="1"/></linearGradient>"#
    )
}

/// The step, in counts, between two seams across a column: 1, 2 or 5 times a power of ten,
/// the smallest that leaves no more than twelve of them from zero to `top`. A column of eight
/// detections is eight notches; one of eight hundred is eight, each a hundred.
pub fn notch_unit(top: f64) -> f64 {
    let mut magnitude = 1.0_f64;
    loop {
        for m in [1.0, 2.0, 5.0] {
            let unit = m * magnitude;
            if top / unit <= 12.0 {
                return unit;
            }
        }
        magnitude *= 10.0;
    }
}

/// A pattern `id` of horizontal seams, one at every whole `unit_px` up from the baseline at
/// `plot_h`, centred on the value boundary it marks. Fill a column's own shape with
/// `url(#id)` over its colour: the column keeps its true height and gains its steps.
pub fn notch_pattern(id: &str, plot_h: f64, unit_px: f64) -> String {
    let seam = 1.5_f64;
    let y0 = (plot_h - seam / 2.0).rem_euclid(unit_px);
    format!(
        r#"<pattern id="{id}" patternUnits="userSpaceOnUse" x="0" y="{y0:.2}" width="40" height="{unit_px:.3}"><rect class="ck-notch" x="0" y="0" width="40" height="{seam}"/></pattern>"#
    )
}

/// Radial gradients that put a soft halo behind a disc of each colour class in `classes`
/// (`ck-q3`, `ck-c2`, …), named `{prefix}-{class}`: the halo is the disc's own colour
/// fading to nothing, so a heat grid glows where it is busy.
pub fn halo_gradients(prefix: &str, classes: &[String]) -> String {
    let mut s = String::new();
    // Once per class, as in `gradient_defs`.
    for (i, c) in classes.iter().enumerate() {
        if classes[..i].contains(c) {
            continue;
        }
        let _ = write!(
            s,
            r#"<radialGradient id="{prefix}-{c}" class="ck-hg {c}"><stop class="a" offset=".5"/><stop class="b" offset="1"/></radialGradient>"#
        );
    }
    s
}

/// `s` reduced to what is safe in an element id and in a `url(#…)` reference: ASCII
/// letters, digits and hyphens.
pub fn id_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// A time of day given as hours since midnight (`6.35`) as `HH:MM`, to the nearest minute, wrapped
/// onto one day (`24.0` is `00:00`). Plain text, for a chart's `<title>` and `data-tip`; a time
/// printed where the reader sees it goes through `crate::display`, which the reader's 12- or
/// 24-hour choice re-prints. The one clock the charts share (UIC-18): `bars`, `bursts`, `funnel`
/// and the dawn chorus each had their own.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn clock_of_hours(h: f64) -> String {
    let total = (h * 60.0).round() as i64;
    format!(
        "{:02}:{:02}",
        (total / 60).rem_euclid(24),
        total.rem_euclid(60)
    )
}

/// Seconds after midnight as `HH:MM`, the minute it falls in (never rounded up), as
/// [`clock_of_hours`].
#[cfg(feature = "analytics")]
#[must_use]
pub fn clock_of_secs(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 3600 % 24, secs % 3600 / 60)
}

/// `v` in `0..=1` as a percentage for a reading: whole numbers, and "<1%" for what rounds to
/// nothing but is not nothing.
#[must_use]
pub fn pct(v: f64) -> String {
    let p = v * 100.0;
    if p > 0.0 && p < 1.0 {
        "<1%".to_owned()
    } else {
        format!("{:.0}%", p.round())
    }
}

/// The class for categorical slot `i` (0-based): `ck-c1` … `ck-c8`, then `ck-cother`.
///
/// For a category that is **not a species**: a single series (`Fill::Slot(0)`, a day chart's
/// one colour). A species takes its colour from the station's [`SpeciesSlots`], never from its
/// position in a chart; `tests/a_species_keeps_its_colour_on_every_chart.rs` lists the callers
/// this is allowed in.
pub fn slot_class(i: usize) -> String {
    if i < 8 {
        format!("ck-c{}", i + 1)
    } else {
        "ck-cother".to_string()
    }
}

// ── one colour per species, station-wide ────────────────────────────────────────

/// How many species own a colour: one each for `--chart-1` … `--chart-8`.
pub const SPECIES_SLOTS: usize = 8;

/// The days of history the station's colours are ranked over, ending yesterday.
pub const SLOT_WINDOW_DAYS: u32 = 90;

/// Which species owns which chart colour, for the whole station (UIR-01).
///
/// A species' colour used to be its rank within each chart, so a bird was green on one chart
/// and orange on the next. Now the eight species heard most over the [`SLOT_WINDOW_DAYS`] days
/// that end yesterday own `--chart-1` … `--chart-8` in that order (most heard first, ties by
/// name), on every chart. Ending the window yesterday keeps the assignment the same all day,
/// so two figures rendered minutes apart agree: it changes at midnight, or when the history
/// itself changes (an import, a deletion, a relabel). A station with
/// fewer than eight such species fills its free colours from today, most heard first, after
/// the ones it already has, so a new station is not grey on its first day and no species gives
/// up a colour during the day.
///
/// Every other species is `--chart-other`, the neutral that already meant "everything else". A
/// colour is never lent to a species that does not own it: that is what makes "the green one is
/// the Robin" true on every page. The legend, the tooltip and the table name every species, so
/// the neutral is never the only thing that tells two of them apart.
///
/// A chart may hold any of a species' names: the one stored with the detection, the scientific
/// name, or the name the station shows. All three find it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeciesSlots {
    /// The owners' stored names, in slot order.
    owners: Vec<String>,
    /// Every name an owner goes by, to its slot (0-based).
    by_name: std::collections::HashMap<String, usize>,
}

impl SpeciesSlots {
    /// The assignment for owners given as `(stored name, scientific name, shown name)`, most
    /// heard first. Past the eighth they are ignored.
    #[must_use]
    pub fn new<I, S>(owners: I) -> Self
    where
        I: IntoIterator<Item = (S, S, S)>,
        S: Into<String>,
    {
        let mut slots = Self::default();
        for (stored, sci, shown) in owners {
            let k = slots.owners.len();
            if k >= SPECIES_SLOTS {
                break;
            }
            let stored: String = stored.into();
            if slots.by_name.contains_key(&stored) {
                continue;
            }
            for name in [sci.into(), shown.into()] {
                if !name.is_empty() {
                    slots.by_name.entry(name).or_insert(k);
                }
            }
            slots.by_name.insert(stored.clone(), k);
            slots.owners.push(stored);
        }
        slots
    }

    /// The owners' stored names, in slot order.
    #[cfg(test)]
    #[must_use]
    pub fn owners(&self) -> &[String] {
        &self.owners
    }

    /// The slot `name` owns (0-based), if it owns one.
    #[must_use]
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// The colour class for `name`: `ck-c1` … `ck-c8`, or `ck-cother`.
    #[must_use]
    pub fn class(&self, name: &str) -> String {
        self.slot(name)
            .map_or_else(|| "ck-cother".to_owned(), |k| format!("ck-c{}", k + 1))
    }

    /// The colour itself, for a tooltip's key: `var(--chart-1)` … or `var(--chart-other)`.
    #[must_use]
    pub fn colour(&self, name: &str) -> String {
        self.slot(name).map_or_else(
            || "var(--chart-other)".to_owned(),
            |k| format!("var(--chart-{})", k + 1),
        )
    }

    /// The slot as the scripts number it: 1 … 8, or 0 for the neutral.
    #[must_use]
    pub fn number(&self, name: &str) -> usize {
        self.slot(name).map_or(0, |k| k + 1)
    }
}

/// The station's [`SpeciesSlots`], read from its history and kept in the analytics cache.
///
/// Computed in one place for every chart; the cache keeps it for the cache's time to live
/// (`analytics_cache::DEFAULT_TTL`) under a key that names the day, and since the ranking ends
/// yesterday every recomputation during a day gives the same answer. A failed read is the
/// caller's to report, as it reports its own: it is not defaulted here, and a colour is never
/// guessed.
///
/// # Errors
///
/// The database error, when the history cannot be read.
pub fn station_slots(
    state: &crate::state::AppState,
) -> Result<SpeciesSlots, birdnet_db::sqlite::DbError> {
    let names = state.names();
    let decode = |stored: &str| -> SpeciesSlots {
        SpeciesSlots::new(stored.lines().filter_map(|l| {
            let (com, sci) = l.split_once('\t')?;
            Some((com, sci, names.name(sci, com)))
        }))
    };
    let today = crate::routes::pages::today_date_string();
    let key = format!("species-slots:{today}");
    if let Some(hit) = state.analytics_cache().get(&key) {
        return Ok(decode(&hit));
    }
    let owners = state.with_read_db(|conn| read_owners(conn, SLOT_WINDOW_DAYS))?;
    let stored = owners
        .iter()
        .map(|(com, sci)| format!("{com}\t{sci}"))
        .collect::<Vec<_>>()
        .join("\n");
    let slots = decode(&stored);
    state.analytics_cache().put(key, stored);
    Ok(slots)
}

/// [`station_slots`] for a figure that is not drawn without them: the error is logged, naming
/// `figure`, and the caller gets `None` to report as it reports its own failed read.
pub fn station_slots_for(state: &crate::state::AppState, figure: &str) -> Option<SpeciesSlots> {
    station_slots(state)
        .map_err(|e| tracing::warn!(error = %e, figure, "the station's species colours could not be read"))
        .ok()
}

/// [`station_slots`] for decoration, where a missing colour costs a picture and not a fact (a
/// list's sparklines): unread, every species is the documented neutral, `--chart-other`, and
/// the error is logged naming `what`. Never a guessed colour presented as a species'.
pub fn station_slots_or_neutral(state: &crate::state::AppState, what: &str) -> SpeciesSlots {
    station_slots(state).unwrap_or_else(|e| {
        tracing::warn!(error = %e, what, "the station's species colours could not be read; drawn neutral");
        SpeciesSlots::default()
    })
}

/// The owners as `(stored name, scientific name)`, slot order: the eight most heard over the
/// `window` days that end yesterday, then, while a colour is free, today's most heard.
///
/// # Errors
///
/// The database error.
pub fn read_owners(
    conn: &rusqlite::Connection,
    window: u32,
) -> Result<Vec<(String, String)>, birdnet_db::sqlite::DbError> {
    let mut owners: Vec<(String, String)> = Vec::with_capacity(SPECIES_SLOTS);
    let mut history = conn.prepare(
        "SELECT Com_Name, MIN(Sci_Name), COUNT(*) AS n FROM detections_analytic
         WHERE Date >= DATE('now', 'localtime', '-' || ?1 || ' days')
           AND Date < DATE('now', 'localtime')
         GROUP BY Com_Name
         ORDER BY n DESC, Com_Name
         LIMIT ?2",
    )?;
    let rows = history.query_map(rusqlite::params![window, SPECIES_SLOTS as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        owners.push(row?);
    }
    if owners.len() < SPECIES_SLOTS {
        let mut today = conn.prepare(
            "SELECT Com_Name, MIN(Sci_Name), COUNT(*) AS n FROM detections_analytic
             WHERE Date = DATE('now', 'localtime')
             GROUP BY Com_Name
             ORDER BY n DESC, Com_Name",
        )?;
        let rows = today.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (com, sci) = row?;
            if owners.len() >= SPECIES_SLOTS {
                break;
            }
            if !owners.iter().any(|(c, _)| *c == com) {
                owners.push((com, sci));
            }
        }
    }
    Ok(owners)
}

/// The sequential step, 1 to 7, for `t` in `0..=1` (0 is the lowest drawn step; a
/// caller handles "empty" itself). Square-root spaced, so a day with one detection
/// beside a day with two hundred is not the same pale cell as every other small day.
pub fn seq_step(t: f64) -> usize {
    let t = t.clamp(0.0, 1.0).sqrt();
    ((t * 7.0).ceil() as usize).clamp(1, 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_axis_never_has_a_fractional_rule() {
        for x in [
            0.0,
            1.0,
            1.2,
            2.0,
            3.0,
            7.0,
            9.0,
            12.0,
            26.0,
            40.0,
            99.0,
            450.0,
            1_600_000.0,
        ] {
            let top = count_ceiling(x);
            assert!(
                top >= x.max(1.0) && (top - top.round()).abs() < 1e-9,
                "{x} -> {top}"
            );
            let t = count_ticks(top, 4);
            for v in &t {
                assert!((v - v.round()).abs() < 1e-9, "{x}: top {top} rules {t:?}");
            }
            assert!(
                (t[t.len() - 1] - top).abs() < 1e-6 && t.len() >= 2 && t.len() <= 7,
                "{x}: {t:?}"
            );
        }
        assert_eq!(count_ticks(1.0, 4), vec![0.0, 1.0]);
        assert_eq!(count_ticks(10.0, 4), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(count_ticks(8.0, 4), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
        assert_eq!(count_ceiling(7.0), 8.0);
    }

    /// The margin leaves room for a label that the phone grows: wider drawings are scaled down
    /// further, so they need more, up to a ceiling; and it is never less than the label as drawn.
    #[test]
    fn the_axis_margin_allows_for_the_growth_a_phone_gives_a_label() {
        let w = text_width("1.6M", 10.0);
        assert!(
            axis_margin(w, 340.0) >= w + 8.0,
            "never less than the label"
        );
        assert!(
            axis_margin(w, 960.0) > axis_margin(w, 560.0),
            "a wider drawing is shrunk more"
        );
        assert!(
            axis_margin(w, 760.0) >= w * 2.3 + 8.0,
            "a 760 drawing on a 300 px card grows its text 2.4 times"
        );
        assert!(
            (axis_margin(w, 5000.0) - (w * 3.2 + 8.0)).abs() < 1e-9,
            "and it stops at 3.2"
        );
    }

    #[test]
    fn counts_are_compacted_without_losing_the_order_of_magnitude() {
        for (v, want) in [
            (0.0, "0"),
            (7.0, "7"),
            (950.0, "950"),
            (1_000.0, "1K"),
            (1_500.0, "1.5K"),
            (12_900.0, "13K"),
            (999_000.0, "999K"),
            (1_600_000.0, "1.6M"),
            (25_000_000.0, "25M"),
            (2.5, "2.5"),
        ] {
            assert_eq!(compact(v), want, "{v}");
        }
    }

    #[test]
    fn a_linear_scale_maps_the_ends_and_the_middle() {
        let s = Linear::new((0.0, 10.0), (100.0, 0.0));
        assert!((s.at(0.0) - 100.0).abs() < 1e-9);
        assert!((s.at(10.0)).abs() < 1e-9);
        assert!((s.at(5.0) - 50.0).abs() < 1e-9);
        // A degenerate domain must not divide by zero.
        assert!((Linear::new((3.0, 3.0), (7.0, 9.0)).at(3.0) - 7.0).abs() < 1e-9);
    }

    /// The claim the streamgraph and the trend line rest on: between two data
    /// points the curve stays between them. A cubic spline would overshoot a spike
    /// and show a peak that was never measured; this one cannot. Checked on a spike,
    /// a plateau, a step and a random walk, at 50 samples per segment.
    #[test]
    fn the_monotone_curve_never_leaves_the_range_of_its_neighbours() {
        let cases: Vec<Vec<f64>> = vec![
            vec![0.0, 0.0, 30.0, 0.0, 0.0],
            vec![5.0, 5.0, 5.0, 5.0],
            vec![0.0, 0.0, 10.0, 10.0, 0.0],
            vec![1.0, 9.0, 2.0, 8.0, 3.0, 7.0],
            (0..40).map(|i| f64::from((i * 7919) % 31)).collect(),
        ];
        for ys in cases {
            let pts: Vec<(f64, f64)> = ys
                .iter()
                .enumerate()
                .map(|(i, y)| (i as f64 * 10.0, *y))
                .collect();
            for w in pts.windows(2) {
                let (lo, hi) = (w[0].1.min(w[1].1), w[0].1.max(w[1].1));
                for s in 0..=50 {
                    let x = w[0].0 + (w[1].0 - w[0].0) * f64::from(s) / 50.0;
                    let y = monotone_at(&pts, x).expect("inside the span");
                    assert!(
                        y >= lo - 1e-6 && y <= hi + 1e-6,
                        "{ys:?} at {x}: {y} outside [{lo}, {hi}]"
                    );
                }
            }
        }
    }

    /// It is a curve *through* the data: the value at each sample is the sample.
    #[test]
    fn the_monotone_curve_passes_through_every_point() {
        let pts: Vec<(f64, f64)> = [3.0, 9.0, 1.0, 4.0]
            .iter()
            .enumerate()
            .map(|(i, y)| (i as f64 * 20.0, *y))
            .collect();
        for p in &pts {
            assert!((monotone_at(&pts, p.0).unwrap() - p.1).abs() < 1e-6);
        }
        assert!(monotone_at(&pts, -1.0).is_none());
        assert!(monotone_at(&pts, 61.0).is_none());
        assert!(monotone_path(&[], false).is_empty());
        assert!(monotone_path(&[(1.0, 2.0)], false).starts_with("M1.0,2.0"));
    }

    /// Two bands that share an edge share the curve: the lower band's top edge
    /// is the upper band's bottom edge walked the other way, so a gap or an overlap
    /// between them cannot appear. The reversed walk must end where the forward one
    /// began, through the same points.
    #[test]
    fn the_reversed_path_retraces_the_forward_one() {
        let pts = [(0.0, 5.0), (10.0, 9.0), (20.0, 2.0)];
        let back = monotone_path_reversed(&pts);
        assert!(back.starts_with("L20.0,2.0"), "{back}");
        assert!(back.ends_with("0.0,5.0"), "{back}");
        assert_eq!(back.matches('C').count(), 2);
    }

    /// A bar is square where it stands and round where it ends, and never wider than
    /// its own radius allows: a one-pixel sliver stays a sliver.
    #[test]
    fn a_bar_is_square_at_the_baseline_and_round_at_the_tip() {
        let d = bar_path(10.0, 20.0, 12.0, 30.0, 4.0);
        // Baseline y = 50: both bottom corners are plain `L`/`M` at y 50.
        assert!(d.starts_with("M10.00,50.00 L10.00,24.00"), "{d}");
        assert!(d.contains("Q10.00,20.00 14.00,20.00"), "{d}");
        assert!(d.contains("Q22.00,20.00 22.00,24.00"), "{d}");
        assert!(d.ends_with("L22.00,50.00 Z"), "{d}");
        // Too short for the radius: it shrinks to the height.
        let sliver = bar_path(0.0, 98.0, 12.0, 2.0, 4.0);
        assert!(
            sliver.contains("L0.00,100.00") && sliver.contains("Q0.00,98.00 2.00,98.00"),
            "{sliver}"
        );
        // Nothing to draw: nothing drawn.
        assert!(bar_path(0.0, 0.0, 10.0, 0.0, 4.0).is_empty());
        assert!(hbar_path(0.0, 0.0, 0.0, 10.0, 4.0).is_empty());
        assert!(hbar_path(0.0, 0.0, 30.0, 10.0, 4.0).contains("Q30.00,0.00 30.00,4.00"));
    }

    #[test]
    fn a_tip_is_escaped_and_cannot_be_split_by_its_own_text() {
        let t = tip(
            "Tue \"14\" Oct",
            &[
                ("A\tB <i>", "3", Some("var(--chart-1)")),
                ("Robin", "4", None),
            ],
            Some("Note\nwith a break"),
        );
        assert!(
            !t.contains('\n') && !t.contains('\t') && !t.contains('<') && !t.contains('"'),
            "{t}"
        );
        assert!(t.contains("Tue &quot;14&quot; Oct&#10;A B &lt;i&gt;&#9;3&#9;var(--chart-1)&#10;Robin&#9;4&#10;Note with a break"), "{t}");
    }

    #[test]
    fn cols_are_json_in_an_attribute_and_null_is_not_zero() {
        let cols = vec![
            Col {
                label: "Mon 1 Jan".into(),
                values: vec![Some(3.0), None],
                note: None,
            },
            Col {
                label: "Tue 2 Jan".into(),
                values: vec![Some(0.0), Some(2.0)],
                note: Some("Station off".into()),
            },
        ];
        let a = cols_attrs(
            &cols,
            &[
                ("Robin".into(), "var(--chart-1)".into()),
                ("Jay \"Blue\"".into(), "var(--chart-2)".into()),
            ],
            10.0,
            90.0,
            false,
            "",
            Some("Total"),
        );
        assert!(a.contains(r#"data-ck="cols""#));
        assert!(a.contains("[[&quot;Mon 1 Jan&quot;,[3.0,null]],[&quot;Tue 2 Jan&quot;,[0.0,2.0],&quot;Station off&quot;]]"), "{a}");
        assert!(a.contains("Jay \\&quot;Blue\\&quot;"), "{a}");
        assert!(a.contains(r#"data-ck-pos="point""#) && a.contains(r#"data-ck-total="Total""#));
    }

    #[test]
    fn dates_walk_across_a_month_and_name_their_weekday() {
        let (short, long) = civil_label("2026-03-02", -5).expect("parses");
        assert_eq!((short.as_str(), long.as_str()), ("25 Feb", "Wed 25 Feb"));
        assert!(civil_label("not-a-date", 0).is_none());
        assert_eq!(shift_date("2026-03-01", -1), Some((2026, 2, 28)));
        assert_eq!(shift_date("2024-03-01", -1), Some((2024, 2, 29)));
        // 1970-01-01 was a Thursday; 2026-10-21 a Wednesday.
        assert_eq!(WEEKDAYS[weekday_of_secs(0)], "Thu");
        assert_eq!(civil_label("2026-10-21", 0).unwrap().1, "Wed 21 Oct");
    }

    /// The step a count gets: at least 1, at most 7, never falling as the count
    /// rises, and spread by square root so the small counts are told apart.
    #[test]
    fn the_sequential_step_rises_with_the_count() {
        assert_eq!(seq_step(0.0), 1);
        assert_eq!(seq_step(1.0), 7);
        let mut last = 0;
        for i in 0..=100 {
            let s = seq_step(f64::from(i) / 100.0);
            assert!(s >= last && (1..=7).contains(&s));
            last = s;
        }
        assert!(
            seq_step(0.05) < seq_step(0.5),
            "a twentieth is paler than a half"
        );
        assert_ne!(
            seq_step(0.01),
            seq_step(0.1),
            "1 vs 10 detections of 100 must differ"
        );
    }

    /// One colour a species, the eight in order, the ninth and every unknown name the neutral,
    /// and each owner found by every name it goes by.
    #[test]
    fn the_owners_take_the_eight_colours_in_order_and_no_one_else_does() {
        let owners: Vec<(String, String, String)> = (0..10)
            .map(|i| {
                (
                    format!("Bird {i}"),
                    format!("Avis {i}"),
                    format!("Vogel {i}"),
                )
            })
            .collect();
        let slots = SpeciesSlots::new(
            owners
                .iter()
                .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())),
        );
        assert_eq!(slots.owners().len(), SPECIES_SLOTS);
        for i in 0..SPECIES_SLOTS {
            let want = format!("ck-c{}", i + 1);
            for name in [
                format!("Bird {i}"),
                format!("Avis {i}"),
                format!("Vogel {i}"),
            ] {
                assert_eq!(slots.class(&name), want, "{name}");
            }
            assert_eq!(
                slots.colour(&format!("Bird {i}")),
                format!("var(--chart-{})", i + 1)
            );
            assert_eq!(slots.number(&format!("Bird {i}")), i + 1);
        }
        for name in ["Bird 8", "Bird 9", "Nobody", ""] {
            assert_eq!(slots.class(name), "ck-cother", "{name}");
            assert_eq!(slots.colour(name), "var(--chart-other)");
            assert_eq!(slots.number(name), 0);
        }
        // A name listed twice keeps its first colour and does not take a second.
        let twice = SpeciesSlots::new([("A", "", "A"), ("A", "", "A"), ("B", "", "B")]);
        assert_eq!(
            (twice.class("A"), twice.class("B")),
            ("ck-c1".into(), "ck-c2".into())
        );
        assert!(SpeciesSlots::default().owners().is_empty());
    }

    /// The owners are the history's most heard to yesterday, not today's: today cannot move a
    /// colour. While a colour is free, today's birds fill it after the history's.
    #[test]
    fn the_owners_come_from_the_history_and_today_only_fills_free_colours() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let add = |back: i64, com: &str, n: usize| {
            for k in 0..n {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                     VALUES (DATE('now', 'localtime', ?1), ?2, ?3, ?4, 0.9)",
                    rusqlite::params![
                        format!("-{back} days"),
                        format!("06:{k:02}:00"),
                        format!("{com} sci"),
                        com
                    ],
                )
                .unwrap();
            }
        };
        add(3, "Robin", 5);
        add(2, "Wren", 3);
        add(0, "Wren", 40); // today: the busiest by far, and it moves nothing
        add(0, "Newcomer", 20);
        add(0, "Rarity", 1);
        let owners: Vec<String> = read_owners(&conn, SLOT_WINDOW_DAYS)
            .unwrap()
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(owners, ["Robin", "Wren", "Newcomer", "Rarity"]);
        // Outside the window, history does not count.
        add(200, "Ancient", 99);
        assert!(
            !read_owners(&conn, SLOT_WINDOW_DAYS)
                .unwrap()
                .iter()
                .any(|(c, _)| c == "Ancient")
        );
    }
}
